// Browser bus-tap decode test: the page 7-seg decoder shape running
// in-page against the showcase firmware (covers the write-tap feed path
// in a real browser).
import { test, expect } from '@playwright/test';

test.describe('Browser bus-tap decode', () => {
  test('showcase: 7-seg bus-tap decodes in-page', async ({ page }) => {
    await page.goto('http://localhost:8765/');
    // 7-seg latch = 4 SPI1 DR bytes while PA4 is low (page decoder shape).
    const digits = await page.evaluate(async () => {
      const { createEmulator } = await import('./emulator.js');
      const load = async (name) => new Uint8Array(await (await fetch(name)).arrayBuffer());
      const fw = await load('arduino_hw_showcase.elf');
      const emu = await createEmulator({
        firmware: fw,
        ext_devices: {
          i2c_oled: [{ peripheral: 'I2C1', address: 0x3C }],
          lcd: [{ peripheral: 'SPI1', cs: 'PA8' }],
        },
      });
      const PA4 = 1 << 4;
      let low = false, buf = [];
      const latches = [];
      emu.onPeriphWrite((addr, size, value) => {
        if (addr === 0x4001080C) low = (value & PA4) === 0;
        else if (addr === 0x40010810) { if (value & PA4) low = false; if (value & (PA4 << 16)) low = true; }
        else if (addr === 0x40010814) { if (value & PA4) low = true; }
        else if (addr === 0x4001300C && low) {
          for (let i = 0; i < size; i++) {
            buf.push((value >>> (i * 8)) & 0xFF);
            if (buf.length === 4) { latches.push(buf); buf = []; }
          }
        }
      });
      await emu.run(260_000_000);
      emu.close();
      return latches;
    });
    console.log(`7-seg latches on rust: ${digits.length}, first=${JSON.stringify(digits[0])}`);
    expect(digits.length).toBeGreaterThan(0);
    // Seconds counter latches increment across 1Hz blocks (last digit changes).
    const last = digits.map(d => d[3]);
    expect(new Set(last).size).toBeGreaterThan(1);
  });
});
