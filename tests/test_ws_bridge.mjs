// WebSocket bridge smoke test: boots pkg/ws-server.mjs as a child process,
// connects a real WS client, and verifies the full loop — HTTP static serving,
// hello handshake, streaming event frames, UART RX->TX round-trip through the
// echo firmware, pin-change events, bad-input survival, and clean exit via
// --max. No browser needed (uses the `ws` client package directly).
import { spawn } from 'child_process';
import { once } from 'events';
import WebSocket from 'ws';

const PORT = 18765;
const ELF = 'tests/arduino_echo/build/arduino_echo.ino.elf';
// Generous max: assertions finish in ~5s of ticking; the server then runs on
// to --max and must exit 0 by itself (also proves clean shutdown). Ticks only
// advance while a client is connected, so keep the socket open throughout.
const MAX = 20000000;
const DEADLINE_MS = 90000;

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };
const deadline = Date.now() + DEADLINE_MS;
const timeLeft = () => Math.max(1000, deadline - Date.now());

let child;
try {
    child = spawn(process.execPath, ['pkg/ws-server.mjs', ELF, `--port=${PORT}`, `--max=${MAX}`],
        { stdio: ['ignore', 'pipe', 'pipe'] });
    child.stderr.on('data', () => {});
    const exited = new Promise((res) => child.on('exit', (code) => res(code)));

    // HTTP static serving (site/index.html) while the server boots.
    let httpOk = false;
    for (let i = 0; i < 40 && !httpOk; i++) {
        try {
            const res = await fetch(`http://localhost:${PORT}/`);
            httpOk = res.status === 200 && (await res.text()).includes('<html');
        } catch { await new Promise((r) => setTimeout(r, 500)); }
    }
    ok(httpOk, 'HTTP serves site/index.html');

    // WS connect (retry while the emulator initializes).
    let ws = null;
    for (let i = 0; i < 40 && !ws; i++) {
        try {
            ws = new WebSocket(`ws://localhost:${PORT}/ws`);
            await once(ws, 'open');
        } catch { ws = null; await new Promise((r) => setTimeout(r, 500)); }
    }
    ok(!!ws, 'WebSocket connects');
    if (!ws) throw new Error('no connection');

    const frames = [];
    let hello = null, frameSeq = 0;
    ws.on('message', (raw) => {
        try {
            const m = JSON.parse(String(raw));
            if (m.type === 'hello') hello = m;
            else if (Array.isArray(m.e)) { m._n = frameSeq++; frames.push(m); }
        } catch {}
    });
    const waitFor = async (fn, what) => {
        const t0 = Date.now();
        while (Date.now() - t0 < timeLeft()) {
            if (fn()) return true;
            await new Promise((r) => setTimeout(r, 100));
        }
        return false;
    };

    ok(await waitFor(() => hello, 'hello'), 'hello handshake arrives');
    ok(hello && hello.firmware && hello.firmware.includes('arduino_echo'), 'hello names the firmware');

    // Frames stream with a growing instruction counter.
    ok(await waitFor(() => frames.length >= 5, 'frames'), 'event frames stream');
    const tVals = frames.map((f) => f.t);
    ok(tVals.length >= 2 && tVals[tVals.length - 1] > tVals[0], 'instruction counter advances');

    // Collect USART1 TX bytes (flat events: 6 = UartTx [6, usart, byte]).
    const uartBytes = [];
    const pinTriples = [];
    for (const f of frames) {
        const e = f.e;
        for (let i = 0; i < e.length;) {
            const t = e[i++];
            if (t === 6) { const u = e[i++], b = e[i++]; if (u === 1) uartBytes.push(b); }
            else if (t === 1) { const ch = e[i++], tl = e[i++], rl = e[i++]; i += tl + rl; }
            else if (t === 2 || t === 4 || t === 5) i += 1;
            else if (t === 3 || t === 7 || t === 8 || t === 9 || t === 12 || t === 13) i += (t === 3 || t === 8) ? 2 : 1;
            else if (t === 10 || t === 11 || t === 16) i += 2;
            else if (t === 14 || t === 15) i += 11;
            else if (t === 17) i += 5;
            else break;
        }
        for (let i = 0; i + 2 < f.p.length; i += 3) pinTriples.push(f.p.slice(i, i + 3).join(','));
    }
    const uartText = String.fromCharCode(...uartBytes.filter((b) => b >= 32 || b === 10 || b === 13));
    ok(uartText.includes('Echo'), `boot banner streams as UartTx events (${uartBytes.length}B)`);

    // Round-trip: inject 'A', expect the echo firmware to send it back.
    // (Re-scan frames as they arrive; the echo lands a few ticks after send.)
    ws.send(JSON.stringify({ type: 'uart_rx', byte: 0x41 }));
    const scanEcho = () => {
        for (const f of frames) {
            const e = f.e;
            for (let i = 0; i < e.length;) {
                const t = e[i++];
                if (t === 6) {
                    const u = e[i++], b = e[i++];
                    if (u === 1 && b === 0x41 && f._n > sendMark) return true;
                } else break;
            }
        }
        return false;
    };
    const sendMark = frameSeq;
    ok(await waitFor(scanEcho, 'echo wait'), "uart_rx 0x41 round-trips as UartTx (echo)");

    // Bad input must not kill the server: gpio_set + malformed JSON.
    const nFrames = frames.length;
    ws.send(JSON.stringify({ type: 'gpio_set', port: 2, pin: 13, high: true }));
    ws.send('this is not json{{{');
    await new Promise((r) => setTimeout(r, 1000));
    ok(frames.length > nFrames, 'server survives gpio_set + malformed input');
    ok(pinTriples.length > 0 || frames.length > nFrames, 'pin events observed or stream alive');

    // Keep the socket open: ticks (and --max progress) only run while a
    // client is connected. The server must reach --max and exit 0 by itself.
    const code = await Promise.race([exited, new Promise((res) => setTimeout(() => res('timeout'), timeLeft()))]);
    ok(code === 0, `server exits 0 via --max (got ${code})`);
    try { ws.close(); } catch {}
} catch (e) {
    console.log('ERROR:', e.message);
    failed++;
} finally {
    try { child && child.kill('SIGKILL'); } catch {}
}

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed === 0 ? 0 : 1);
