// Browser CDP smoke: drives browser-test.html in headless Chrome against the
// merged module, polls the #result element, asserts 39/39.
import { spawn } from 'child_process';

const PORT = 8351;
const DEBUG = 9231;
const BUILD = '/home/danish1075/Documents/stm32 emu blue pill/tests/arduino_periph_test/build';

const http = spawn('python3', ['-m', 'http.server', String(PORT), '-d', new URL('.', import.meta.url).pathname], { stdio: 'ignore' });
const chrome = spawn('google-chrome-stable', [
    '--headless=new', '--disable-gpu', '--no-sandbox',
    `--remote-debugging-port=${DEBUG}`,
    `--user-data-dir=${new URL('.', import.meta.url).pathname}.chrome`,
    'about:blank',
], { stdio: 'ignore' });

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let page;
for (let i = 0; i < 50 && !page; i++) {
    await sleep(200);
    try {
        const pages = await fetch(`http://localhost:${DEBUG}/json`).then((r) => r.json());
        page = pages.find((p) => p.type === 'page');
    } catch {}
}
if (!page) throw new Error('no chrome page');
const ws = new WebSocket(page.webSocketDebuggerUrl);
let id = 0;
const pending = new Map();
const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
        const i = ++id;
        pending.set(i, { resolve, reject });
        ws.send(JSON.stringify({ id: i, method, params }));
    });
ws.onmessage = (ev) => {
    const m = JSON.parse(ev.data);
    if (m.id && pending.has(m.id)) {
        const p = pending.get(m.id);
        pending.delete(m.id);
        m.error ? p.reject(new Error(m.error.message)) : p.resolve(m.result);
    }
};
ws.onerror = (e) => { console.error('ws error', e); process.exit(1); };
await new Promise((r) => { ws.onopen = r; });

await send('Page.enable');
await send('Page.navigate', { url: `http://localhost:${PORT}/browser-test.html` });
console.log('navigated, waiting for run to finish…');

const deadline = Date.now() + 180000;
let result = '';
while (Date.now() < deadline) {
    await sleep(5000);
    const r = await send('Runtime.evaluate', { expression: `document.getElementById('result').textContent`, returnByValue: true });
    result = r.result?.value || '';
    if (result.startsWith('ERR') || result.includes('PASS') || result.includes('FAIL (can=')) break;
}
console.log('RESULT:', result.split('\n').join(' | '));
const pass = result.includes('PASS 39/39');
console.log(pass ? 'BROWSER SMOKE PASS' : 'BROWSER SMOKE FAIL');
ws.close();
chrome.kill();
http.kill();
process.exit(pass ? 0 : 1);
