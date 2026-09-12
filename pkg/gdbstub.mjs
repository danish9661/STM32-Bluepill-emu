// pkg/gdbstub.mjs — GDB Remote Serial Protocol stub for the STM32F1 emulator.
//
// Lets a debugger (or a script speaking RSP) inspect and control live
// firmware: registers, memory, breakpoints, step/continue. Node-only
// transport (TCP; GDB connects with `target remote :<port>`).
//
//   import { serveGdb } from 'stm32f1-emu/gdb';  // (see package.json exports)
//   const srv = await serveGdb({ firmware, port: 1234, chip: 'gd32f103c8' });
//
// Breakpoints are 16-bit BKPT patches (restored + single-stepped past on
// hit, re-inserted on resume — the standard dance). The core faults loudly
// on BKPT, which the stub distinguishes from genuine decode gaps by
// address: hits report SIGTRAP, anything else SIGILL and halt.
//
// Data watchpoints (Z2/Z3/Z4) map onto the model's 4 DWT-style comparator
// slots and report T05watch:/rwatch:/awatch: with the tripping address.
// `c` resumes a halted core; `s` single-steps past a halt.
//
// Fidelity notes: single thread; `s` steps one thread instruction plus any
// domestically-pending IRQ service for that batch (an interrupt may be
// entered as part of a step, like silicon); `c` runs until a breakpoint,
// fault, or Ctrl-C. Registers are the 17 ARM core regs (no FPU on M3).
import { createEmulator } from './emulator.js';
import net from 'node:net';

const u32le = (n) => {
    n >>>= 0;
    return [n & 0xFF, (n >> 8) & 0xFF, (n >> 16) & 0xFF, (n >> 24) & 0xFF]
        .map((b) => b.toString(16).padStart(2, '0')).join('');
};
const unhex = (s) => {
    const out = [];
    for (let i = 0; i + 1 < s.length; i += 2) out.push(parseInt(s.slice(i, i + 2), 16));
    return out;
};
const pkt = (data) => {
    let c = 0;
    for (let i = 0; i < data.length; i++) c = (c + data.charCodeAt(i)) & 0xFF;
    return `$${data}#${c.toString(16).padStart(2, '0')}`;
};

const TARGET_XML = `<?xml version="1.0"?>` +
    `<target version="1.0"><architecture>arm</architecture><feature name="org.gnu.gdb.arm.m-profile">` +
    Array.from({ length: 13 }, (_, i) => `<reg name="r${i}" bitsize="32"/>`).join('') +
    `<reg name="sp" bitsize="32"/><reg name="lr" bitsize="32"/>` +
    `<reg name="pc" bitsize="32"/><reg name="xpsr" bitsize="32"/>` +
    `</feature></target>`;

/**
 * Serve one emulator instance over GDB RSP.
 * @param {object} opts  createEmulator opts + { port=1234, chunk=20000 }
 * @returns {Promise<{port:number, close():void, emu:any}>}
 */
