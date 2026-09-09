// Per-board demo firmware test: the same sketch compiled for four board
// targets (Blue Pill / Maple Mini / Nucleo-F103RB / Generic F103RC) boots
// on its matching chip variant, prints its board banner (proving the right
// binary), blinks LED_BUILTIN and ticks. No UART input needed.
import { readFileSync } from 'fs';
import { createEmulator } from '../pkg/emulator.js';

const CASES = [
    ['site/arduino_board_pill.elf', 'stm32f103c8', 'Blue Pill'],
    ['site/arduino_board_maple.elf', 'maple_mini', 'Maple Mini'],
    ['site/arduino_board_nucleo.elf', 'nucleo_f103rb', 'Nucleo-F103RB'],
    ['site/arduino_board_rc.elf', 'stm32f103rc', 'Generic F103RC'],
];

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

for (const [elf, chip, name] of CASES) {
    const emu = await createEmulator({ firmware: readFileSync(elf), chip });
    for (let i = 0; i < 40; i++) await emu.run(1000000);
    const out = String(emu.getUartOutput() || '');
    ok(out.includes(`board demo: ${name}`), `${name} banner on ${chip}`);
    const ticks = [...out.matchAll(/tick (\d+)/g)].map((m) => +m[1]);
    ok(ticks.length >= 2 && ticks[0] === 0 && ticks[1] === 1, `${name} ticks 0,1,… on ${chip}`);
}

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
