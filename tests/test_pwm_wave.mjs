// PWM wave regression test: the arduino_pwm_wave firmware stages one duty
// value per 100ms frame and re-arms DMA1 CH3 (mem->TIM3_DMAR, CNDTR=1);
// each TIM3 update event bursts it into CCR1 (DCR: DBA=CCR1, DBL=0).
// Duty follows an 8-step triangle; each step prints duty + CCR1 readback.
//
// Guards the TIM DMA-burst path (DCR/DMAR window sequencing) + DMA update
// requests + millis() rate end to end. No ext devices needed.
import { readFileSync } from 'fs';
import { createEmulator } from '../pkg/emulator.js';

const ELF = 'site/arduino_pwm_wave.elf';
const MAX = 70000000;
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
ok(out.includes('=== TIM DMA-burst PWM wave'), 'banner printed');
const steps = lines.filter(s => /^duty=\d+\s+ccr=\d+$/.test(s));
ok(steps.length >= 8, `full 8-step wave in 70M instr (got ${steps.length})`);
const WAVE = [0, 143, 286, 429, 571, 714, 857, 999];
const duties = steps.map(s => parseInt(s.split(' ')[0].split('=')[1], 10));
ok(WAVE.every((v, i) => duties[i] === v), `wave order exact (${duties.slice(0, 8).join(',')})`);
const ccrs = steps.map(s => parseInt(s.split('ccr=')[1], 10));
ok(duties.every((d, i) => ccrs[i] === d), 'CCR1 readback tracks staged duty (burst landed)');
ok(ccrs.every(c => c >= 0 && c <= 999), 'duties within ARR=999');
ok(duties[0] === 0 && duties[7] === 999, 'wave spans 0..999');

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
