// Per-board peripheral showcase test: the showcase sketch (OLED + LCD +
// 7-seg + RGB + buzzer + button, board banner + LED_BUILTIN per target)
// boots on its matching chip variant and prints the setup line plus
// per-second t= status lines. No ext devices needed for the UART asserts.
import { readFileSync } from 'fs';
import { createEmulator } from '../pkg/emulator.js';

const CASES = [
    ['site/arduino_board_showcase_pill.elf', 'stm32f103c8', 'Blue Pill'],
    ['site/arduino_board_showcase_maple.elf', 'maple_mini', 'Maple Mini'],
    ['site/arduino_board_showcase_nucleo.elf', 'nucleo_f103rb', 'Nucleo-F103RB'],
    ['site/arduino_board_showcase_rc.elf', 'stm32f103rc', 'Generic F103RC'],
];

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

for (const [elf, chip, name] of CASES) {
    const emu = await createEmulator({ firmware: readFileSync(elf), chip });
    for (let i = 0; i < 20; i++) await emu.run(10000000);
    const out = String(emu.getUartOutput() || '');
    ok(out.includes(`Peripheral showcase (${name})`), `${name} banner on ${chip}`);
    ok(out.includes('OLED=ok'), `${name} setup line on ${chip}`);
    ok(/t=\d+s btn=\d+/.test(out), `${name} per-second status on ${chip}`);
}

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
