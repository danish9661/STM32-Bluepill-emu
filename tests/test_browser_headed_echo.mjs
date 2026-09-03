import { test, expect } from '@playwright/test';
test('headed: echo preset UART round-trip', async ({ page }) => {
  page.on('console', m => { if (m.type() === 'error') console.log('BROWSER ERR:', m.text().slice(0,150)); });
  page.on('pageerror', e => console.log('PAGEERROR:', String(e).slice(0,200)));
  await page.goto('http://localhost:8765/');
  await page.selectOption('#presetSelect', 'echo');
  await page.click('#loadPresetBtn');
  await expect(page.locator('#runBtn')).toBeEnabled({ timeout: 30000 });
  await page.click('#runBtn');
  await page.waitForTimeout(3000);
  await page.fill('#uartInput', 'Hi');
  await page.click('#sendBtn');
  await page.waitForTimeout(4000);
  const term = await page.locator('#terminal').textContent();
  console.log('TERMINAL:', JSON.stringify(term.slice(-300)));
  await page.screenshot({ path: '/tmp/echo_headed.png' });
  expect(term).toContain('Hi');
});
