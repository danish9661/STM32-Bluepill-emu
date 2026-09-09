// Chip-variant regression test: every builtin chip boots the same firmware
// and reports its DBGMCU IDCODE (STM32F103: 0x10016410, GD32F103: 0x2BA01477).
// GD32F103 is register-identical at everything modeled, so one UART echo
// run proves the whole path there too.
import { readFileSync } from 'fs';
import { createEmulator } from '../pkg/emulator.js';
const ELF = 'site/arduino_echo.elf';
const IDCODE = 0xE0042000;
const F103 = 0x10016410, GD32 = 0x2BA01477;

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

for (const [chip, id] of [
    ['stm32f103c8', F103], ['stm32f103cb', F103], ['stm32f103rc', F103],
    ['maple_mini', F103], ['nucleo_f103rb', F103],
    ['gd32f103c8', GD32], ['gd32f103cb', GD32], ['gd32f103rb', GD32],
]) {
    const emu = await createEmulator({ firmware: readFileSync(ELF), chip });
    ok((emu.periphRead(IDCODE, 4) >>> 0) === id,
        `${chip} IDCODE ${(emu.periphRead(IDCODE, 4) >>> 0).toString(16)}`);
}
// GD32 runs real firmware: UART echo round-trip.
{
    const emu = await createEmulator({ firmware: readFileSync(ELF), chip: 'gd32f103c8' });
    emu.uartRxBytes([72, 105]);
    for (let i = 0; i < 10; i++) await emu.run(1000000);
    ok(String(emu.getUartOutput() || '').includes('Hi'), 'GD32F103C8 UART echo round-trip');
}

// Board Arduino-pin aliases (site/board_pins.json, extracted from the
// STM32duino variant files): physical pin -> Arduino names.
{
    const pins = JSON.parse(readFileSync('site/board_pins.json', 'utf8'));
    ok(pins['nucleo_f103rb']['PA5'] === 'D13/A8', 'Nucleo D13 LED = PA5');
    ok(pins['nucleo_f103rb']['PA0'] === 'D46/A0', 'Nucleo A0 = PA0');
    ok(pins['maple_mini']['PB1'] === 'D33', 'Maple D33 LED = PB1');
    ok(pins['maple_mini']['PB8'] === 'D32', 'Maple button = PB8');
    ok(pins['stm32f103c8']['PC13'] === 'D17', 'BluePill LED = PC13 (D17)');
    ok(pins['stm32f103cb']['PC13'] === 'D17', 'F103CB shares the Pill map');
}

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
