// Deterministic TIM input-capture test (the Wokwi-style queue, type 16).
//
// Configure TIM2 CH1 as input capture (TI1, rising edge) on PA0 (TIM2_CH1),
// then drive PA0 high via gpioSetInput -> a rising edge must latch CNT into
// CCR1 and emit a TimCapture{tim=2, ch=0, value} event.
import { readFileSync } from 'fs';
import { STM32F1 } from '../pkg/stm32f1.js';

const ELF = 'site/arduino_ws2812.elf'; // PA0/PA1 are free (see EXTI test)

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const mcu = await STM32F1.fromELF(readFileSync(ELF));

let cap = [];
mcu.onTimCapture = (tim, ch, value) => cap.push({ tim, ch, value });

const TIM2 = 0x4000_0000;
// Enable TIM2 clock (RCC APB1ENR bit 0) and configure CH1 input capture.
mcu._emu.periphWrite(0x4002_101C, 4, 1);            // RCC APB1ENR TIM2EN
mcu._emu.periphWrite(TIM2 + 0x18, 4, 0x01);         // CCMR1: CC1S=01 (TI1)
mcu._emu.periphWrite(TIM2 + 0x20, 4, 0x01);         // CCER: CC1E, rising edge
mcu._emu.periphWrite(TIM2 + 0x2C, 4, 0xFFFF);       // ARR (avoid 0xFFFF_FFFF wrap)
mcu._emu.periphWrite(TIM2 + 0x00, 4, 0x01);         // CR1: CEN

// PA0 low first; run a batch so the capture sampler initializes its baseline.
mcu.gpio.pin('A', 0).setInput(false);
mcu.execute(2000);

// Rising edge on PA0 -> capture.
mcu.gpio.pin('A', 0).setInput(true);
mcu.execute(2000); // drain

ok(cap.length > 0, `TimCapture fired (${cap.length})`);
if (cap.length > 0) {
    ok(cap[0].tim === 2, `tim == 2 (got ${cap[0].tim})`);
    ok(cap[0].ch === 0, `ch == 0 (got ${cap[0].ch})`);
    ok(cap[0].value > 0, `captured CNT > 0 (got ${cap[0].value})`);
    console.log(`TimCapture: tim=${cap[0].tim} ch=${cap[0].ch} value=${cap[0].value}`);
}

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed === 0 ? 0 : 1);
