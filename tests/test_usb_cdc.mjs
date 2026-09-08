// USB CDC serial regression test: the arduino_usb_cdc firmware implements a
// register-level CDC-ACM device (EP0 control + EP1 bulk echo). The test
// drives a full host enumeration over usb_inject_* and asserts every reply
// from UsbIn events: device/config descriptors, address/configure/class
// status stages, then a bulk OUT->IN echo.
//
// Guards the whole USB device path end-to-end (SETUP/OUT staging, IN
// completion events, DTOG sequencing, PMA buffers, DADDR).
import { readFileSync } from 'fs';
import { STM32F1 } from '../pkg/stm32f1.js';

const ELF = 'site/arduino_usb_cdc.elf';

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

mcu.step(5000000); // boot: FRES release -> RESET -> EP arming

// 1. GET_DESCRIPTOR DEVICE (18 bytes, single packet)
ok(await injectSetup([0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x12, 0x00]), 'device desc SETUP accepted');
settle();
let d = takeIn(0);
ok(d && d.length === 18 && d[0] === 18 && d[1] === 1, `device desc 18B type 1 (got ${d ? hex(d.slice(0, 4)) : 'none'})`);
ok(await injectOut(0, []), 'device desc status OUT accepted');
settle();

// 2. SET_ADDRESS(5): zero-length IN status, then DADDR applies
ok(await injectSetup([0x00, 0x05, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00]), 'SET_ADDRESS SETUP accepted');
settle();
d = takeIn(0);
ok(d && d.length === 0, 'SET_ADDRESS status IN (zero-length)');

// 3. GET_DESCRIPTOR CONFIG (68 bytes: 64 + 4 across two packets)
ok(await injectSetup([0x80, 0x06, 0x00, 0x02, 0x00, 0x00, 68, 0x00]), 'config desc SETUP accepted');
settle(5);
const c1 = takeIn(0), c2 = takeIn(0);
const cfg = [...(c1 || []), ...(c2 || [])];
ok(cfg.length === 68, `config desc 68B in 2 packets (got ${cfg.length})`);
ok(cfg[0] === 9 && cfg[1] === 2 && cfg[2] === 68 && cfg[3] === 0, 'config header (9,2,len 68)');
ok(cfg[9] === 8 && cfg[10] === 11, 'IAD present (8,11)');
ok(cfg.length > 60 && cfg[61] === 7 && cfg[62] === 5 && cfg[63] === 0x81, 'EP1 IN bulk descriptor');
ok(await injectOut(0, []), 'config desc status OUT accepted');
settle();

// 4. SET_CONFIGURATION(1)
ok(await injectSetup([0x00, 0x09, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00]), 'SET_CONFIGURATION accepted');
settle();
ok((takeIn(0) || []).length === 0, 'SET_CONFIGURATION status IN');

// 5. CDC class: SET_LINE_CODING (7-byte OUT) + SET_CONTROL_LINE_STATE
ok(await injectSetup([0x21, 0x20, 0x00, 0x00, 0x00, 0x00, 0x07, 0x00]), 'SET_LINE_CODING SETUP accepted');
ok(await injectOut(0, [0x00, 0xC2, 0x01, 0x00, 0x00, 0x00, 0x08]), 'line coding 115200 8N1 accepted');
settle();
ok((takeIn(0) || []).length === 0, 'SET_LINE_CODING status IN');
ok(await injectSetup([0x21, 0x22, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00]), 'SET_CONTROL_LINE_STATE accepted');
settle();
ok((takeIn(0) || []).length === 0, 'SET_CONTROL_LINE_STATE status IN');

// 6. Bulk echo twice (validates EP1 re-arm across transfers)
for (const [msg, name] of [[[72, 105], 'Hi'], [[66, 121, 101], 'Bye']]) {
    ok(await injectOut(1, msg), `bulk OUT '${name}' accepted`);
    settle(5);
    const echo = takeIn(1);
    ok(echo && echo.join(',') === msg.join(','), `bulk echo '${name}' exact (${echo})`);
}

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
