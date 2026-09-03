// Browser-based firmware test: loads createEmulator() in the actual Chromium
// browser, runs the 24-peripheral firmware to completion, and asserts 39/39.
//
// Unlike the page's rAF runLoop (which caps at ~40M instr), this test runs the
// full 200M firmware load directly in-browser via emulator.js, same as the
// test_emulator_js.mjs Node path but validating the browser WASM stack.
import { test, expect } from '@playwright/test';

test.describe('Browser firmware tests', () => {
  test('periph37: full 200M run, 39/39 checks', async ({ page }) => {
    // Capture console for debugging
    page.on('console', msg => {
      if (msg.type() === 'error') console.log('BROWSER ERR:', msg.text());
    });

    await page.goto('http://localhost:8765/');

    // Run the firmware directly in the browser context via createEmulator.
    // This bypasses the page's rAF runLoop and its step budget.
    const result = await page.evaluate(async () => {
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

      // Inject 'A' + 'B' (same as CLI stdin pump)
      emu.uartRx(0x41);
      emu.uartRx(0x42);

      const MAX = 200_000_000;
      const CHUNK = 10_000_000;
      // Resolved from ELF symbols — a hardcoded address goes stale on rebuild
      // (0x200000b8 silently became canRxTries: 3M-iteration RF0R spin storms).
      const canSym = parseElf(fw).symbols.find((s) => s.name === 'canRxArmed');
      if (!canSym) throw new Error('canRxArmed symbol missing from ELF');
      const CAN_RAM_FLAG = canSym.addr;
      let canInjected = false;
      let done = 0;
      const t0 = performance.now();

      while (done < MAX) {
        const n = Math.min(CHUNK, MAX - done);
        const r = await emu.run(n);
        done += n;
        // CAN autopilot: inject when firmware arms the CAN RX test
        if (!canInjected && emu.memRead32(CAN_RAM_FLAG) !== 0) {
          canInjected = emu.canInjectMessage(0x40006400, 0 << 21, 2, 0xDEAD, 0);
        }
        if (r.stopped) break;
      }

      const elapsed = (performance.now() - t0) / 1000;
      const out = String(emu.getUartOutput() || '');
      const summary = out.split('\n').filter(s => s.includes('SUMMARY')).join(' ').trim();
      const fails = out.split('\n').filter(s => s.includes('FAIL'));
      const lastLines = out.split('\n').slice(-10);

      emu.close();

      return {
        done,
        elapsed: elapsed.toFixed(2),
        canInjected,
        summary,
        fails,
        lastLines,
      };
    });

    console.log(`${result.done} instructions in ${result.elapsed}s`);
    console.log(`CAN injected: ${result.canInjected}`);
    console.log(`Summary: ${result.summary || '(missing)'}`);
    if (result.fails.length) console.log(`FAIL lines: ${result.fails.join('; ')}`);

    expect(result.canInjected).toBe(true);
    expect(result.fails).toHaveLength(0);
    expect(result.summary).toMatch(/pass=39 fail=0/);
  });

  test('GPIO grid renders after loading firmware', async ({ page }) => {
    await page.goto('http://localhost:8765/');
    await page.selectOption('#presetSelect', 'blink');
    await page.click('#loadPresetBtn');
    await expect(page.locator('#runBtn')).toBeEnabled({ timeout: 30000 });
    await page.click('#runBtn');
    await page.waitForTimeout(3000);
    const gpioPorts = await page.locator('.gpio-port').count();
    expect(gpioPorts).toBeGreaterThanOrEqual(3);
    await page.click('#stopBtn');
  });

  test('chip selector works (STM32F105)', async ({ page }) => {
    await page.goto('http://localhost:8765/');
    await page.selectOption('#chipSelect', 'stm32f105');
    await page.selectOption('#presetSelect', 'periph37');
    await page.click('#loadPresetBtn');
    await expect(page.locator('#runBtn')).toBeEnabled({ timeout: 30000 });
    const termText = await page.locator('#terminal').textContent();
    expect(termText).toContain('STM32F105');

    // Run directly in browser context (bypass rAF loop)
    const result = await page.evaluate(async () => {
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
        chip: 'STM32F105',
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
      emu.uartRx(0x41);
      emu.uartRx(0x42);
      const MAX = 200_000_000, CHUNK = 10_000_000;
      // Resolved from ELF symbols — a hardcoded address goes stale on rebuild
      // (0x200000b8 silently became canRxTries: 3M-iteration RF0R spin storms).
      const canSym = parseElf(fw).symbols.find((s) => s.name === 'canRxArmed');
      if (!canSym) throw new Error('canRxArmed symbol missing from ELF');
      const CAN_RAM_FLAG = canSym.addr;
      let canInjected = false, done = 0;
      while (done < MAX) {
        const n = Math.min(CHUNK, MAX - done);
        const r = await emu.run(n);
        done += n;
        if (!canInjected && emu.memRead32(CAN_RAM_FLAG) !== 0) {
          canInjected = emu.canInjectMessage(0x40006400, 0 << 21, 2, 0xDEAD, 0);
        }
        if (r.stopped) break;
      }
      const out = String(emu.getUartOutput() || '');
      const summary = out.split('\n').filter(s => s.includes('SUMMARY')).join(' ').trim();
      const fails = out.split('\n').filter(s => s.includes('FAIL'));
      emu.close();
      return { done, summary, fails, canInjected };
    });
    console.log(`F105: ${result.done} instructions, CAN=${result.canInjected}`);
    console.log(`Summary: ${result.summary || '(missing)'}`);
    expect(result.canInjected).toBe(true);
    expect(result.fails).toHaveLength(0);
    expect(result.summary).toMatch(/pass=39 fail=0/);
  });
});
