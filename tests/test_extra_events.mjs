// Extended virtual-peripheral event test: EXTI edge, ADC conversion-done, and
// TIM update events (the Wokwi-style queue, types 7/8/9).
//
// periph_test exercises TIM2/TIM3/TIM4 (PWM/update), ADC conversions, and
// EXTI0/1/13 in its sections, so running it a bounded number of instructions
// must emit TimUpdate + AdcDone (and ExtiEdge) events.
import { readFileSync } from 'fs';
import { STM32F1 } from '../pkg/stm32f1.js';

const ELF = 'site/arduino_periph_test.elf';

const ext_devices = {
    i2c_eeprom: [
        { peripheral: 'I2C1', address: '0x50', data: new Uint8Array(256) },
        { peripheral: 'I2C2', address: '0x51', data: new Uint8Array(256) },
    ],
    i2c_oled: [{ peripheral: 'I2C1', address: '0x3C', width: 128, height: 64 }],
    spi_flash: [
        { peripheral: 'SPI1', jedec_id: '0xEF4016', data: new Uint8Array(4096), cs: 'PA4' },
        { peripheral: 'SPI2', jedec_id: '0xEF4017', data: new Uint8Array(4096), cs: 'PB12' },
    ],
    lcd: [{ peripheral: 'SPI1', cs: 'PA1' }],
    touchscreen: [{ peripheral: 'SPI1', touch_detected_pin: 'PA3', cs: 'PA2' }],
};

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const mcu = await STM32F1.fromELF(readFileSync(ELF), { ext_devices });

let timUpdates = 0, adcDone = 0, extiEdges = 0;
const timSet = new Set(), adcSet = new Set(), extiSet = new Set();
mcu.onTimUpdate = (tim) => { timUpdates++; timSet.add(tim); };
mcu.onAdcDone = (adc, chan) => { adcDone++; adcSet.add(adc + ':' + chan); };
mcu.onExtiEdge = (line) => { extiEdges++; extiSet.add(line); };

const t0 = Date.now();
let done = 0;
for (let i = 0; i < 20; i++) {
    const r = mcu.execute(5_000_000);
    done += 5_000_000;
    if (timUpdates > 0 && adcDone > 0 && extiEdges > 0) break;
    if (r.stopped) break;
}
const elapsed = ((Date.now() - t0) / 1000).toFixed(2);

ok(timUpdates > 0, `TimUpdate events fired (${timUpdates}, timers=${[...timSet].sort((a,b)=>a-b).join(',')})`);
ok(adcDone > 0, `AdcDone events fired (${adcDone}, adc:chan=${[...adcSet].slice(0, 6).join(',')}...)`);
// EXTI edges require external stimulus, which periph_test does not provide headlessly;
// covered deterministically by test_exti_events.mjs. Log only here.
console.log(`EXTI edges seen in periph_test (informational): ${extiEdges}, lines=${[...extiSet].sort((a,b)=>a-b).join(',')}`);

console.log(`extra events: TIM=${timUpdates} ADC=${adcDone} EXTI=${extiEdges} in ${done} instructions (${elapsed}s)`);
console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed === 0 ? 0 : 1);
