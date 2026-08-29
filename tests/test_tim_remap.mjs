// TIM AFIO remap test (the Wokwi-style queue, type 16).
//
// Set AFIO MAPR TIM2_REMAP=01 (CH2 -> PB3 instead of the default PA1) and
// configure TIM2 CH2 as input capture. A rising edge on PA1 (the default CH2
// pin, now NOT watched) must NOT capture; a rising edge on PB3 (the remapped
// CH2 pin) MUST capture. This proves tim_chan_pin honors the AFIO remap.
import { readFileSync } from 'fs';
import { STM32F1 } from '../pkg/stm32f1.js';

const ELF = 'site/arduino_ws2812.elf'; // PA1/PB3 are free (see EXTI test)

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const mcu = await STM32F1.fromELF(readFileSync(ELF));

let cap = [];
mcu.onTimCapture = (tim, ch, value) => cap.push({ tim, ch, value });

const TIM2 = 0x4000_0000;
// AFIO MAPR: TIM2_REMAP = 01 (bits [9:8])
mcu._emu.periphWrite(0x4001_0004, 4, 1 << 8);
// TIM2 CH2 input capture: CCMR1 CC2S=01 (bits [11:8]), CCER CC2E (bit 4), rising, ARR, CEN
mcu._emu.periphWrite(TIM2 + 0x18, 4, 1 << 8);
mcu._emu.periphWrite(TIM2 + 0x20, 4, 1 << 4);
mcu._emu.periphWrite(TIM2 + 0x2C, 4, 0xFFFF);
mcu._emu.periphWrite(TIM2 + 0x00, 4, 0x01);

// Baseline: PA1 (default CH2 pin) and PB3 (remapped CH2 pin) both low.
mcu.gpio.pin('A', 1).setInput(false);
mcu.gpio.pin('B', 3).setInput(false);
mcu.execute(2000);

// Rising edge on PA1 (default CH2 pin, NOT watched under remap) -> no capture.
mcu.gpio.pin('A', 1).setInput(true);
mcu.execute(2000);

// Rising edge on PB3 (remapped CH2 pin) -> capture.
mcu.gpio.pin('B', 3).setInput(true);
mcu.execute(2000);

ok(cap.length === 1, `exactly one TimCapture from remapped pin (got ${cap.length})`);
if (cap.length > 0) {
    ok(cap[0].tim === 2 && cap[0].ch === 1, `TimCapture{2,1} (got {${cap[0].tim},${cap[0].ch}})`);
    ok(cap[0].value > 0, `captured CNT > 0 (got ${cap[0].value})`);
}

console.log(`tim remap capture: ${JSON.stringify(cap)}`);
console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed === 0 ? 0 : 1);
