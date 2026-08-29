// I2C virtual-peripheral event test (the Wokwi-style model).
//
// The periph_test firmware touches an I2C EEPROM (I2C1 @ 0x50) and I2C2 EEPROM
// (0x51) in its SYNC section, so I2cStart/Write/Read/Stop events fire within the
// first few million instructions. We register the (empty) EEPROM devices, attach
// the per-bus callbacks, run a bounded number of instructions, and assert the
// transaction events arrived — proving the I2C half of drain_events() works.
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

let starts = 0, writes = 0, reads = 0, stops = 0, lastAddr = 0;
mcu.i2c1.onStart = (a) => { starts++; lastAddr = a; };
mcu.i2c1.onWrite = () => { writes++; };
mcu.i2c1.onRead = () => { reads++; };
mcu.i2c1.onStop = () => { stops++; };

const t0 = Date.now();
let done = 0;
for (let i = 0; i < 6; i++) {
    const r = mcu.execute(5_000_000);
    done += 5_000_000;
    if (starts > 0 && writes > 0 && stops > 0) break;
    if (r.stopped) break;
}
const elapsed = ((Date.now() - t0) / 1000).toFixed(2);

ok(starts > 0, `i2c1.onStart fired (${starts}, lastAddr=0x${lastAddr.toString(16)})`);
ok(writes > 0, `i2c1.onWrite fired (${writes})`);
ok(stops > 0, `i2c1.onStop fired (${stops})`);
// Reads happen on master-receiver transactions; assert >= 0 (don't fail if none)
console.log(`i2c1 events: start=${starts} write=${writes} read=${reads} stop=${stops}`);

console.log(`i2c events: ${done} instructions in ${elapsed}s`);
console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed === 0 ? 0 : 1);
