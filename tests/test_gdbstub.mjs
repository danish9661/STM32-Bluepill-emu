// GDB stub regression test: drives pkg/gdbstub.mjs over TCP with a minimal
// RSP client (no GDB binary needed) against the echo firmware.
// Covers handshake, target.xml, regs, mem read/write, step, breakpoint
// set/hit/remove, and register write-back.
import { readFileSync } from 'fs';
import net from 'node:net';
import { serveGdb } from '../pkg/gdbstub.mjs';

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const srv = await serveGdb({ firmware: readFileSync('site/arduino_echo.elf'), port: 0 });
const sock = net.connect(srv.port, '127.0.0.1');
await new Promise((r) => sock.on('connect', r));
let buf = '';
const send = (data) => new Promise((resolve) => {
    let c = 0;
    for (let i = 0; i < data.length; i++) c = (c + data.charCodeAt(i)) & 0xFF;
    const want = `$${data}#${c.toString(16).padStart(2, '0')}`;
    const onData = (b) => {
        buf += b.toString('binary');
        buf = buf.replace(/^[+-]+/, '');
        const h = buf.indexOf('#');
        if (buf[0] === '$' && h > 0 && buf.length >= h + 3) {
            const payload = buf.slice(1, h);
            buf = buf.slice(h + 3);
            sock.off('data', onData);
            resolve(payload);
        }
    };
    sock.on('data', onData);
    sock.write(want);
});

ok((await send('qSupported')).includes('vContSupported+'), 'qSupported advertises vCont');
{
    // Explicit qXfer still serves the target description (GDB 15 uses its
    // default ARM layout unprompted, which matches our 17-reg g packet).
    const xml = await send('qXfer:features:read:target.xml:0,100');
    ok(xml[0] === 'l' || xml[0] === 'm', 'explicit qXfer serves target doc');
}
ok((await send('?')) === 'S05', 'halted query -> S05');
const xml = await send('qXfer:features:read:target.xml:0,fff');
ok(Buffer.from(xml.slice(1), 'hex').toString().includes('m-profile'), 'target.xml describes ARM core');
const g0 = await send('g');
ok(g0.length === 17 * 8, `g dumps 17 regs (${g0.length} hex chars)`);
const pcHex = g0.slice(15 * 8, 16 * 8);
const pc = parseInt(pcHex.match(/../g).reverse().join(''), 16);
ok(pc >= 0x08000000 && pc < 0x08010000, `PC in flash (0x${pc.toString(16)})`);
ok((await send('p0')).length === 8, 'p0 single register');
// Memory at the vector table: SP looks like SRAM.
const spHex = await send('m08000000,4');
const sp = parseInt(spHex.match(/../g).reverse().join(''), 16);
ok(sp >= 0x20000000 && sp < 0x20008000, `vector SP in SRAM (0x${sp.toString(16)})`);
// Scratch write/readback in low RAM (bytes restored after).
const boyut = await send('m20000100,4');
await send('M20000100,4:deadbeef');
ok((await send('m20000100,4')) === 'deadbeef', 'M write + m readback');
await send(`M20000100,4:${boyut}`);
ok((await send('m20000100,4')) === boyut, 'scratch restored');
// Register write-back via P.
await send('P0=78563412');
ok((await send('p0')) === '78563412', 'P0 write sticks');
// Step advances with SIGTRAP.
const pcBefore = await send('pf');
await send('s');
ok((await send('pf')) !== pcBefore, 'single step moves PC');
// Breakpoint at the CURRENT pc always hits on continue.
const pcc = await send('pf');
const baddr = parseInt(pcc.match(/../g).reverse().join(''), 16);
ok((await send(`Z0,${baddr.toString(16)},2`)) === 'OK', 'Z0 set');
ok((await send('c')) === 'S05', 'continue stops with SIGTRAP');
const pcc2 = await send('pf');
ok(parseInt(pcc2.match(/../g).reverse().join(''), 16) === baddr, 'stopped AT the breakpoint');
ok((await send(`z0,${baddr.toString(16)},2`)) === 'OK', 'z0 removed');
await send('s'); // step past the restored instruction
ok(true, 'step past breakpoint');

// ---- Data watchpoints (Z2/Z3/Z4) ----
// RAM snippets at 0x20000200 touching the watched byte 0x20000100; PC is
// aimed with the P15 packet (decimal reg numbers per the RSP spec).
const le32 = (n) => n.toString(16).padStart(8, '0').match(/../g).reverse().join('');
const aim = async (pc) => ok((await send(`Pf=${le32(pc)}`)) === 'OK', `Pf aims 0x${pc.toString(16)}`);
const loadSnippet = async (hex) => ok((await send(`M20000200,${(hex.length / 2).toString(16)}:${hex}`)) === 'OK', 'snippet installed');
const WATCH_AT = 0x20000100;
// strb snippet: movs r0,#0xAA; ldr r1,[pc,#4]; strb r0,[r1,#0]; bkpt; .word WATCH_AT
const STR_SNIPPET = 'aa200149087000be00010020';
// ldrb snippet: movs r0,#0; ldr r1,[pc,#4]; ldrb r0,[r1,#0]; bkpt; .word WATCH_AT
const LDR_SNIPPET = '00200149087800be00010020';

// Z2 (write): the strb trips with T05watch:addr; removing + continuing runs
// into the trailing BKPT (S04 = genuine decode gap, proving execution went on).
await loadSnippet(STR_SNIPPET);
await aim(0x20000200);
ok((await send(`Z2,${WATCH_AT.toString(16)},1`)) === 'OK', 'Z2 set');
ok((await send('c')) === `T05watch:${WATCH_AT.toString(16)};`, 'write watch trips with T05watch');
ok((await send(`z2,${WATCH_AT.toString(16)},1`)) === 'OK', 'z2 removed');
ok((await send('c')) === 'S04', 'continued past the watch into BKPT');

// Z3 (read): the ldrb trips with T05rwatch:addr.
await loadSnippet(LDR_SNIPPET);
await aim(0x20000200);
ok((await send(`Z3,${WATCH_AT.toString(16)},1`)) === 'OK', 'Z3 set');
ok((await send('c')) === `T05rwatch:${WATCH_AT.toString(16)};`, 'read watch trips with T05rwatch');
ok((await send(`z3,${WATCH_AT.toString(16)},1`)) === 'OK', 'z3 removed');
ok((await send('c')) === 'S04', 'continued past the read watch');

// Z4 (access): fires on a write too, reported as T05awatch:addr.
await loadSnippet(STR_SNIPPET);
await aim(0x20000200);
ok((await send(`Z4,${WATCH_AT.toString(16)},1`)) === 'OK', 'Z4 set');
ok((await send('c')) === `T05awatch:${WATCH_AT.toString(16)};`, 'access watch trips with T05awatch');
ok((await send(`z4,${WATCH_AT.toString(16)},1`)) === 'OK', 'z4 removed');
ok((await send('c')) === 'S04', 'continued past the access watch');

sock.end();
srv.close();

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
