// Deterministic EXTI edge-event test (the Wokwi-style queue, type 7).
//
// The headless periph_test does not generate EXTI edges on its own (its EXTI
// tests wait for external button stimulus), so we configure EXTI0/EXTI1 for a
// rising edge directly via the peripheral bus and then drive PA0/PA1 high via
// gpioSetInput — which must produce ExtiEdge{0} / ExtiEdge{1} events.
import { readFileSync } from 'fs';
import { STM32F1 } from '../pkg/stm32f1.js';

const ELF = 'site/arduino_ws2812.elf'; // any firmware; we only need an initialized emulator

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const mcu = await STM32F1.fromELF(readFileSync(ELF));

let exti = [];
mcu.onExtiEdge = (line) => exti.push(line);

// EXTI base 0x40010400: IMR (0x00) unmask lines 0/1, RTSR (0x08) rising edge.
mcu._emu.periphWrite(0x40010400, 4, (1 << 0) | (1 << 1));
mcu._emu.periphWrite(0x40010408, 4, (1 << 0) | (1 << 1));
mcu.execute(1000); // let the config settle (drains any stray events)

mcu.gpio.pin('A', 0).setInput(true); // rising edge on line 0
mcu.gpio.pin('A', 1).setInput(true); // rising edge on line 1
mcu.execute(1000); // drain the queued edge events

ok(exti.includes(0), `EXTI0 edge fired (got ${JSON.stringify(exti)})`);
ok(exti.includes(1), `EXTI1 edge fired (got ${JSON.stringify(exti)})`);

console.log(`exti events: ${JSON.stringify(exti)}`);
console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed === 0 ? 0 : 1);
