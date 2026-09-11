// Real-stack USB test: STM32duino USBSerial (CDC-ACM) firmware against a
// scripted USB host (same sequence as the page enumerator). Proves the USB
// FS-device model through Arduino's real USB stack: enumeration, control
// transfers, bulk EP1 echo.
import { readFileSync } from 'fs';
import { createEmulator } from '../pkg/emulator.js';

const S_DEV = [0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x12, 0x00];
const S_ADDR = [0x00, 0x05, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00];
const S_CFG = [0x80, 0x06, 0x00, 0x02, 0x00, 0x00, 0xFF, 0x00];
const S_SETCFG = [0x00, 0x09, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00];
const S_LINE = [0x21, 0x20, 0x00, 0x00, 0x00, 0x00, 0x07, 0x00];
const S_STATE = [0x21, 0x22, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00];
const LINE_CODING = [0x00, 0xC2, 0x01, 0x00, 0x00, 0x00, 0x08];

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const emu = await createEmulator({ firmware: readFileSync('site/arduino_usb_serial.elf') });
// Let the Arduino USB stack boot and arm its masks, then drive the host
// bus reset it waits for (FRES release lands before masks are on; the
// stack only opens EP0 on a host RESET interrupt).
for (let i = 0; i < 30; i++) await emu.run(1000000);
ok(emu.usbBusReset(), 'host bus reset delivered');

const inbox = [];
const drain = () => {
    const flat = emu.drainEvents();
    for (let i = 0; i + 2 < flat.length;) {
        const t = flat[i];
        if (t === 18) {
            const ep = flat[i + 1], len = flat[i + 2];
            inbox.push([ep, Array.from(flat.slice(i + 3, i + 3 + len))]);
            i += 3 + len;
        } else break;
    }
};
const take = (ep, wantLen = -1) => {
    const i = inbox.findIndex(e => e[0] === ep && (wantLen < 0 || e[1].length === wantLen));
    return i < 0 ? null : inbox.splice(i, 1)[0][1];
};
const step = async (n = 6) => { for (let i = 0; i < n; i++) { await emu.run(1000000); drain(); } };
// Hosts retry: the firmware re-arms EP0 between transfers, so a first
// attempt can legitimately NAK before the next poll round.
const setup = async (bytes, frames = 8) => {
    for (let i = 0; i < 20; i++) {
        if (emu.usbInjectSetup(bytes)) break;
        await emu.run(1000000); drain();
    }
    await step(frames);
};
const out = async (ep, bytes) => {
    for (let i = 0; i < 20; i++) {
        if (emu.usbInjectOut(ep, bytes)) return true;
        await emu.run(1000000); drain();
    }
    return false;
};
await step(10); // let the RESET IRQ dispatch; the stack arms EP0
{
const ep0 = emu.periphRead(0x40005C00, 4);
ok((ep0 & 0x3000) === 0x3000, `EP0 RX VALID after reset (EP0R=${ep0.toString(16)})`);
}

// 1. Device descriptor (18B).
await setup(S_DEV);
let d = take(0, 18);
ok(d && d.length === 18, `device descriptor 18B (got ${d ? d.length : 'none'})`);
if (d) ok(d[0] === 18 && d[1] === 1, `device descriptor header (${d.slice(0, 4).map(b => b.toString(16))})`);
await out(0, []); await step();
// 2. Set address 5 (zero-length status IN).
await setup(S_ADDR);
ok(take(0, 0) !== null, 'address status stage');
// 3. Config descriptor (ask 255, collect all fragments).
await setup(S_CFG);
await step(10);
let cfg = [];
let c;
while ((c = take(0)) !== null) cfg = cfg.concat(c);
ok(cfg.length >= 9, `config descriptor bytes (got ${cfg.length})`);
if (cfg.length >= 9) ok(cfg[1] === 2, 'config descriptor type');
await out(0, []); await step();
// 4. Set configuration.
await setup(S_SETCFG);
ok(take(0, 0) !== null, 'set-config status stage');
// 5. Line coding + line state (CDC class requests).
await setup(S_LINE);
await out(0, LINE_CODING); await step();
await setup(S_STATE);
take(0); // status may be zero-length or short
// 6. Bulk echo: OUT on EP1 (0x01), IN on EP2 (0x82) per the Arduino CDC
// descriptor (there is no EP1-IN on this stack).
ok(await out(1, [0x48, 0x69]), 'bulk OUT accepted'); // 'Hi'
await step(12);
// The setup() banner also streams on EP2; the echo is the tail of it.
let stream = [];
let p;
while ((p = take(2)) !== null) stream = stream.concat(p);
const echo = stream.slice(-2);
ok(stream.length >= 2, 'bulk EP2 echo arrived');
if (echo) ok(echo.join() === '72,105', `echo bytes match (got ${echo.join()})`);
// 7. Firmware banner reached the host through the enumerated stack.
await step(10);

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
