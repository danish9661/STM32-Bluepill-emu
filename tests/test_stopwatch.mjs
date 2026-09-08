// Stopwatch regression test: the arduino_stopwatch firmware toggles run/stop
// on PB13 falling edges (EXTI13) and prints elapsed TIM2 time.
//
// Guards EXTI13 interrupt delivery from a JS-driven pin + TIM2 free-run +
// UART prints. NOTE: drive PB13 high once first — the pin-driver state
// starts low, so the first drive-low would make no edge (the pullup idles
// high only once firmware configures it, which doesn't move driver state).
import { readFileSync } from 'fs';
import { createEmulator } from '../pkg/emulator.js';

const ELF = 'site/arduino_stopwatch.elf';
const CHUNK = 5000000;

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const emu = await createEmulator({ firmware: readFileSync(ELF) });
const step = async (n) => {
    let done = 0;
    while (done < n) {
        const r = await emu.run(Math.min(CHUNK, n - done));
        done += CHUNK;
        if (r.stopped) break;
    }
};
// Button press = falling edge (idle high first so the edge exists).
const press = async () => {
    emu.gpioSetInput(1, 13, true);
    await step(200000);
    emu.gpioSetInput(1, 13, false);
    await step(2000000);
    emu.gpioSetInput(1, 13, true);
};
const uart = () => String(emu.getUartOutput() || '');

await step(20000000);
ok(uart().includes('=== Stopwatch demo'), 'banner printed');
await press();                    // start
await step(150000000);
await press();                    // stop
await step(30000000);
const out = uart();
ok(out.includes('run'), 'first press starts (run printed)');
ok(/t=\d+\.\d+s/.test(out), 'elapsed prints while running');
const stop = out.match(/stop t=(\d+)\.(\d+)s/);
ok(stop !== null, 'second press stops with elapsed time');
if (stop) {
    const secs = parseInt(stop[1], 10);
    ok(secs >= 1 && secs <= 5, `elapsed plausible (~2s, got ${secs}s)`);
}

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
