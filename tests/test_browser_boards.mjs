// Board selector + SVG art + demo filter coverage: switching chips filters
// the preset menu to that board's demos, swaps the rig artwork (chip marking
// + LED), and board presets boot their matched ELF on the matched chip.
import { test, expect } from '@playwright/test';

const visPresets = (page) =>
    page.$$eval('#presetSelect option:not([hidden])', (els) => els.map((e) => e.value));

test.describe('Board selector', () => {
    test('chip filters presets to that board', async ({ page }) => {
        await page.goto('http://localhost:8765/');
        await page.selectOption('#chipSelect', 'maple_mini');
        const vis = await visPresets(page);
        expect(vis).toContain('board_maple_echo');
        expect(vis).toContain('board_maple_showcase');
        expect(vis).toContain('board_maple_rtc');
        expect(vis).toContain('blink'); // portable demos stay
        expect(vis).not.toContain('board_pill_echo');
        expect(vis).not.toContain('board_nucleo_echo');
        expect(vis).not.toContain('board_rc_rtc');
        expect(vis).not.toContain('can_dual');
    });
    test('F105 shows only portable + dual-CAN', async ({ page }) => {
        await page.goto('http://localhost:8765/');
        await page.selectOption('#chipSelect', 'stm32f105');
        const vis = await visPresets(page);
        expect(vis).toContain('can_dual');
        expect(vis).toContain('showcase');
        expect(vis).not.toContain('board_pill');
        expect(vis).not.toContain('board_maple');
    });
    test('RC shows high-density desk, F103C8 does not', async ({ page }) => {
        await page.goto('http://localhost:8765/');
        await page.selectOption('#chipSelect', 'stm32f103rc');
        expect(await visPresets(page)).toContain('hd_fsmc');
        await page.selectOption('#chipSelect', 'stm32f103c8');
        expect(await visPresets(page)).not.toContain('hd_fsmc');
    });
    test('rig SVG changes per board', async ({ page }) => {
        await page.goto('http://localhost:8765/');
        await page.selectOption('#chipSelect', 'nucleo_f103rb');
        await expect(page.locator('#boardWrap')).toContainText('STM32F103RBT6');
        await expect(page.locator('#boardWrap')).toContainText('LD2 (D13)');
        await page.selectOption('#chipSelect', 'maple_mini');
        await expect(page.locator('#boardWrap')).toContainText('STM32F103CBT6');
        await expect(page.locator('#boardWrap')).toContainText('D33 LED');
        await page.selectOption('#chipSelect', 'stm32f103c8');
        await expect(page.locator('#boardWrap')).toContainText('STM32F103C8T6');
        await expect(page.locator('#boardWrap')).toContainText('PC13 LED');
    });
    test('board preset auto-switches chip and boots matched ELF', async ({ page }) => {
        await page.goto('http://localhost:8765/');
        await page.selectOption('#chipSelect', 'stm32f103c8');
        // board_maple_rtc is hidden under f103c8: picking it must flip the chip
        await page.selectOption('#presetSelect', 'board_maple_rtc');
        await expect(page.locator('#chipSelect')).toHaveValue('maple_mini');
        await page.click('#loadPresetBtn');
        await page.click('#runBtn');
        await page.waitForFunction(
            (n) => (document.querySelector('#terminal')?.innerText || '').includes(n),
            'board: Maple Mini', { timeout: 120000 });
        const chip = await page.$eval('#statChip', (el) => el.innerText);
        expect(chip).toContain('Maple');
    });
    test('board echo round-trips typed text', async ({ page }) => {
        await page.goto('http://localhost:8765/');
        await page.selectOption('#presetSelect', 'board_rc_echo');
        await page.click('#loadPresetBtn');
        await page.click('#runBtn');
        await page.fill('#uartInput', 'Z9');
        await page.click('#sendBtn');
        await page.waitForFunction(
            (n) => (document.querySelector('#terminal')?.innerText || '').includes(n),
            'Z9', { timeout: 120000 });
    });
});
