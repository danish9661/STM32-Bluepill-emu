// Write-tap parity: the rust backend has no mem hooks, so onPeriphWrite is
// fed from the in-model tap. Run showcase (CPU-driven SPI, 7-seg latch) +
// ws2812 (DMA-driven SPI strip) on BOTH backends and assert the DECODED
// content matches — this is exactly what the page 7-seg/strip decoders eat.
// (Raw write streams differ by batch-phase slop: adaptive 20K/50K decisions
// shift per-batch TX progress, so counts can't match tick-for-tick.)
process.env.POLL_SHRINK = '0';
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

async function runTap({ cpu, firmware, ext_devices, instr }) {
    const emu = await createEmulator({ cpu, firmware, ext_devices });
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
    await emu.run(instr);
    const out = String(emu.getUartOutput() || '');
    emu.close();
    return { segLatches, spiBytes, out };
}

// Showcase 7-seg: PA4-CS latched 4-byte digits must match on both backends.
{
    const firmware = readFileSync('site/arduino_hw_showcase.elf');
    const ext_devices = {
        i2c_oled: [{ peripheral: 'I2C1', address: 0x3C }],
        lcd: [{ peripheral: 'SPI1', cs: 'PA8' }],
    };
    const u = await runTap({ cpu: 'unicorn', firmware, ext_devices, instr: 150000000 });
    const r = await runTap({ cpu: 'rust', firmware, ext_devices, instr: 150000000 });
    ok(u.segLatches.length > 0, `showcase unicorn latched 7-seg digits (${u.segLatches.length} latches)`);
    ok(JSON.stringify(u.segLatches) === JSON.stringify(r.segLatches),
        `showcase 7-seg digits identical (${u.segLatches.length} vs ${r.segLatches.length} latches)`);
    ok(u.out.includes('BTN=armed') && r.out.includes('BTN=armed'), 'showcase armed on both');
}

// WS2812: raw SPI1 DR byte stream (fire-and-forget, no CS) must match —
// frame decode (72B GRB) then follows identically on both.
{
    const firmware = readFileSync('site/arduino_ws2812.elf');
    const u = await runTap({ cpu: 'unicorn', firmware, ext_devices: {}, instr: 40000000 });
    const r = await runTap({ cpu: 'rust', firmware, ext_devices: {}, instr: 40000000 });
    ok(u.spiBytes.length > 70, `ws2812 unicorn saw strip bytes (${u.spiBytes.length})`);
    ok(JSON.stringify(u.spiBytes) === JSON.stringify(r.spiBytes),
        `ws2812 SPI bytes identical (${u.spiBytes.length} vs ${r.spiBytes.length})`);
    // Decode first complete frames to GRB LEDs on both (proves frame alignment).
    const frames = (bytes) => {
        const out = [];
        for (let f = 0; f + 72 <= bytes.length; f += 72) {
            const leds = [];
            for (let i = 0; i < 8; i++) {
                leds.push([bytes[f + i * 3], bytes[f + i * 3 + 1], bytes[f + i * 3 + 2]]);
            }
            out.push(leds);
        }
        return out;
    };
    const uf = frames(u.spiBytes), rf = frames(r.spiBytes);
    ok(uf.length > 0 && JSON.stringify(uf) === JSON.stringify(rf), `ws2812 decoded frames identical (${uf.length})`);
}

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed === 0 ? 0 : 1);
