// Per-board RTC clock test: the rtc sketch (board banner + HH:MM:SS on the
// board's native Serial port) boots on its matching chip variant and ticks
// from 12:00:00 monotonically. Mirrors test_rtc_clock.mjs per target.
import { readFileSync } from 'fs';
import { createEmulator } from '../pkg/emulator.js';

const CASES = [
    ['site/arduino_board_rtc_pill.elf', 'stm32f103c8', 'Blue Pill'],
    ['site/arduino_board_rtc_maple.elf', 'maple_mini', 'Maple Mini'],
    ['site/arduino_board_rtc_nucleo.elf', 'nucleo_f103rb', 'Nucleo-F103RB'],
    ['site/arduino_board_rtc_rc.elf', 'stm32f103rc', 'Generic F103RC'],
];

const MAX = 50000000;
const CHUNK = 5000000;

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

for (const [elf, chip, name] of CASES) {
    const emu = await createEmulator({ firmware: readFileSync(elf), chip });
    let done = 0;
    while (done < MAX) {
        const r = await emu.run(Math.min(CHUNK, MAX - done));
        done += CHUNK;
        if (r.stopped) break;
    }
    const out = String(emu.getUartOutput() || '');
    const lines = out.split('\n').map(s => s.trim()).filter(Boolean);
    ok(out.includes('=== RTC clock demo ==='), `${name} banner on ${chip}`);
    ok(out.includes(`board: ${name}`), `${name} board line on ${chip}`);
    const clocks = lines.filter(s => /^\d\d:\d\d:\d\d\s+rtc=\d+$/.test(s));
    ok(clocks.length >= 3, `${name} >=3 clock lines (got ${clocks.length})`);
    ok(clocks.length > 0 && clocks[0].startsWith('12:00:00'), `${name} starts 12:00:00`);
    let mono = true;
    for (let i = 1; i < clocks.length; i++) {
        const a = parseInt(clocks[i - 1].split('rtc=')[1], 10);
        const b = parseInt(clocks[i].split('rtc=')[1], 10);
        if (!(b > a)) mono = false;
    }
    ok(mono, `${name} clock monotonic`);
}

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