export async function serveGdb(opts = {}) {
    const { port = 1234, chunk = 20000, ...emuOpts } = opts;
    const emu = await createEmulator(emuOpts);
    emu.setSymbols([]); // no UNDEFINSTR escalation: faults surface via takeFault
    const bps = new Map(); // addr -> { orig: number[] }
    let reinsert = null;   // addr whose BKPT was lifted for a single-step
    // Data watchpoints (GDB Z2/Z3/Z4 -> model kinds 1/2/3): addr -> { slot, zkind }.
    // Model checks are exact byte ranges on guest data accesses; debugger
    // memory writes use the raw path and never trip.
    const watches = new Map();
    const zkindToModel = { 2: 1, 3: 2, 4: 3 };

    const memReadBytes = (addr, len) => {
        const out = [];
        let a = addr >>> 0, n = len;
        const head = a & 3;
        if (head) {
            const w = emu.memRead32(a - head) >>> 0;
            for (let i = head; i < 4 && n > 0; i++, n++, a++) out.push((w >>> (i * 8)) & 0xFF);
        }
        while (n >= 4) { const w = emu.memRead32(a) >>> 0; out.push(w & 0xFF, (w >> 8) & 0xFF, (w >> 16) & 0xFF, (w >> 24) & 0xFF); a += 4; n -= 4; }
        if (n > 0) { const w = emu.memRead32(a) >>> 0; for (let i = 0; i < n; i++) out.push((w >>> (i * 8)) & 0xFF); }
        return out;
    };
    const memWrite = (addr, bytes) => {
        let a = addr >>> 0, i = 0;
        while (i < bytes.length) {
            const base = a & ~3;
            const w = emu.memRead32(base) >>> 0;
            const arr = [w & 0xFF, (w >> 8) & 0xFF, (w >> 16) & 0xFF, (w >> 24) & 0xFF];
            while (i < bytes.length && (a - base) < 4) { arr[a - base] = bytes[i++]; a++; }
            emu.memWriteBytes(base, arr);
        }
    };

    const server = net.createServer((sock) => {
        let inbuf = '', noAck = false, closed = false;
        const send = (data) => { if (!closed) sock.write(pkt(data)); };
        const sendRaw = (s) => { if (!closed) sock.write(s); };

        const regs = () => {
            const r = emu.getRegisters();
            const order = [...Array(13).keys()].map((i) => r[`R${i}`]);
            return [...order, r.SP, r.LR, r.PC, r.xPSR].map(u32le).join('');
        };
        const stopReply = () => 'S05';

        // GDB stop reason for a watch trip ([addr, dir 1=write/2=read]):
        // the Z-kind comes from the first watch covering the address
        // (trip may land mid-range for len > 1), else the direction.
        const watchStopReply = (trip) => {
            let zkind = trip[1] === 2 ? 3 : 2;
            for (const w of watches.values()) {
                if (trip[0] >= w.addr && trip[0] < w.addr + w.len) { zkind = w.zkind; break; }
            }
            const name = zkind === 3 ? 'rwatch' : zkind === 4 ? 'awatch' : 'watch';
            return `T05${name}:${(trip[0] >>> 0).toString(16)};`;
        };

        const runUntilEvent = async () => {
            for (;;) {
                if (closed) return null;
                if (inbuf.includes('\x03')) { inbuf = inbuf.replace('\x03', ''); return 'S02'; }
                const r = await emu.step(chunk);
                // Watchpoint trip first: the debug stop reason outranks a
                // same-batch fault (which stays live for takeFault).
                const trip = emu.swdTakeTrip();
                if (trip.length >= 2) return watchStopReply(trip);
                // DHCSR halt with no trip (probe halt, VC catch): plain trap.
                if (emu.swdHalted()) return 'S05';
                const f = emu.takeFault();
                if (f) {
                    const fpc = f[0] >>> 0;
                    if (bps.has(fpc)) {
                        // BKPT hit: restore original, resume AT the instruction.
                        const bp = bps.get(fpc);
                        memWrite(fpc, bp.orig);
                        emu.setPc(fpc);
                        reinsert = fpc;
                        return 'S05';
                    }
                    return 'S04'; // genuine decode gap
                }
                if (r.stopped) return 'S05';
            }
        };

        const handle = async (cmd) => {
            const q = cmd.split(',')[0].split(':')[0].split(';')[0];
            if (cmd === '?') return send(stopReply());
            if (cmd.startsWith('qSupported')) {
                // NOTE: no qXfer:features:read advertisement — GDB 15's XML
                // parser rejects our minimal doc, while its default ARM
                // layout matches our 17-register g packet exactly. The
                // endpoint below still serves it to explicit requesters.
                if (!noAck) sendRaw('+');
                return send('PacketSize=3fff;QStartNoAckMode+;vContSupported+');
            }
            if (cmd === 'QStartNoAckMode') { noAck = true; return send('OK'); }
            if (cmd === 'qAttached') return send('1');
            if (cmd === 'qfThreadInfo') return send('m1');
            if (cmd === 'qsThreadInfo') return send('l');
            if (cmd.startsWith('qXfer:features:read:target.xml:')) {
                const [off, len] = cmd.split(':').pop().split(',').map((x) => parseInt(x, 16));
                const hex = Buffer.from(TARGET_XML, 'utf8').toString('hex');
                const start = (off || 0) * 2, piece = hex.slice(start, start + (len || 0xfff) * 2);
                return send((start + piece.length >= hex.length ? 'l' : 'm') + piece);
            }
            if (cmd.startsWith('qXfer:features:read:target-features')) return send('');
            if (cmd === 'qC') return send('QC1');
            if (cmd === 'g') return send(regs());
            // NOTE: RSP register numbers are HEX (`Pf` = PC = 15) — a real
            // GDB client found the old decimal parse dropping `set $pc`.
            if (/^p[0-9a-fA-F]+$/.test(cmd)) {
                const n = parseInt(cmd.slice(1), 16);
                const r = emu.getRegisters();
                const order = [...Array(13).keys()].map((i) => r[`R${i}`]);
                const all = [...order, r.SP, r.LR, r.PC, r.xPSR];
                if (n < 0 || n >= all.length) return send('E01');
                return send(u32le(all[n]));
            }
            if (/^P[0-9a-fA-F]+?=/.test(cmd)) {
                const [rn, val] = cmd.slice(1).split('=');
                const n = parseInt(rn, 16);
                if (n < 0 || n > 15) return send('E01');
                emu.setReg(n, parseInt(val.slice(0, 8).match(/../g).reverse().join(''), 16));
                return send('OK');
            }
            // G = write all 17 registers at once (GDB's fallback when a
            // single P fails; now that P is hex-correct this rarely fires).
            if (cmd.startsWith('G')) {
                const words = cmd.slice(1).match(/.{8}/g) || [];
                if (words.length !== 17) return send('E01');
                words.forEach((w, i) => {
                    if (i > 15) return; // xPSR stays read-only, like P
                    emu.setReg(i, parseInt(w.match(/../g).reverse().join(''), 16));
                });
                return send('OK');
            }
            if (cmd.startsWith('m')) {
                const [a, l] = cmd.slice(1).split(',').map((x) => parseInt(x, 16));
                if (!(a >= 0) || !(l >= 0) || l > 0x1000) return send('E01');
                return send(memReadBytes(a, l).map((b) => b.toString(16).padStart(2, '0')).join(''));
            }
            if (cmd.startsWith('M')) {
                const [head, hexdata] = cmd.slice(1).split(':');
                const [a, l] = head.split(',').map((x) => parseInt(x, 16));
                const bytes = unhex(hexdata).slice(0, l);
                if (!(a >= 0) || bytes.length !== l) return send('E01');
                memWrite(a, bytes);
                return send('OK');
            }
            if (cmd.startsWith('Z0,')) {
                const [a] = cmd.slice(3).split(',').map((x) => parseInt(x, 16));
                if (!(a >= 0)) return send('E01');
                const addr = a & ~1; // Thumb bit: breakpoints bind the halfword
                if (!bps.has(addr)) bps.set(addr, { orig: memReadBytes(addr, 2) });
                memWrite(addr, [0x00, 0xBE]);
                return send('OK');
            }
            if (cmd.startsWith('z0,')) {
                const [a] = cmd.slice(3).split(',').map((x) => parseInt(x, 16));
                const addr = a & ~1;
                const bp = bps.get(addr);
                if (bp) { memWrite(addr, bp.orig); bps.delete(addr); }
                if (reinsert === addr) reinsert = null;
                return send('OK');
            }
            // Data watchpoints: Z2 = write, Z3 = read, Z4 = access.
            // Model slots are a scarce 4 (like DWT comparators); the stub
            // maps one slot per Z-packet and frees it on z*/restart.
            if (/^Z[234],/.test(cmd)) {
                const zkind = parseInt(cmd[1], 10);
                const [a, l] = cmd.slice(3).split(',').map((x) => parseInt(x, 16));
                if (!(a >= 0) || !(l > 0)) return send('E01');
                const slot = emu.swdAddWatch(zkindToModel[zkind], a >>> 0, l >>> 0);
                if (slot < 0) return send('E01');
                watches.set(a >>> 0, { slot, zkind, addr: a >>> 0, len: l >>> 0 });
                return send('OK');
            }
            if (/^z[234],/.test(cmd)) {
                const [a] = cmd.slice(3).split(',').map((x) => parseInt(x, 16));
                const w = watches.get(a >>> 0);
                if (w) { emu.swdRemoveWatch(w.slot); watches.delete(a >>> 0); }
                return send('OK');
            }
            if (cmd === 'c' || cmd.startsWith('c;') || cmd === 'vCont;c') {
                // A stale halt (watch trip, DHCSR C_HALT) must not wedge
                // continue: GDB `c` means run. (BKPT re-insert dance below
                // is unaffected — resume only clears the halt mirror/bits.)
                emu.swdResume();
                if (reinsert !== null) {
                    await emu.step(1); // run the original instruction once
                    const bp = bps.get(reinsert);
                    if (bp) memWrite(reinsert, [0x00, 0xBE]);
                    reinsert = null;
                }
                return send(await runUntilEvent());
            }
            if (cmd === 's' || cmd.startsWith('s;') || cmd === 'vCont;s') {
                // Step past a halt (trip/DHCSR) with the debug single-step;
                // otherwise a normal one-instruction batch.
                if (emu.swdHalted()) emu.swdStep();
                else { await emu.step(1); emu.takeFault(); }
                return send('S05');
            }
            if (cmd === 'vCont?') return send('vCont;c;s');
            if (cmd === 'k' || cmd === 'D') { send('OK'); sock.end(); closed = true; return; }
            // Single thread: any Hc/Hg selection (GDB 15 sends Hc0/Hc1/Hc-1)
            // addresses our one thread.
            if (/^H[gc]-?[0-9]*$/.test(cmd)) return send('OK');
            return send('');
        };

        let busy = Promise.resolve();
        sock.on('data', (buf) => {
            inbuf += buf.toString('binary');
            busy = busy.then(async () => {
                for (;;) {
                    if (closed) return;
                    if (inbuf[0] === '+' || inbuf[0] === '-') { inbuf = inbuf.slice(1); continue; }
                    if (inbuf[0] === '\x03') break; // Ctrl-C: consumed by runUntilEvent
                    if (inbuf[0] !== '$') { inbuf = inbuf.replace(/^[^$]*/, ''); if (!inbuf) return; continue; }
                    const hash = inbuf.indexOf('#');
                    if (hash < 0 || inbuf.length < hash + 3) return; // incomplete
                    const data = inbuf.slice(1, hash);
                    inbuf = inbuf.slice(hash + 3);
                    if (!noAck) sendRaw('+');
                    await handle(data);
                }
            });
        });
        sock.on('close', () => { closed = true; });
        sock.on('error', () => { closed = true; });
    });

    await new Promise((resolve) => server.listen(port, '127.0.0.1', resolve));
    return { port: server.address().port, emu, close: () => server.close() };
}
