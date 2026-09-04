// Browser Rust-CPU backend tests: same in-page createEmulator shape as
// test_browser.mjs, but cpu:'rust' (no Unicorn anywhere in the loop).
// Asserts periph39 39/39 + MIPS and showcase 7-seg bus-tap decode.
import { test, expect } from '@playwright/test';

test.describe('Browser Rust CPU backend (Path B)', () => {
  test('periph39: full 200M run, 39/39 checks, no unicorn', async ({ page }) => {
    page.on('console', msg => {
      if (msg.type() === 'error') console.log('BROWSER ERR:', msg.text());
    });
    await page.goto('http://localhost:8765/');

    const result = await page.evaluate(async () => {
      // NOTE: index.html loads unicorn_arm.js globally via <script>, so
      // MUnicorn existing proves nothing — emu.uc === null below is the
      // no-Unicorn proof for this instance.
      const { createEmulator, parseElf } = await import('./emulator.js');
      const load = async (name) => {
        const r = await fetch(name);
        return new Uint8Array(await r.arrayBuffer());
      };
      const [fw, e1, e2, sf1, sf2] = await Promise.all([
        load('arduino_periph_test.elf'),
        load('arduino_eeprom.bin'),
        load('arduino_eeprom2.bin'),
        load('arduino_spi_flash.bin'),
        load('arduino_spi_flash2.bin'),
      ]);
      const emu = await createEmulator({
        cpu: 'rust',
        firmware: fw,
        ext_devices: {
          i2c_eeprom: [
            { peripheral: 'I2C1', address: 0x50, data: e1 },
            { peripheral: 'I2C2', address: 0x51, data: e2 },
          ],
          i2c_oled: [{ peripheral: 'I2C1', address: 0x3C, width: 128, height: 64 }],
          spi_flash: [
            { peripheral: 'SPI1', jedec_id: 0xEF4016, data: sf1, cs: 'PA4' },
            { peripheral: 'SPI2', jedec_id: 0xEF4017, data: sf2, cs: 'PB12' },
          ],
          lcd: [{ peripheral: 'SPI1', cs: 'PA1' }],
          touchscreen: [{ peripheral: 'SPI1', cs: 'PA2', touch_detected_pin: 'PA3' }],
        },
      });
      if (emu.uc) throw new Error('emu.uc set on rust backend');
      emu.uartRx(0x41);
      emu.uartRx(0x42);
      const MAX = 200_000_000, CHUNK = 10_000_000;
      const canSym = parseElf(fw).symbols.find((s) => s.name === 'canRxArmed');
      if (!canSym) throw new Error('canRxArmed symbol missing from ELF');
      let canInjected = false, done = 0;
      const t0 = performance.now();
      while (done < MAX) {
        const n = Math.min(CHUNK, MAX - done);
        const r = await emu.run(n);
        done += n;
        if (!canInjected && emu.memRead32(canSym.addr) !== 0) {
          canInjected = emu.canInjectMessage(0x40006400, 0 << 21, 2, 0xDEAD, 0);
        }
        if (r.stopped) break;
      }
      const elapsed = (performance.now() - t0) / 1000;
      const out = String(emu.getUartOutput() || '');
      emu.close();
      return {
        done, elapsed: elapsed.toFixed(2),
        mips: (done / elapsed / 1e6).toFixed(1),
        canInjected,
        summary: out.split('\n').filter(s => s.includes('SUMMARY')).join(' ').trim(),
        fails: out.split('\n').filter(s => s.includes('FAIL')),
      };
    });

    console.log(`${result.done} instructions in ${result.elapsed}s (${result.mips} MIPS, rust)`);
    console.log(`Summary: ${result.summary || '(missing)'}`);
    if (result.fails.length) console.log(`FAIL lines: ${result.fails.join('; ')}`);
    expect(result.canInjected).toBe(true);
    expect(result.fails).toHaveLength(0);
    expect(result.summary).toMatch(/pass=39 fail=0/);
  });

  test('showcase: 7-seg bus-tap decodes on rust backend', async ({ page }) => {
    await page.goto('http://localhost:8765/');
    // 7-seg latch = 4 SPI1 DR bytes while PA4 is low (page decoder shape).
    const digits = await page.evaluate(async () => {
      const { createEmulator } = await import('./emulator.js');
      const load = async (name) => new Uint8Array(await (await fetch(name)).arrayBuffer());
      const fw = await load('arduino_hw_showcase.elf');
      const emu = await createEmulator({
        cpu: 'rust',
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
