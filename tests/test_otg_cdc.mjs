// OTG_FS CDC end-to-end: bare-metal register-level firmware on the F105
// SVD map, driven by a scripted host over otg_inject_setup/out. Proves the
// OTG_FS device model through real machine code: core reset, bus reset,
// enumeration (device/config descriptors, address, configure, CDC class),
// bulk EP1 echo. Mirrors tests/test_usb_cdc.mjs for the FS core.
import { readFileSync } from 'fs';
import { createEmulator } from '../pkg/emulator.js';

const ELF = 'site/otg_cdc.elf';

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const svd = readFileSync('svd/STM32F105xx.svd', 'utf8');
const emu = await createEmulator({
  firmware: readFileSync(ELF),
  chip: { name: 'STM32F105', svd, flash: 0x40000, ram: 0x10000 },
});
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
const takeIn = (ep) => {
  const i = inbox.findIndex(e => e[0] === ep);
  if (i < 0) return null;
  return inbox.splice(i, 1)[0][1];
};
// Inject with retries: the firmware arms endpoints as it polls, so a first
// attempt can legitimately drop before the next poll round.
const injectSetup = async (bytes) => {
  for (let i = 0; i < 20; i++) {
    if (emu.otgInjectSetup(bytes)) return true;
    await emu.step(1000000); drain();
  }
  return false;
};
const injectOut = async (ep, bytes) => {
  for (let i = 0; i < 20; i++) {
    if (emu.otgInjectOut(ep, bytes)) return true;
    await emu.step(1000000); drain();
  }
  return false;
};
const settle = async (n = 3) => { for (let i = 0; i < n; i++) { await emu.step(1000000); drain(); } };
const hex = (a) => a.map(b => b.toString(16).padStart(2, '0')).join(' ');

for (let i = 0; i < 5; i++) { await emu.step(1000000); drain(); } // boot
ok(emu.otgBusReset(), 'host bus reset delivered');
await settle(4); // RESET dispatch; firmware opens EP0

// 1. GET_DESCRIPTOR DEVICE (18 bytes, single packet)
ok(await injectSetup([0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x12, 0x00]), 'device desc SETUP accepted');
await settle();
let d = takeIn(0);
ok(d && d.length === 18 && d[0] === 18 && d[1] === 1, `device desc 18B type 1 (got ${d ? hex(d.slice(0, 4)) : 'none'})`);
ok(await injectOut(0, []), 'device desc status OUT accepted');
await settle();

// 2. SET_ADDRESS(5): zero-length IN status, then DCFG applies
ok(await injectSetup([0x00, 0x05, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00]), 'SET_ADDRESS SETUP accepted');
await settle();
d = takeIn(0);
ok(d && d.length === 0, 'SET_ADDRESS status IN (zero-length)');
const dcDad = (emu.periphRead(0x50000800, 4) >>> 4) & 0x7F;
ok(dcDad === 5, `DCFG DAD = 5 (got ${dcDad})`);

// 3. GET_DESCRIPTOR CONFIG (75 bytes across two packets: 64 + 11)
ok(await injectSetup([0x80, 0x06, 0x00, 0x02, 0x00, 0x00, 0xFF, 0x00]), 'config desc SETUP accepted');
await settle(5);
const c1 = takeIn(0), c2 = takeIn(0);
const cfg = [...(c1 || []), ...(c2 || [])];
ok(cfg.length === 75, `config desc 75B in 2 packets (got ${cfg.length})`);
ok(cfg[0] === 9 && cfg[1] === 2 && cfg[2] === 75 && cfg[3] === 0, 'config header (9,2,len 75)');
ok(cfg.length > 70 && cfg[68] === 7 && cfg[69] === 5 && cfg[70] === 0x81, 'EP1 IN bulk descriptor');
ok(await injectOut(0, []), 'config desc status OUT accepted');
await settle();

// 4. SET_CONFIGURATION(1)
ok(await injectSetup([0x00, 0x09, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00]), 'SET_CONFIGURATION accepted');
await settle();
ok((takeIn(0) || []).length === 0, 'SET_CONFIGURATION status IN');

// 5. CDC class: SET_LINE_CODING (7-byte OUT) + SET_CONTROL_LINE_STATE
ok(await injectSetup([0x21, 0x20, 0x00, 0x00, 0x00, 0x00, 0x07, 0x00]), 'SET_LINE_CODING SETUP accepted');
ok(await injectOut(0, [0x00, 0xC2, 0x01, 0x00, 0x00, 0x00, 0x08]), 'line coding 115200 8N1 accepted');
await settle();
ok((takeIn(0) || []).length === 0, 'SET_LINE_CODING status IN');
ok(await injectSetup([0x21, 0x22, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00]), 'SET_CONTROL_LINE_STATE accepted');
await settle();
ok((takeIn(0) || []).length === 0, 'SET_CONTROL_LINE_STATE status IN');

// 6. Bulk echo twice on EP1 (validates re-arm across transfers)
for (const [msg, name] of [[[72, 105], 'Hi'], [[66, 121, 101], 'Bye']]) {
  ok(await injectOut(1, msg), `bulk OUT '${name}' accepted`);
  await settle(5);
  const echo = takeIn(1);
  ok(echo && echo.join(',') === msg.join(','), `bulk echo '${name}' exact (${echo})`);
}

await emu.close();
console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
