// stm32f1.js wrapper test: validates the ergonomic API over emulator.js,
// including the Wokwi-style virtual-peripheral event queue (USART/SPI/I2C
// transaction events drained from the core once per batch).
//
// Key checks:
//  - gpio.pin().read() returns a 0/1 level
//  - usart1.onData captures USART1 TX ("WS2812=ok") via the core event queue
//    (the firmware prints on Serial1 = USART1)
//  - spi1.onTransfer fires for the SPI1 DMA-to-DR transfers that drive the strip
import { readFileSync } from 'fs';
import { STM32F1 } from '../pkg/stm32f1.js';

const ELF = 'site/arduino_ws2812.elf';
const MAX = 100000000;
const CHUNK = 5000000;

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const mcu = await STM32F1.fromELF(readFileSync(ELF));

// Ergonomic surface present
ok(mcu.gpio && typeof mcu.gpio.pin === 'function', 'gpio.pin() exists');
ok(mcu.usart1 && mcu.usart2 && mcu.usart3, 'usart1/2/3 exist');
ok(mcu.spi1 && mcu.spi2 && mcu.i2c1, 'spi1/2 and i2c1 exist');
const pa5 = mcu.gpio.pin('A', 5);
ok(typeof pa5.read() === 'number', 'gpio.pin().read() returns a level');

// Per-pin change subscription returns an unsubscribe fn
const unsub = pa5.on('change', () => {});
ok(typeof unsub === 'function', 'gpio.pin().on("change") returns unsubscribe');
unsub();

// Capture USART1 TX (firmware prints "WS2812=ok" on Serial1 = USART1)
let tx = [];
mcu.usart1.onData = (b) => tx.push(b);

// Capture SPI1 transactions (firmware drives the WS2812 strip over SPI1)
let spiTx = [], spiCount = 0;
mcu.spi1.onTransfer = (ch, t, r) => { spiCount++; spiTx.push(...t); };

const t0 = Date.now();
let done = 0, captured = false;
while (done < MAX) {
    const n = Math.min(CHUNK, MAX - done);
    const r = await mcu.execute(n);
    done += n;
    const s = new TextDecoder().decode(Uint8Array.from(tx));
    if (s.includes('WS2812=ok')) { captured = true; break; }
    if (r.stopped) break;
}
const elapsed = ((Date.now() - t0) / 1000).toFixed(2);
const txStr = new TextDecoder().decode(Uint8Array.from(tx));

ok(captured, `usart1.onData captured "WS2812=ok" (got: ${JSON.stringify(txStr.slice(0, 60))})`);
ok(spiCount > 0, `spi1.onTransfer fired (${spiCount} transfers, ${spiTx.length} tx bytes)`);

// USB IN completion -> onUsbIn dispatch (registers driven directly; the
// ws2812 firmware never touches USB so manual arming is undisturbed)
let usbIn = null;
mcu.onUsbIn = (ep, data) => { usbIn = [ep, data]; };
const U = 0x40005C00, PMA = 0x40006000;
mcu._emu.periphWrite(U + 0x40, 4, 0x8000); // CNTR: CTRM
mcu._emu.periphWrite(U + 0x00, 4, 0x3200); // EP0 control + RX VALID
mcu._emu.periphWrite(U + 0x50, 4, 0);      // BTABLE = 0
mcu._emu.periphWrite(PMA + 0, 2, 0x40);    // ADDR0_TX
mcu._emu.periphWrite(PMA + 2, 2, 4);       // COUNT0_TX = 4
mcu._emu.periphWrite(PMA + 4, 2, 0x80);    // ADDR0_RX
ok(mcu._emu.usbInjectSetup([1, 2, 3, 4, 5, 6, 7, 8]) === true, 'usbInjectSetup accepted');
mcu._emu.periphWrite(U + 0x00, 4, 0x3200); // clear CTR_RX
for (let i = 0; i < 4; i++) mcu._emu.periphWrite(PMA + 0x40 + i, 1, 0xA0 + i);
mcu._emu.periphWrite(PMA + 2, 2, 4);
mcu._emu.periphWrite(U + 0x00, 4, 0x0030); // STAT_TX DISABLED -> VALID: IN fires
mcu.step(5000);
ok(usbIn && usbIn[0] === 0 && usbIn[1].join(',') === '160,161,162,163',
    'onUsbIn dispatched with PMA buffer bytes');

console.log(`stm32f1 api: ${done} instructions in ${elapsed}s, usart1 bytes=${tx.length}, spi1 transfers=${spiCount}`);
console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed === 0 ? 0 : 1);
