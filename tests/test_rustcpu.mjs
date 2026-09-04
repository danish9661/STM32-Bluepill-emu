// Rust-CPU backend (Path B) product-path test: same createEmulator + run()
// shape as test_emulator_js.mjs, but with cpu:'rust' (no Unicorn instance,
// hooks, or patches). Runs the 24-peripheral firmware to 39/39 like the CLI.
import { readFileSync } from 'fs';
import { createEmulator, parseElf } from '../pkg/emulator.js';

const BUILD = 'tests/arduino_periph_test/build';
const read = (f) => readFileSync(`${BUILD}/${f}`);

const MAX = 200000000;
const CHUNK = 1000000;
const CAN_RAM_FLAG = parseElf(read('arduino_periph_test.ino.elf'))
    .symbols.find((s) => s.name === 'canRxArmed')?.addr
    ?? (() => { throw new Error('canRxArmed symbol missing from ELF'); })();

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const emu = await createEmulator({
    cpu: 'rust',
    firmware: read('arduino_periph_test.ino.elf'),
    ext_devices: {
        i2c_eeprom: [
            { peripheral: 'I2C1', address: 0x50, data: read('eeprom.bin') },
            { peripheral: 'I2C2', address: 0x51, data: read('eeprom2.bin') },
        ],
        i2c_oled: [{ peripheral: 'I2C1', address: 0x3C }],
        spi_flash: [
            { peripheral: 'SPI1', jedec_id: 0xEF4016, data: read('spi_flash.bin'), cs: 'PA4' },
            { peripheral: 'SPI2', jedec_id: 0xEF4017, data: read('spi_flash2.bin'), cs: 'PB12' },
        ],
        lcd: [{ peripheral: 'SPI1', cs: 'PA1' }],
        touchscreen: [{ peripheral: 'SPI1', cs: 'PA2', touch_detected_pin: 'PA3' }],
    },
});

for (const b of [0x41, 0x42]) emu.uartRx(b);

const t0 = Date.now();
let canInjected = false, done = 0;
while (done < MAX) {
    const n = Math.min(CHUNK, MAX - done);
    const r = await emu.run(n);
    done += n;
    if (!canInjected && emu.memRead32(CAN_RAM_FLAG) !== 0) {
        canInjected = emu.canInjectMessage(0x40006400, 0 << 21, 2, 0xDEAD, 0);
    }
    if (r.stopped) break;
}
const elapsed = ((Date.now() - t0) / 1000).toFixed(2);

const out = String(emu.getUartOutput() || '');
const summary = out.split('\n').filter((s) => s.includes('SUMMARY')).join(' ').trim();
const fails = out.split('\n').filter((s) => s.includes('FAIL'));

ok(canInjected, `CAN frame injected (${canInjected})`);
ok(!fails.length, `no FAIL lines (${fails.length})`);
ok(/pass=39 fail=0/.test(summary), `SUMMARY 39/39 (${summary || '(missing)'})`);
console.log(`emulator.js rust path: ${done} instructions in ${elapsed}s`);

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed === 0 ? 0 : 1);
