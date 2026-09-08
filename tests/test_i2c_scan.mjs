// I2C scanner regression test: the arduino_i2c_scan firmware probes every
// 7-bit address over Wire; with an SSD1306 (0x3C) + 24Cxx EEPROM (0x50) on
// I2C1 it must report exactly those two.
//
// Guards I2C address-phase ACK/NACK end-to-end through the Arduino stack.
import { readFileSync } from 'fs';
import { createEmulator } from '../pkg/emulator.js';

const ELF = 'site/arduino_i2c_scan.elf';
const MAX = 200000000;
const CHUNK = 10000000;

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const emu = await createEmulator({
    firmware: readFileSync(ELF),
    ext_devices: {
        i2c_eeprom: [{ peripheral: 'I2C1', address: 0x50, data: readFileSync('site/arduino_eeprom.bin') }],
        i2c_oled: [{ peripheral: 'I2C1', address: 0x3C }],
    },
});
let done = 0;
while (done < MAX) {
    const r = await emu.run(Math.min(CHUNK, MAX - done));
    done += CHUNK;
    if (r.stopped) break;
}
const out = String(emu.getUartOutput() || '');
ok(out.includes('=== I2C scanner demo'), 'banner printed');
ok(/found 0x3[Cc]/.test(out), 'OLED at 0x3C found');
ok(/found 0x50/.test(out), 'EEPROM at 0x50 found');
ok(out.includes('done, found=2'), 'exactly 2 devices reported');

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
