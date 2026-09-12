// OTG_FS host end-to-end: bare-metal HCD firmware on the F105 SVD map
// enumerates a scripted virtual USB device and runs a bulk echo. The
// FIRMWARE drives (host); the TEST plays the device, answering HostTx /
// HostRx events. Firmware progress is recorded in its RAM trace[] (see
// tests/otg_host/main.c): 1 = port reset, 2 = dev desc, 3 = address,
// 4 = config desc, 5 = set-config, 6 = bulk OUT, 7 = echo verified,
// 8 = done.
import { readFileSync } from 'fs';
import { createEmulator } from '../pkg/emulator.js';

const ELF = 'site/otg_host.elf';
const TRACE = 0x20000084, TRACE_N = 0x20000080;

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const svd = readFileSync('svd/STM32F105xx.svd', 'utf8');
const emu = await createEmulator({
  firmware: readFileSync(ELF),
  chip: { name: 'STM32F105', svd, flash: 0x40000, ram: 0x10000 },
});
// Virtual device descriptors (same shapes as the OTG device demo).
const DEV = [18, 1, 0x00, 0x02, 0xEF, 0x02, 0x01, 64, 0x83, 0x04, 0x40, 0x57, 0x00, 0x01, 1, 2, 0, 1];
const CFG = [9, 2, 75, 0, 2, 1, 0, 0x80, 100, 8, 11, 0, 2, 0x02, 0x02, 0x01, 0,
  9, 4, 0, 0, 1, 0x02, 0x02, 0x01, 0, 5, 0x24, 0x00, 0x10, 0x01, 5, 0x24, 0x01, 0x00, 1,
  4, 0x24, 0x02, 0x02, 5, 0x24, 0x06, 0, 1, 7, 5, 0x83, 0x03, 8, 0, 10,
  9, 4, 1, 0, 2, 0x0A, 0x00, 0x00, 0, 7, 5, 0x01, 0x02, 64, 0, 0, 7, 5, 0x81, 0x02, 64, 0, 0];
const txQ = [], rxQ = [];
const drain = () => {
  const flat = emu.drainEvents();
  for (let i = 0; i < flat.length;) {
    const t = flat[i++];
    if (t === 20) {
      const ch = flat[i++], ep = flat[i++], su = flat[i++], ln = flat[i++];
      txQ.push({ ch, ep, setup: su !== 0, data: Array.from(flat.slice(i, i + ln)) });
      i += ln;
    } else if (t === 21) {
      rxQ.push({ ch: flat[i++], ep: flat[i++], len: flat[i++] });
    } else break;
  }
};
const step = async (n = 1) => { for (let i = 0; i < n; i++) { await emu.step(1000000); drain(); } };

// Virtual-device responder state.
let lastSetup = null, bulkOut = null, echoFed = null;
const pump = async () => {
  await step();
  for (const t of txQ.splice(0)) {
    if (t.setup) lastSetup = t.data;
    else if (t.ep === 1 && t.data.length > 0) bulkOut = t.data;
  }
  for (const r of rxQ.splice(0)) {
    if (lastSetup && lastSetup[1] === 0x06 && lastSetup[3] === 1) emu.otgHostFeedIn(r.ep, DEV, false);
    else if (lastSetup && lastSetup[1] === 0x06 && lastSetup[3] === 2) emu.otgHostFeedIn(r.ep, CFG, false);
    else if (r.ep === 1 && bulkOut) { echoFed = bulkOut.slice(); emu.otgHostFeedIn(r.ep, bulkOut, false); }
    else emu.otgHostFeedIn(r.ep, [], false);
  }
};

for (let i = 0; i < 5; i++) await emu.step(1000000);
ok(emu.otgHostAttach(true) === true, 'virtual device attach accepted');
// Run until the firmware trace reaches 8 (done) or a step budget expires.
let trace = [];
for (let i = 0; i < 120; i++) {
  await pump();
  const n = emu.memRead32(TRACE_N) >>> 0;
  if (n > 0 && n < 12) {
    trace = [];
    for (let k = 0; k < n; k++) trace.push(emu.memRead32(TRACE + k * 4) >>> 0);
    if (trace.includes(8)) break;
  }
}
ok(trace.join(',') === '1,2,3,4,5,6,7,8', `firmware trace exact 1..8 (got ${trace.join(',')})`);
ok(echoFed !== null && echoFed.join(',') === '72,105', `bulk echo bytes exact (${echoFed})`);
// Descriptor bytes the firmware validated itself (trace 2/4 only advance
// on exact header match); link still attached, SOF flowing.
ok((emu.periphRead(0x50000440, 4) & 1) === 1, 'HPRT PCSTS still set');
const f0 = emu.periphRead(0x50000408, 4) & 0xFFFF;
await emu.step(200000); drain();
ok((emu.periphRead(0x50000408, 4) & 0xFFFF) !== f0, 'HFNUM advances');

await emu.close();
console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
