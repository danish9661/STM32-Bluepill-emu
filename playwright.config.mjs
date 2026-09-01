import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './tests',
  testMatch: 'test_browser*.mjs',
  timeout: 120_000,
  retries: 0,
  use: {
    headless: true,
    viewport: { width: 1280, height: 800 },
    launchOptions: {
      args: ['--no-sandbox', '--disable-gpu', '--disable-software-rasterizer'],
    },
  },
  webServer: {
    command: 'python3 -m http.server -d site 8765',
    port: 8765,
    reuseExistingServer: true,
    timeout: 10_000,
  },
});
