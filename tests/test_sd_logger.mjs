// SD data-logger regression test: the arduino_sd_logger firmware brings
// up SDIO (CMD0/8/55/41/2/3/7/16), then every RTC second samples the ADC
// temperature sensor, writes {magic, rtc, adc} to an SD block (CMD24),
// reads it back (CMD17) and verifies. Progress prints as log lines.
//
// Guards the SDIO+ADC+RTC stack end to end against a real SDHC image.
import { readFileSync } from 'fs';
import { createEmulator } from '../pkg/emulator.js';

const ELF = 'site/arduino_sd_logger.elf';
const MAX = 70000000;
const CHUNK = 5000000;

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const img = new Uint8Array(2048 * 512); // 1 MiB SDHC, like the unit tests
const emu = await createEmulator({ firmware: readFileSync(ELF), ext_devices: { sd_card: [{ peripheral: 'SDIO', data: img }] } });
let done = 0;
while (done < MAX) {
    const r = await emu.run(Math.min(CHUNK, MAX - done));
    done += CHUNK;
    if (r.stopped) break;
}
const out = String(emu.getUartOutput() || '');
const lines = out.split('\n').map(s => s.trim()).filter(Boolean);
ok(out.includes('=== SD data logger'), 'banner printed');
ok(out.includes('sd ready'), 'SD init sequence completed');
const logs = lines.map(s => s.match(/^log (\d+) rtc=(\d+) adc=(\d+) (ok|MISMATCH)$/)).filter(Boolean);
ok(logs.length >= 4, `4 logged samples in 70M instr (got ${logs.length})`);
ok(logs.every((m, i) => +m[1] === i && m[4] === 'ok'), 'samples numbered 0..3, all verified ok');
const rtcs = logs.map(m => +m[2]);
ok(rtcs.every((v, i) => v === i), `RTC timestamps 0,1,2,3 (${rtcs.join(',')})`);
const adcs = logs.map(m => +m[3]);
ok(adcs.every((v, i) => i === 0 || v >= adcs[i - 1]), `ADC temp converges upward (${adcs.join(',')})`);
ok(adcs[3] > 1500 && adcs[3] < 2000, `ADC near nominal 0x6EE (${adcs[3]})`);
ok(out.includes('logger done'), 'logger finished');

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
