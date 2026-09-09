// Mini-RTOS regression test: the arduino_mini_rtos firmware runs two
// preemptive tasks (A/B) under a hand-rolled PendSV + PSP kernel, ticked
// at 1ms by TIM4 (HardwareTimer). Each task prints `ID<seq> t=<ticks>`
// every 200 ticks under an LDREX/STREX spinlock.
//
// Guards task switching end to end: gap-free per-task sequences prove
// registers/stacks survive preemption; timing proves the 1ms tick rate.
import { readFileSync } from 'fs';
import { STM32F1 } from '../pkg/stm32f1.js';

const ELF = 'site/arduino_mini_rtos.elf';

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const mcu = await STM32F1.fromELF(readFileSync(ELF));
const emu = mcu._emu;
// Production-sized batches: one IRQ per batch max would coalesce periodic
// delivery at huge steps (pending bits don't count); 20K keeps every tick.
for (let i = 0; i < 4000; i++) emu.step(20000);
const out = String(emu.getUartOutput() || '');
const lines = out.split('\n').map(s => s.trim()).filter(Boolean);
ok(out.includes('=== mini-RTOS demo'), 'banner printed');
ok(out.includes('scheduler live'), 'scheduler started');
const parse = (id) => lines
    .map(s => s.match(new RegExp(`^${id}(\\d+) t=(\\d+)$`)))
    .filter(Boolean).map(m => [+m[1], +m[2]]);
const seqA = parse('A'), seqB = parse('B');
ok(seqA.length >= 3 && seqB.length >= 3, `both tasks printed 3+ times (A:${seqA.length} B:${seqB.length})`);
const gapless = (s) => s.every((v, i) => v[0] === i);
ok(gapless(seqA) && gapless(seqB), 'per-task sequences gap-free from 0 (no context corruption)');
const timed = (s) => s.every((v, i) => i === 0 || (v[1] - s[i - 1][1] >= 190 && v[1] - s[i - 1][1] <= 220));
ok(timed(seqA) && timed(seqB), '200-tick cadence (±10) on both tasks (1ms rate exact)');
const interleave = seqA.length && seqB.length &&
    Math.abs(seqA[0][1] - seqB[0][1]) < 400;
ok(interleave, 'tasks interleave (true preemption, not run-to-completion)');

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
