// High-density desk test: the arduino_hd_fsmc firmware (FSMC NOR + dual
// DAC loopback, peripherals the C8 lacks) boots on stm32f103rc and verifies
// an FSMC write/read round-trip plus live DAC readings.
import { readFileSync } from 'fs';
import { createEmulator } from '../pkg/emulator.js';

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const emu = await createEmulator({
    firmware: readFileSync('site/arduino_hd_fsmc.elf'),
    chip: 'stm32f103rc',
    ext_devices: { fsmc_bank: [{ name: 'FSMC.BANK1', data: new Uint8Array(65536) }] },
});
for (let i = 0; i < 120; i++) await emu.run(1000000);
const out = String(emu.getUartOutput() || '');
ok(out.includes('HD desk (F103RC)'), 'HD banner on stm32f103rc');
const lines = out.split('\n').map(s => s.trim()).filter(s => s.startsWith('hd '));
ok(lines.length >= 1, `at least 1 hd line (got ${lines.length})`);
ok(lines.every(s => s.includes('fsmc=ok')), 'all FSMC round-trips ok');
ok(lines.some(s => /dac1=[0-9A-F]+/.test(s)), 'DAC1 loopback readings present');
ok(lines.some(s => /dac2=[0-9A-F]+/.test(s)), 'DAC2 loopback readings present');

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
