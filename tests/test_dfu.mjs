// USB DFU bootloader regression test: the arduino_dfu firmware implements a
// Maple-style DFU device (EP0 control only) on the FS USB core. The test
// drives host enumeration + the DFU download flow over usb_inject_* and
// asserts every reply from UsbIn events: descriptors (VID/PID + DFU
// functional), address/configure, GETSTATUS/GETSTATE states,
// SetAddressPointer, two DNLOAD blocks with staged-image readback,
// UPLOAD readback, manifest completion, and ABORT recovery.
//
// Guards the DFU class path end-to-end (SETUP/OUT staging, flash unlock +
// program sequence, state machine, manifest). Like the HCD proof, the
// staged image is resolved from ELF symbols (dfu_stage), since guest
// stores to flash are dropped by the memory model.
import { readFileSync } from 'fs';
import { STM32F1 } from '../pkg/stm32f1.js';

const ELF = 'site/arduino_dfu.elf';

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const mcu = await STM32F1.fromELF(readFileSync(ELF));
const emu = mcu._emu;
const inbox = [];
mcu.onUsbIn = (ep, data) => { inbox.push([ep, Array.from(data)]); };
const takeIn = (ep) => {
    const i = inbox.findIndex(e => e[0] === ep);
    if (i < 0) return null;
    return inbox.splice(i, 1)[0][1];
};
// Inject with retries: the firmware arms endpoints as it polls, so a first
// attempt can legitimately NAK before the next poll round.
const injectSetup = async (bytes) => {
    for (let i = 0; i < 20; i++) {
        if (emu.usbInjectSetup(bytes)) return true;
        mcu.step(1000000);
    }
    return false;
};
const injectOut = async (ep, bytes) => {
    for (let i = 0; i < 20; i++) {
        if (emu.usbInjectOut(ep, bytes)) return true;
        mcu.step(1000000);
    }
    return false;
};
const settle = (n = 3) => { for (let i = 0; i < n; i++) mcu.step(1000000); };
const hex = (a) => a.map(b => b.toString(16).padStart(2, '0')).join(' ');

mcu.step(5000000); // boot to attach (FRES release alone is not a reset)
emu.usbBusReset(); // host SE0: RESET event -> firmware arms EPs
settle();

// Resolve the RAM stage buffer + trace from ELF symbols (never hardcode:
// rebuilds move .bss around — cf. canRxArmed). Match whole identifiers:
// 'dfu_trace' is a prefix of 'dfu_trace_n' (and 'dfu_stage' of
// 'dfu_staged'), so a naive substring find resolves the wrong symbol and
// shifts every read by a word.
const { createEmulator, parseElf } = await import('../pkg/emulator.js');
const syms = parseElf(readFileSync(ELF)).symbols;
const sym = (frag) => {
    const re = new RegExp(frag + '(?![A-Za-z_])');
    const s = syms.find(s => re.test(s.name));
    if (!s) throw new Error(`symbol missing: ${frag}`);
    return s.addr >>> 0;
};
const STAGE = sym('dfu_stage'), TRACE = sym('dfu_trace'), TRACEN = sym('dfu_trace_n');
const trace = () => {
    const n = emu.memRead32(TRACEN) >>> 0;
    const t = [];
    for (let k = 0; k < n && k < 8; k++) t.push(emu.memRead32(TRACE + k * 4) >>> 0);
    return t;
};
const stageBytes = (off, n) => {
    const out = [];
    for (let k = 0; k < n; k += 4) {
        const w = emu.memRead32(STAGE + off + k) >>> 0;
        for (let j = 0; j < 4 && off + k + j < off + n; j++) out.push((w >>> (j * 8)) & 0xFF);
    }
    return out;
};

// 1. GET_DESCRIPTOR DEVICE (Maple VID/PID).
ok(await injectSetup([0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x12, 0x00]), 'device desc SETUP accepted');
settle();
let d = takeIn(0);
ok(d && d.length === 18 && d[8] === 0xAF && d[9] === 0x1E && d[10] === 0x03 && d[11] === 0x00,
    `device desc Maple 1EAF:0003 (got ${d ? hex(d.slice(8, 12)) : 'none'})`);
ok(await injectOut(0, []), 'device desc status OUT accepted');
settle();

// 2. SET_ADDRESS(5) + 3. GET_DESCRIPTOR CONFIG (27B: config + interface + DFU functional).
ok(await injectSetup([0x00, 0x05, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00]), 'SET_ADDRESS accepted');
settle();
ok((takeIn(0) || []).length === 0, 'SET_ADDRESS status IN');
ok(await injectSetup([0x80, 0x06, 0x00, 0x02, 0x00, 0x00, 0xFF, 0x00]), 'config desc SETUP accepted');
settle(5);
const c = takeIn(0) || [];
ok(c.length === 27 && c[0] === 9 && c[1] === 2 && c[2] === 27, `config 27B header (got ${c.length})`);
ok(c[9] === 9 && c[10] === 4 && c[14] === 0xFE && c[15] === 1 && c[16] === 2, 'DFU interface (FE/1/2)');
ok(c[18] === 9 && c[19] === 0x21 && c[23] === 64, 'DFU functional descriptor (type 0x21, xfer 64)');
ok(await injectOut(0, []), 'config status OUT accepted');
settle();

