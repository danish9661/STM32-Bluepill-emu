// Dual-CAN regression test: the arduino_can_dual firmware talks to itself
// on CAN1 + CAN2 (loopback, accept-all filters) on the STM32F105 SVD map —
// CAN2 @ 0x40006800 exists only there, so this is the F105 differentiator
// proven end to end. No transceiver needed.
import { readFileSync } from 'fs';
import { createEmulator } from '../pkg/emulator.js';

const ELF = 'site/arduino_can_dual.elf';
const MAX = 40000000;
const CHUNK = 5000000;

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const svd = readFileSync('svd/STM32F105xx.svd', 'utf8');
// F105RC-class sizes: the SVD path defaults to 64K/20K, but firmware
// linked for 48K RAM (SP=0x2000C000) needs the RAM window to cover it —
// otherwise stack pushes land unmapped (writes dropped, pops read 0)
// and startup dies in _init with a bogus LR (IBUSERR).
const emu = await createEmulator({ firmware: readFileSync(ELF), chip: { name: 'STM32F105', svd, flash: 0x40000, ram: 0x10000 } });
let done = 0;
while (done < MAX) {
    const r = await emu.run(Math.min(CHUNK, MAX - done));
    done += CHUNK;
    if (r.stopped) break;
}
const out = String(emu.getUartOutput() || '');
const lines = out.split('\n').map(s => s.trim()).filter(Boolean);
ok(out.includes('=== Dual-CAN demo'), 'banner printed');
ok(out.includes('both buses up'), 'CAN1 + CAN2 initialized');
const c1 = lines.filter(s => /^CAN1 tx id=[0-9A-F]+ data=[0-9A-F]+  ->  rx id=[0-9A-F]+ data=[0-9A-F]+$/.test(s));
const c2 = lines.filter(s => /^CAN2 tx id=[0-9A-F]+ data=[0-9A-F]+  ->  rx id=[0-9A-F]+ data=[0-9A-F]+$/.test(s));
ok(c1.length >= 2, `CAN1 loopback frames (${c1.length})`);
ok(c2.length >= 2, `CAN2 loopback frames (${c2.length})`);
const roundtrip = (s) => {
    const m = s.match(/^CAN\d tx id=([0-9A-F]+) data=([0-9A-F]+)  ->  rx id=([0-9A-F]+) data=([0-9A-F]+)$/);
    return m && m[1] === m[3] && m[2] === m[4];
};
ok(c1.every(roundtrip) && c2.every(roundtrip), 'tx matches rx on both buses');

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
