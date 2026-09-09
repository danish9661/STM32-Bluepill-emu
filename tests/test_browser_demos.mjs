import { test, expect } from '@playwright/test';
test.describe('New demo presets', () => {
  for (const [preset, needle] of [['rtc_clock', '12:00:01'], ['servo', 'deg=10'], ['dac_sine', 'adc='], ['i2c_scan', 'found=2'], ['can_chat', 'rx id=100'], ['stopwatch', 'Stopwatch demo'], ['pwm_wave', 'duty=143 ccr=143'], ['mini_rtos', 'B1 t='], ['sd_logger', 'logger done']]) {
    test(`${preset} loads and prints`, async ({ page }) => {
      page.on('console', msg => { if (msg.type() === 'error') console.log('BROWSER ERR:', msg.text()); });
      await page.goto('http://localhost:8765/');
      await page.selectOption('#presetSelect', preset);
      await page.click('#loadPresetBtn');
      await page.click('#runBtn');
      await page.waitForFunction(
        (n) => (document.querySelector('#terminal')?.innerText || '').includes(n),
        needle, { timeout: 120000 });
      const term = await page.$eval('#terminal', el => el.innerText);
      expect(term).toContain(needle);
    });
  }
  test('usb_cdc enumerates and echoes live', async ({ page }) => {
    page.on('console', msg => { if (msg.type() === 'error') console.log('BROWSER ERR:', msg.text()); });
    await page.goto('http://localhost:8765/');
    await page.selectOption('#presetSelect', 'usb_cdc');
    await page.click('#loadPresetBtn');
    await page.click('#runBtn');
    await page.click('#usbEnumBtn');
    await page.waitForFunction(
      (n) => (document.querySelector('#terminal')?.innerText || '').includes(n),
      'enumerated — CDC serial live', { timeout: 120000 });
    await page.fill('#usbEchoInput', 'Hi');
    await page.click('#usbEchoBtn');
    await page.waitForFunction(
      (n) => (document.querySelector('#terminal')?.innerText || '').includes(n),
      'USB EP1 echo: "Hi"', { timeout: 120000 });
    const term = await page.$eval('#terminal', el => el.innerText);
    expect(term).toContain('config descriptor (68B, 2 packets)');
  });
  test('i2c_slave writes and reads live', async ({ page }) => {
    page.on('console', msg => { if (msg.type() === 'error') console.log('BROWSER ERR:', msg.text()); });
    await page.goto('http://localhost:8765/');
    await page.selectOption('#presetSelect', 'i2c_slave');
    await page.click('#loadPresetBtn');
    await page.click('#runBtn');
    await page.click('#i2cWriteBtn');
    await page.waitForFunction(
      (n) => (document.querySelector('#terminal')?.innerText || '').includes(n),
      'rx=2:', { timeout: 120000 });
    await page.click('#i2cReadBtn');
    await page.waitForFunction(
      (n) => (document.querySelector('#terminal')?.innerText || '').includes(n),
      'I2C host read: 48 69', { timeout: 120000 });
  });
});
