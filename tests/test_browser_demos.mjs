import { test, expect } from '@playwright/test';
test.describe('New demo presets', () => {
  for (const [preset, needle] of [['rtc_clock', '12:00:01'], ['servo', 'deg=10']]) {
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
});
