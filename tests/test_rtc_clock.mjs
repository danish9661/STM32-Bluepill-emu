// RTC clock regression test: the arduino_rtc_clock firmware presets the RTC
// to 12:00:00 with PRL=1M and prints HH:MM:SS plus the raw CNT every tick.
//
// Guards the RTC counter path (PRL/unit?). No ext devices needed.
import { readFileSync } from 'fs';
import { createEmulator } from '../pkg/emulator.js';

const ELF = 'tests/arduino_rtc_clock/build/arduino_rtc_clock.ino.elf';
const MAX = 50000000;
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
ok(out.includes('=== RTC clock demo ==='), 'banner printed');
const clocks = lines.filter(s => /^\d\d:\d\d:\d\d\s+rtc=\d+$/.test(s));
ok(clocks.length >= 3, `at least 3 clock lines (got ${clocks.length})`);
ok(clocks[0].startsWith('12:00:00'), `starts at 12:00:00 (got ${clocks[0]})`);
let mono = true;
for (let i = 1; i < clocks.length; i++) {
    const a = parseInt(clocks[i - 1].split('rtc=')[1], 10);
    const b = parseInt(clocks[i].split('rtc=')[1], 10);
    if (!(b > a)) mono = false;
}
ok(mono, 'RTC counter increases monotonically');

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
