// Per-board UART echo test: the board_echo sketch (banner + echo on the
// board's native Serial port) boots on its matching chip variant and
// echoes injected bytes. USART1 boards (pill/maple) use the default
// uart_addr; USART2 boards (nucleo/rc) pass uart_addr explicitly.
import { readFileSync } from 'fs';
import { createEmulator } from '../pkg/emulator.js';

const CASES = [
    ['site/arduino_board_echo_pill.elf', 'stm32f103c8', 'Blue Pill', 0x40013800],
    ['site/arduino_board_echo_maple.elf', 'maple_mini', 'Maple Mini', 0x40013800],
    ['site/arduino_board_echo_nucleo.elf', 'nucleo_f103rb', 'Nucleo-F103RB', 0x40004400],
    ['site/arduino_board_echo_rc.elf', 'stm32f103rc', 'Generic F103RC', 0x40004400],
];

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

for (const [elf, chip, name, uart] of CASES) {
    const emu = await createEmulator({ firmware: readFileSync(elf), chip });
    for (let i = 0; i < 20; i++) await emu.run(1000000);
    const boot = String(emu.getUartOutput() || '');
    ok(boot.includes(`board echo: ${name}`), `${name} banner on ${chip}`);
    emu.uartRxAddr(uart, 0x51); // 'Q'
    emu.uartRxAddr(uart, 0x52); // 'R'
    for (let i = 0; i < 20; i++) await emu.run(1000000);
    const echo = String(emu.getUartOutput() || '');
    ok(echo.includes('QR'), `${name} echoes QR on ${chip} (uart 0x${uart.toString(16)})`);
}

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
