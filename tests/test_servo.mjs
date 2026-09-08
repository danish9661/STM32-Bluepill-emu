// Servo sweep regression test: the arduino_servo firmware drives a 50Hz
// TIM3 CH1 (PA6) PWM from 0 to 180 degrees and back, printing deg + pulse.
//
// Guards the TIM3 PWM path + UART prints. No ext devices needed.
import { readFileSync } from 'fs';
import { createEmulator } from '../pkg/emulator.js';

const ELF = 'site/arduino_servo.elf';
const MAX = 100000000;
const CHUNK = 5000000;

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
const lines = out.split('\n').map(s => s.trim()).filter(Boolean);
ok(out.includes('=== Servo sweep demo'), 'banner printed');
const steps = lines.filter(s => /^deg=\d+\s+pulse=\d+$/.test(s));
ok(steps.length >= 5, `at least 5 sweep steps (got ${steps.length})`);
const degs = steps.map(s => parseInt(s.split(' ')[0].split('=')[1], 10));
ok(degs[0] === 0, `sweep starts at 0 (got ${degs[0]})`);
let rising = true;
for (let i = 1; i < degs.length; i++) {
    if (degs[i] < degs[i - 1]) { rising = false; break; }
}
ok(rising || degs.includes(180), 'angles rise monotonically (or reached 180)');
const pulses = steps.map(s => parseInt(s.split('pulse=')[1], 10));
ok(pulses.every(p => p >= 1000 && p <= 2000), 'pulse widths in 1000..2000');
ok(lines.some(s => s === 'turn deg=0'), 'turn marker printed');

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
