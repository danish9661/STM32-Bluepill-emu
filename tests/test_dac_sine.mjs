// DAC sine + ADC loopback regression test: the arduino_dac_sine firmware
// drives DAC1 CH1 (PA4) through a 16-point sine table while ADC1 CH4
// samples the same pin via the emulator's analog loopback.
//
// Guards the DAC->ADC wire (DOR1 -> channel voltage -> DR). No devices.
import { readFileSync } from 'fs';
import { createEmulator } from '../pkg/emulator.js';

const ELF = 'site/arduino_dac_sine.elf';
const MAX = 120000000;
const CHUNK = 10000000;

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const emu = await createEmulator({ firmware: readFileSync(ELF) });
let done = 0;
while (done < MAX) {
    const r = await emu.run(Math.min(CHUNK, MAX - done));
    done += CHUNK;
    if (r.stopped) break;
}
const out = String(emu.getUartOutput() || '');
ok(out.includes('=== DAC sine + ADC loopback demo'), 'banner printed');
const pairs = [];
for (const s of out.split('\n')) {
    const m = s.trim().match(/^dac=(\d+)\s+adc=(\d+)$/);
    if (m) pairs.push([parseInt(m[1], 10), parseInt(m[2], 10)]);
}
ok(pairs.length >= 8, `at least 8 dac/adc lines (got ${pairs.length})`);
ok(pairs[0][0] === 2048 && pairs[1][0] === 2831, 'sine table order (2048, 2831, ...)');
const adcs = pairs.map(p => p[1]);
ok(Math.min(...adcs) < 1000, `loopback reaches low end (min ${Math.min(...adcs)})`);
ok(Math.max(...adcs) > 3000, `loopback reaches high end (max ${Math.max(...adcs)})`);
// Loopback tracks: adc rises while dac rises over the first quarter wave.
let tracks = true;
for (let i = 1; i < Math.min(4, pairs.length); i++) {
    if (pairs[i][0] <= pairs[i - 1][0] || pairs[i][1] <= pairs[i - 1][1]) tracks = false;
}
ok(tracks, 'adc tracks rising dac over first quarter wave');

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
