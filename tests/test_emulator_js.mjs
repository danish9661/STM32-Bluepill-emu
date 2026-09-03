// Emulator.js (browser run-loop) path test: the SAME createEmulator + run()
// code the page executes, driven from Node. cli.mjs and emulator.js share the
// dispatch implementation now, but this test exists because they USED to drift
// (the xPSR-restore bug was cli-only) — the browser path must stay green.
// Runs the 24-peripheral firmware to completion and asserts 39/39 like the CLI.
import { readFileSync } from 'fs';
import { createEmulator, parseElf } from '../pkg/emulator.js';

const BUILD = 'tests/arduino_periph_test/build';
const read = (f) => readFileSync(`${BUILD}/${f}`);

const MAX = 200000000;           // full run (canary pace, ~30-60s)
const CHUNK = 1000000;            // 1M autopilot granularity: the CAN frame must
                                  // land within ~a batch of arming (like cli.mjs),
                                  // else the firmware burns an RF0R spin storm
                                  // waiting (10M chunks cost ~3s per 200M run)
// Resolve canRxArmed from the ELF symbols at runtime — a hardcoded address
// goes stale on every firmware rebuild (was 0x200000b8 = canRxTries after
// one rebuild: injection landed a full 3M-iteration RF0R timeout late).
const CAN_RAM_FLAG = parseElf(read('arduino_periph_test.ino.elf'))
    .symbols.find((s) => s.name === 'canRxArmed')?.addr
    ?? (() => { throw new Error('canRxArmed symbol missing from ELF'); })();

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const emu = await createEmulator({
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

// 'A' -> DMA RX test gate, 'B' -> UART RX byte (same as the CLI's stdin pump)
for (const b of [0x41, 0x42]) emu.uartRx(b);

// CAN autopilot (mirrors site/index.html:537): inject once the firmware arms
// the CAN RX test — filter bank 0 is ID-list mode, only STDID 0 matches.
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
console.log(`emulator.js path: ${done} instructions in ${elapsed}s`);

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed === 0 ? 0 : 1);
