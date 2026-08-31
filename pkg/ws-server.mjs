#!/usr/bin/env node
// pkg/ws-server.mjs — WebSocket bridge for the STM32F1 emulator.
//
// Usage:  node pkg/ws-server.mjs <firmware.elf> [--port=8080] [--config=config.yaml]
//
// Serves the site/ directory over HTTP and streams emulator events to connected
// browser clients over WebSocket.  Clients can send input (UART RX, GPIO, CAN).

import { createServer } from 'http';
import { readFile, stat } from 'fs/promises';
import { join, extname, resolve } from 'path';
import { fileURLToPath } from 'url';
import { WebSocketServer } from 'ws';
import { createEmulator } from './emulator.js';

const __dirname = fileURLToPath(new URL('.', import.meta.url));
const SITE = join(__dirname, '..', 'site');

const MIME = {
    '.html': 'text/html', '.js': 'application/javascript', '.mjs': 'application/javascript',
    '.css': 'text/css', '.wasm': 'application/wasm', '.json': 'application/json',
    '.png': 'image/png', '.svg': 'image/svg+xml', '.ico': 'image/x-icon',
    '.elf': 'application/octet-stream', '.bin': 'application/octet-stream',
};

// ── CLI args ────────────────────────────────────────────────────────────────
let firmwarePath = null, port = 8080, maxInstructions = 0;
const args = process.argv.slice(2);
for (const a of args) {
    if (a.startsWith('--port=')) port = parseInt(a.split('=')[1]);
    else if (a.startsWith('--max=')) maxInstructions = parseInt(a.split('=')[1]);
    else if (!a.startsWith('-')) firmwarePath = a;
}
if (!firmwarePath) { console.error('Usage: node pkg/ws-server.mjs <firmware.elf> [--port=8080]'); process.exit(1); }

// ── Load firmware ───────────────────────────────────────────────────────────
const fwBuf = await readFile(resolve(firmwarePath));
console.log(`Firmware: ${firmwarePath} (${fwBuf.length} bytes)`);

const emu = await createEmulator({ firmware: fwBuf });

// ── HTTP server (static files from site/) ───────────────────────────────────
const server = createServer(async (req, res) => {
    const url = new URL(req.url, 'http://localhost');
    let filePath = join(SITE, url.pathname === '/' ? 'index.html' : url.pathname);
    try {
        const s = await stat(filePath);
        if (s.isDirectory()) filePath = join(filePath, 'index.html');
        const content = await readFile(filePath);
        res.writeHead(200, { 'Content-Type': MIME[extname(filePath)] || 'application/octet-stream' });
        res.end(content);
    } catch {
        res.writeHead(404); res.end('Not found');
    }
});

// ── WebSocket server ────────────────────────────────────────────────────────
const wss = new WebSocketServer({ server });
const clients = new Set();

wss.on('connection', (ws) => {
    clients.add(ws);
    console.log(`Client connected (${clients.size} total)`);
    ws.on('close', () => { clients.delete(ws); console.log(`Client disconnected (${clients.size} total)`); });
    ws.on('message', (raw) => {
        try {
            const cmd = JSON.parse(String(raw));
            if (cmd.type === 'uart_rx')       emu.uartRxAddr(cmd.addr || 0x40013800, cmd.byte);
            else if (cmd.type === 'gpio_set')  emu.gpioSetInput(cmd.port, cmd.pin, cmd.high);
            else if (cmd.type === 'can_inject') emu.canInjectMessage(cmd.addr, cmd.tir, cmd.tdtr, cmd.tdlr, cmd.tdhr);
        } catch (e) { /* ignore bad commands */ }
    });
    // Send a welcome frame so the client knows it's connected
    ws.send(JSON.stringify({ type: 'hello', firmware: firmwarePath }));
});

// ── Emulation loop ──────────────────────────────────────────────────────────
const BATCH = 20000;
let totalInstructions = 0, frameCount = 0, fps = 0, lastFpsTime = Date.now();

const tick = () => {
    if (clients.size === 0) return;            // idle when nobody is watching
    emu.step(BATCH);
    totalInstructions += BATCH;

    const events = Array.from(emu.drainEvents());
    const pins   = Array.from(emu.takePinEvents());

    const msg = JSON.stringify({ e: events, p: pins, fps, t: totalInstructions });
    for (const c of clients) { if (c.readyState === 1) c.send(msg); }

    frameCount++;
    const now = Date.now();
    if (now - lastFpsTime >= 1000) { fps = frameCount; frameCount = 0; lastFpsTime = now; }

    if (maxInstructions > 0 && totalInstructions >= maxInstructions) {
        console.log(`Reached ${maxInstructions} instructions, stopping.`);
        process.exit(0);
    }
};

setInterval(tick, 16);  // ~60 fps update rate

server.listen(port, () => {
    console.log(`WebSocket bridge: http://localhost:${port}   ws://localhost:${port}`);
    console.log(`Streaming events to ${clients.size} client(s)...`);
});