// 4. SET_CONFIGURATION -> trace[0] = 1 (enumerated).
ok(await injectSetup([0x00, 0x09, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00]), 'SET_CONFIGURATION accepted');
settle();
ok((takeIn(0) || []).length === 0, 'SET_CONFIGURATION status IN');
ok(trace().join(',') === '1', `trace shows configured (got ${trace()})`);

// 5. GETSTATUS -> dfuIDLE (2); GETSTATE -> 2.
ok(await injectSetup([0xA1, 0x03, 0x00, 0x00, 0x00, 0x00, 0x06, 0x00]), 'GETSTATUS accepted');
settle();
d = takeIn(0);
ok(d && d.length === 6 && d[4] === 2, `GETSTATUS dfuIDLE (got ${d ? hex(d) : 'none'})`);
ok(await injectOut(0, []), 'GETSTATUS status OUT accepted');
settle();
ok(await injectSetup([0xA1, 0x05, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00]), 'GETSTATE accepted');
settle();
d = takeIn(0);
ok(d && d.length === 1 && d[0] === 2, 'GETSTATE returns dfuIDLE');
ok(await injectOut(0, []), 'GETSTATE status OUT accepted');
settle();

// 6. SetAddressPointer (block 0, 0x21 + LE32 0x08005000) -> trace 1,2.
ok(await injectSetup([0x21, 0x01, 0x00, 0x00, 0x00, 0x00, 0x05, 0x00]), 'SetAddress SETUP accepted');
ok(await injectOut(0, [0x21, 0x00, 0x50, 0x00, 0x08]), 'SetAddressPointer 0x08005000 accepted');
settle();
ok((takeIn(0) || []).length === 0, 'SetAddress status IN');
ok(trace().join(',') === '1,2', `trace shows address set (got ${trace()})`);

// 7. DNLOAD block 1 (64B pattern) -> staged bytes + trace 1,2,3.
const blk1 = [];
for (let i = 0; i < 64; i++) blk1.push((0xA0 + i) & 0xFF);
ok(await injectSetup([0x21, 0x01, 0x01, 0x00, 0x00, 0x00, 64, 0x00]), 'DNLOAD block1 SETUP accepted');
ok(await injectOut(0, blk1), 'DNLOAD block1 data accepted');
settle();
ok((takeIn(0) || []).length === 0, 'DNLOAD block1 status IN');
ok(trace().join(',') === '1,2,3', `trace shows block programmed (got ${trace()})`);
ok(stageBytes(0, 64).join(',') === blk1.join(','), 'staged image matches block 1');

// 8. DNLOAD block 2 (short 16B tail) -> staged append.
const blk2 = [];
for (let i = 0; i < 16; i++) blk2.push((0x50 + i) & 0xFF);
ok(await injectSetup([0x21, 0x01, 0x02, 0x00, 0x00, 0x00, 16, 0x00]), 'DNLOAD block2 SETUP accepted');
ok(await injectOut(0, blk2), 'DNLOAD short block accepted');
settle();
ok((takeIn(0) || []).length === 0, 'DNLOAD block2 status IN');
ok(stageBytes(64, 16).join(',') === blk2.join(','), 'staged image appends block 2');

// 9. UPLOAD block 1 -> programmed bytes back.
ok(await injectSetup([0xA1, 0x02, 0x01, 0x00, 0x00, 0x00, 64, 0x00]), 'UPLOAD block1 accepted');
settle();
d = takeIn(0);
ok(d && d.join(',') === blk1.join(','), 'UPLOAD returns staged block 1');
ok(await injectOut(0, []), 'UPLOAD status OUT accepted');
settle();

// 10. Manifest (block 0, zero length) -> trace done (1,2,3,3,4).
ok(await injectSetup([0x21, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]), 'manifest SETUP accepted');
settle();
ok((takeIn(0) || []).length === 0, 'manifest status IN');
ok(trace().join(',') === '1,2,3,3,4', `trace shows manifest (got ${trace()})`);
ok(await injectSetup([0xA1, 0x03, 0x00, 0x00, 0x00, 0x00, 0x06, 0x00]), 'GETSTATUS after manifest');
settle();
d = takeIn(0);
ok(d && d[4] === 7, `GETSTATUS MANIFEST state (got ${d ? d[4] : 'none'})`);
ok(await injectOut(0, []), 'manifest GETSTATUS status OUT accepted');
settle();

// 11. Error + recovery: bad address STALLs (-> ERROR), ABORT returns to IDLE.
ok(await injectSetup([0x21, 0x01, 0x00, 0x00, 0x00, 0x00, 0x05, 0x00]), 'bad-addr SETUP accepted');
ok(await injectOut(0, [0x21, 0x00, 0x00, 0x00, 0x08]), 'bad address (0x08000000) sent');
settle(5);
ok(await injectSetup([0xA1, 0x05, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00]), 'GETSTATE after error');
settle();
d = takeIn(0);
ok(d && d[0] === 10, `GETSTATE dfuERROR (got ${d ? d[0] : 'none'})`);
ok(await injectOut(0, []), 'error GETSTATE status OUT accepted');
settle();
ok(await injectSetup([0x21, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]), 'ABORT accepted');
settle();
ok((takeIn(0) || []).length === 0, 'ABORT status IN');
ok(await injectSetup([0xA1, 0x05, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00]), 'GETSTATE after ABORT');
settle();
d = takeIn(0);
ok(d && d[0] === 2, `GETSTATE back to IDLE (got ${d ? d[0] : 'none'})`);
ok(await injectOut(0, []), 'final status OUT accepted');
settle();

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
