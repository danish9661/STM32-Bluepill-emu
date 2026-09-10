// Site verification: every static page loads without console/page errors,
// shows its key content, and screenshots cleanly (headed Chromium, matching
// how a visitor sees it). Screenshots land in test-results/ (gitignored).
import { test, expect } from '@playwright/test';

const PAGES = [
    ['/', '#loadPresetBtn', 'demo preset loader'],
    ['/about.html', 'main', 'about content'],
    ['/docs.html', '#doc-cards .card', 'docs hub (manifest-driven)'],
    ['/doc.html?f=BOARDS.md', 'table', 'rendered board matrix'],
    ['/doc.html?f=GDB.md', 'pre', 'rendered code blocks'],
    ['/doc.html?f=NOPE.md', '.err', 'missing doc error state'],
    ['/ws-viewer.html', 'main, body', 'ws viewer'],
];

test.describe('Site pages', () => {
    for (const [path, needle, name] of PAGES) {
        test(`${path} loads clean (${name})`, async ({ page }) => {
            const errors = [];
            page.on('console', (msg) => {
                // ws-viewer auto-dials its server; without one running the
                // browser logs a connection error (the page itself retries).
                if (path === '/ws-viewer.html' && msg.text().includes('ws://')) return;
                // The missing-doc probe 404s by design (error UI asserted).
                if (msg.text().includes('404 (File not found)')) return;
                if (msg.type() === 'error') errors.push(msg.text());
            });
            page.on('pageerror', (err) => errors.push(String(err)));
            await page.goto(`http://localhost:8765${path}`);
            await expect(page.locator(needle).first()).toBeVisible({ timeout: 30000 });
            await page.waitForTimeout(1500);
            expect(errors, `console/page errors on ${path}`).toEqual([]);
            const shot = `site-${path === '/' ? 'index' : path.slice(1).replace(/[/.]/g, '_')}.png`;
            await page.screenshot({ path: `test-results/${shot}`, fullPage: false });
        });
    }
    test('demo nav reaches docs and back', async ({ page }) => {
        await page.goto('http://localhost:8765/');
        await page.click('header a.nav[href="docs.html"]');
        await expect(page).toHaveURL(/docs\.html$/);
        await page.click('header a.nav[href="index.html"]');
        await expect(page).toHaveURL(/index\.html$/);
    });
});
