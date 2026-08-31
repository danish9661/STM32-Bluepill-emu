// STM32F1 Emulator — minimal Vite template.
//
// Usage:
//   npm install
//   npm run dev
//
// Drop your firmware file (ELF, HEX, or raw BIN) onto the page or click "Load Firmware".

import { createEmulator } from 'stm32f1-emu/emulator';
import { parseIntelHex } from 'stm32f1-emu/emulator';

let emu = null;
let running = false;
let rafId = null;

const terminal = document.getElementById('terminal');
const stats = document.getElementById('stats');
const fileInput = document.getElementById('firmwareInput');
const runBtn = document.getElementById('runBtn');
const stopBtn = document.getElementById('stopBtn');
const resetBtn = document.getElementById('resetBtn');
const uartInput = document.getElementById('uartInput');
const uartSend = document.getElementById('uartSend');

// ── Helpers ─────────────────────────────────────────────────────────────────

function appendTerminal(text, cls = 'out') {
  const span = document.createElement('span');
  span.className = cls;
  span.textContent = text;
  terminal.appendChild(span);
  terminal.scrollTop = terminal.scrollHeight;
}

function resetButtons(hasFirmware) {
  runBtn.disabled = !hasFirmware || running;
  stopBtn.disabled = !running;
  resetBtn.disabled = !hasFirmware || running;
  uartInput.disabled = !hasFirmware;
  uartSend.disabled = !hasFirmware;
}

// ── Load firmware ───────────────────────────────────────────────────────────

fileInput.addEventListener('change', async (e) => {
  const file = e.target.files[0];
  if (!file) return;

  if (emu) { emu.close(); emu = null; running = false; if (rafId) cancelAnimationFrame(rafId); }
  terminal.innerHTML = '';
  stats.textContent = `Loading ${file.name}…`;

  const buf = new Uint8Array(await file.arrayBuffer());
  try {
    emu = await createEmulator({ firmware: buf, verbose: false });
    stats.textContent = `${file.name} loaded — click Run.`;
    resetButtons(true);
  } catch (err) {
    stats.textContent = `Error: ${err.message}`;
    console.error(err);
  }
});

// ── Run loop ────────────────────────────────────────────────────────────────

const BATCH = 20000;
let lastTime = performance.now();
let frameCount = 0;

function runFrame() {
  if (!running || !emu) return;

  const result = emu.step(BATCH);

  // Drain UART output
  const uart = emu.getUartOutput();
  if (uart) appendTerminal(uart);

  // Stats
  frameCount++;
  const now = performance.now();
  if (now - lastTime >= 1000) {
    const ips = Math.round((frameCount * BATCH) / ((now - lastTime) / 1000));
    const inst = emu.getRegisters().PC; // approximate
    stats.innerHTML = `IPS: <b>${(ips / 1e6).toFixed(1)}M</b> | Batches: <b>${frameCount}</b>`;
    frameCount = 0;
    lastTime = now;
  }

  if (result.stopped) {
    running = false;
    appendTerminal('\n[Emulator stopped]\n');
    resetButtons(true);
    return;
  }

  rafId = requestAnimationFrame(runFrame);
}

runBtn.addEventListener('click', () => {
  if (!emu || running) return;
  running = true;
  lastTime = performance.now();
  frameCount = 0;
  resetButtons(true);
  rafId = requestAnimationFrame(runFrame);
});

stopBtn.addEventListener('click', () => {
  running = false;
  if (emu) emu.stop();
  resetButtons(true);
});

resetBtn.addEventListener('click', async () => {
  if (!emu) return;
  running = false;
  if (rafId) cancelAnimationFrame(rafId);
  terminal.innerHTML = '';
  try {
    // Re-create from the same firmware
    const buf = fileInput.files[0] ? new Uint8Array(await fileInput.files[0].arrayBuffer()) : null;
    if (buf) {
      emu.close();
      emu = await createEmulator({ firmware: buf, verbose: false });
      stats.textContent = 'Reset.';
    }
  } catch (err) {
    stats.textContent = `Reset error: ${err.message}`;
  }
  resetButtons(true);
});

// ── UART input ──────────────────────────────────────────────────────────────

function sendUart() {
  const text = uartInput.value;
  if (!text || !emu) return;
  for (let i = 0; i < text.length; i++) {
    emu.uartRx(text.charCodeAt(i));
  }
  appendTerminal(text, 'in');
  uartInput.value = '';
}

uartSend.addEventListener('click', sendUart);
uartInput.addEventListener('keydown', (e) => { if (e.key === 'Enter') sendUart(); });
