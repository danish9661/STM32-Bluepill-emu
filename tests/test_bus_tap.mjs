// Bus-tap smoke: onPeriphWrite is fed from the in-model write tap
// (peripheral writes never cross JS). Run showcase (CPU-driven SPI, 7-seg
// latch) + ws2812 (DMA-driven SPI strip) and assert the DECODED content —
// exactly what the page 7-seg/strip decoders eat.
import { readFileSync } from 'fs';
import { createEmulator } from '../pkg/emulator.js';

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const GPIOA_ODR = 0x4001080C, GPIOA_BSRR = 0x40010810, GPIOA_BRR = 0x40010814;
const SPI1_DR = 0x4001300C;
const PA4 = 1 << 4;

function expandBytes(size, value) {
    const out = [];
    for (let i = 0; i < size; i++) out.push((value >>> (i * 8)) & 0xFF);
    return out;
}

async function runTap({ firmware, ext_devices, instr, until }) {
    const emu = await createEmulator({ firmware, ext_devices });
    const pa4low = { level: false };
    const segLatches = [];
    let segBuf = [];
    const spiBytes = [];
    emu.onPeriphWrite((addr, size, value) => {
        if (addr === GPIOA_ODR) pa4low.level = (value & PA4) === 0;
        else if (addr === GPIOA_BSRR) { if (value & PA4) pa4low.level = false; if (value & (PA4 << 16)) pa4low.level = true; }
        else if (addr === GPIOA_BRR) { if (value & PA4) pa4low.level = true; }
        else if (addr === SPI1_DR) {
            const bytes = expandBytes(size, value);
            for (const b of bytes) {
                spiBytes.push(b);
                if (pa4low.level) {
                    segBuf.push(b);
                    if (segBuf.length === 4) { segLatches.push(segBuf); segBuf = []; }
                }
            }
        }
    });
    const CHUNK = 10000000;
    let done = 0, out = '';
    while (done < instr) {
        const n = Math.min(CHUNK, instr - done);
        await emu.run(n);
        done += n;
        out += String(emu.getUartOutput() || '');
        if (until && until({ segLatches, spiBytes, out })) break;
    }
    out += String(emu.getUartOutput() || '');
    emu.close();
    return { segLatches, spiBytes, out, done };
}

// Showcase 7-seg: PA4-CS latched 4-byte digits decode (page-decoder shape).
{
    const firmware = readFileSync('site/arduino_hw_showcase.elf');
    const ext_devices = {
        i2c_oled: [{ peripheral: 'I2C1', address: 0x3C }],
        lcd: [{ peripheral: 'SPI1', cs: 'PA8' }],
    };
    const { segLatches, out, done } = await runTap({ firmware, ext_devices, instr: 300000000,
        until: ({ segLatches: l }) => l.length >= 2 && new Set(l.map(d => d[3])).size > 1 });
    ok(segLatches.length > 0, `showcase latched 7-seg digits (${segLatches.length} latches)`);
    ok(out.includes('BTN=armed'), 'showcase armed');
    // Seconds counter advances across 1Hz latches (last digit changes).
    ok(new Set(segLatches.map(d => d[3])).size > 1, `7-seg counter advances (${JSON.stringify(segLatches[0])})`);
}

// WS2812: raw SPI1 DR byte stream (fire-and-forget, no CS) decodes to
// full 72B GRB frames.
{
    const firmware = readFileSync('site/arduino_ws2812.elf');
    const { spiBytes } = await runTap({ firmware, ext_devices: {}, instr: 40000000 });
    ok(spiBytes.length > 70, `ws2812 saw strip bytes (${spiBytes.length})`);
    const frames = [];
    for (let f = 0; f + 72 <= spiBytes.length; f += 72) {
        const leds = [];
        for (let i = 0; i < 8; i++) {
            leds.push([spiBytes[f + i * 3], spiBytes[f + i * 3 + 1], spiBytes[f + i * 3 + 2]]);
        }
        frames.push(leds);
    }
    ok(frames.length > 0, `ws2812 decoded ${frames.length} complete frames`);
}

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed === 0 ? 0 : 1);
