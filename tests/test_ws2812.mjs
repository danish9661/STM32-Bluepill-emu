// WS2812 regression test: drives the same createEmulator + run() path the page
// uses, with the arduino_ws2812 firmware (8-LED strip over SPI1 + DMA1 CH3).
//
// Guards the emulator's SPI1+DMA->WS2812 path + the 3-SPI-bits-per-WS2812-bit
// GRB decode. The old manual "Node smoke" checked this by hand and was never
// committed as a test (and the render-throttle regression showed how easily
// browser/feature validations get lost) — this locks it in.
//
// Note: the firmware prints on Serial1 = USART2, but getUartOutput() only
// captures USART1, so we validate via the onPeriphWrite decoder (exactly how
// the page validates — by decoding SPI1 DR writes into LED colors), not UART.
import { readFileSync } from 'fs';
import { createEmulator } from '../pkg/emulator.js';

const ELF = 'site/arduino_ws2812.elf';
const MAX = 100000000;        // ~5s of emulated time is plenty for many frames
const CHUNK = 5000000;

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const emu = await createEmulator({ firmware: readFileSync(ELF) });

// Replicate site/index.html wsWatch: decode SPI1 DR writes (0x4001300C) into
// 8 GRB LEDs, 72 SPI bytes/frame, 3 SPI bits per WS2812 bit.
const WS_SPI_DR = 0x4001300C;
const wsBuf = [];
const wsLeds = Array.from({ length: 8 }, () => ({ r: 0, g: 0, b: 0 }));
let wsFrames = 0;
let firstFrameLed0 = null;
const wsWatch = (addr, width, value) => {
    if (addr !== WS_SPI_DR) return;
    wsBuf.push(value & 0xFF);
    if (wsBuf.length < 72) return;
    const spi = wsBuf.splice(0, 72);
    const ledBytes = [];
    for (let d = 0; d < 24; d++) {
        const bits = (spi[d * 3] << 16) | (spi[d * 3 + 1] << 8) | spi[d * 3 + 2];
        let out = 0;
        for (let i = 23; i >= 2; i -= 3) out = (out << 1) | (((bits >> (i - 2)) & 0b111) === 0b110 ? 1 : 0);
        ledBytes.push(out);
    }
    for (let l = 0; l < 8; l++) {
        wsLeds[l].g = ledBytes[l * 3];
        wsLeds[l].r = ledBytes[l * 3 + 1];
        wsLeds[l].b = ledBytes[l * 3 + 2];
    }
    wsFrames++;
    if (wsFrames === 1) firstFrameLed0 = { ...wsLeds[0] };
};
emu.onPeriphWrite(wsWatch);

const t0 = Date.now();
let done = 0;
while (done < MAX) {
    const n = Math.min(CHUNK, MAX - done);
    const r = await emu.run(n);
    done += n;
    if (wsFrames >= 10) break;       // enough frames to prove sustained streaming
    if (r.stopped) break;
}
const elapsed = ((Date.now() - t0) / 1000).toFixed(2);

ok(wsFrames >= 5, `decoder streamed multiple frames (wsFrames=${wsFrames})`);
ok(
    firstFrameLed0 && firstFrameLed0.r > 200 && firstFrameLed0.g < 60 && firstFrameLed0.b < 60,
    `frame0 LED0 is red — GRB order + bit-mapping intact (got ${JSON.stringify(firstFrameLed0)})`,
);

console.log(`ws2812: ${done} instructions in ${elapsed}s, wsFrames=${wsFrames}`);
console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed === 0 ? 0 : 1);
