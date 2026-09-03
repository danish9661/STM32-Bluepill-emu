// site/worker.js — off-main-thread emulation (type: module Worker)
// Runs createEmulator + step loop, posts frames to main thread.
// Main thread keeps all DOM/canvas work; this thread never touches DOM.

let createEmulator;
let _queue = [];
let _ready = false;
self.onmessage = (e) => {
  if (!_ready) { _queue.push(e); return; }
  handleMessage(e);
};
async function _initEmu() {
  try { self.postMessage({ type: 'debug', msg: 'worker boot' }); } catch {}
  if (!globalThis.MUnicorn) {
    try { self.postMessage({ type: 'debug', msg: 'fetching unicorn_arm.js' }); } catch {}
    try {
      const r = await fetch('./unicorn_arm.js');
      const txt = await r.text();
      (0, eval)(txt);
    } catch (e) {
      try { const m = await import('./unicorn_arm.js'); globalThis.MUnicorn = m.MUnicorn || m.default || globalThis.MUnicorn; } catch {}
    }
  }
  if (!globalThis.MUnicorn) {
    try { self.postMessage({ type: 'error', message: 'MUnicorn not loaded in worker' }); } catch {}
  } else {
    try { self.postMessage({ type: 'debug', msg: 'MUnicorn ready' }); } catch {}
  }
  try { self.postMessage({ type: 'debug', msg: '_initEmu start' }); } catch {}
  const mod = await import('./emulator.js');
  createEmulator = mod.createEmulator;
  try { self.postMessage({ type: 'debug', msg: 'emulator imported' }); } catch {}
  _ready = true;
  for (const q of _queue.splice(0)) handleMessage(q);
  try { self.postMessage({ type: 'debug', msg: '_initEmu done' }); } catch {}
}
_initEmu();

let emu = null;
let running = false;
let runSteps = 0;
let totalInstBase = 0;
let autoBytes = [];
let canInjected = false;
const CAN_RAM_FLAG = 0x200000b8; // fallback when main thread sends no address
let canFlagAddr = CAN_RAM_FLAG;
let oledOff = null, lcdOff = null, oledCtx = null, lcdCtx = null;
let sabView = null;

// Pin activity buffer drained per frame
let pinBuf = [];

function post(type, extra = {}) {
  self.postMessage({ type, ...extra });
}

async function handleMessage(e) {
  try { self.postMessage({ type: 'debug', msg: 'handleMessage '+e.data.type }); } catch {}
  const msg = e.data;
  switch (msg.type) {
    case 'init': {
      if (emu) { try { emu.close(); } catch {} emu = null; }
      running = false;
      runSteps = 0;
      totalInstBase = 0;
      autoBytes = msg.autoBytes ? msg.autoBytes.slice() : [];
      canInjected = false;
      // Main thread resolves 'canRxArmed' from the ELF symbols (hardcoded
      // addresses go stale on rebuild); fall back for hex/bin firmware.
      canFlagAddr = msg.canFlagAddr || CAN_RAM_FLAG;
      pinBuf = [];
      try { self.postMessage({ type: 'debug', msg: 'createEmulator start' }); } catch {}
      try {
        emu = await createEmulator({
          chip: msg.chip,
          svd: msg.svd,
          firmware: msg.firmware,
          flash_size: 0x10000,
          ram_size: 0x5000,
          vector_table: 0x08000000,
          ext_devices: msg.ext_devices || {},
        });
        try { self.postMessage({ type: 'debug', msg: 'createEmulator done' }); } catch {}
        emu.onPinChange((port, pin, level) => pinBuf.push(port, pin, level));
        if (msg.symbols) emu.setSymbols(msg.symbols);
        const regs = emu.getRegisters();
        post('ready', { pc: regs.PC, sp: regs.SP });
      } catch (err) {
        try { self.postMessage({ type: 'debug', msg: 'createEmulator err: '+(err.message||String(err)) }); } catch {}
        post('error', { message: err.message || String(err) });
      }
      break;
    }
    case 'run': {
      if (!emu || running) return;
      running = true;
      // allow main thread to send uart/gpio while running
      loop();
      break;
    }
    case 'stop': {
      running = false;
      try { emu && emu.stop(); } catch {}
      post('stopped');
      break;
    }
    case 'uartRx': {
      if (emu) emu.uartRx(msg.byte & 0xFF);
      break;
    }
    case 'gpioSetInput': {
      if (emu) emu.gpioSetInput(msg.port, msg.pin, !!msg.value);
      break;
    }
    case 'setSymbols': {
      if (emu) emu.setSymbols(msg.text);
      const regs = emu.getRegisters();
      post('symbolsSet', { pc: regs.PC });
      break;
    }
    case 'initCanvas': {
      oledOff = msg.oled; lcdOff = msg.lcd;
      try { oledCtx = oledOff ? oledOff.getContext('2d') : null; } catch {}
      try { lcdCtx = lcdOff ? lcdOff.getContext('2d') : null; } catch {}
      break;
    }
    case 'initSAB': {
      try { sabView = new Int32Array(msg.sab); } catch {}
      break;
    }
  }
}

function loop() {
  if (!running || !emu) return;
  const t0 = performance.now();
  let lastResult = null;
  let stepsThisFrame = 0;
  try {
    do {
      lastResult = emu.step(20000);
      runSteps++;
      stepsThisFrame++;
      // autopilot: inject UART bytes and CAN when firmware waits
      if (autoBytes.length && emu.rxPending() === 0) {
        const b = autoBytes.shift();
        emu.uartRx(b);
      }
      if (!canInjected) {
        try {
          if (emu.memRead32(canFlagAddr) !== 0) {
            canInjected = !!emu.canInjectMessage(0x40006400, 0 << 21, 2, 0xDEAD, 0);
          }
        } catch {}
      }
      if (lastResult.stopped) { running = false; break; }
    } while (performance.now() - t0 < 80);
  } catch (err) {
    running = false;
    post('error', { message: err.message || String(err) });
    return;
  }

  const regs = emu.getRegisters();
  const uartOut = emu.getUartOutput();
  const pins = pinBuf.splice(0);
  if (sabView) {
    try { Atomics.store(sabView, 0, lastResult ? lastResult.instCount : 0); Atomics.store(sabView, 1, regs.PC); Atomics.store(sabView, 2, regs.SP); Atomics.store(sabView, 3, runSteps); if (typeof Atomics.notify === 'function') Atomics.notify(sabView, 0, 1); } catch {}
  }
  // OffscreenCanvas: render directly in worker if transferred, else send FB to main
  let oledFb = null, lcdFb = null, rgbDuty = null, buzz = null;
  if (oledCtx) {
    try {
      const fb = emu.i2cOledFb('I2C1', 0x3C);
      oledCtx.clearRect(0, 0, 128, 64);
      if (fb && fb.length) {
        const img = oledCtx.createImageData(128, 64);
        for (let x = 0; x < 128; x++) for (let page = 0; page < 8; page++) {
          const b = fb[page * 128 + x] || 0;
          for (let bit = 0; bit < 8; bit++) if (b & (1 << bit)) {
            const i = ((page * 8 + bit) * 128 + x) * 4;
            img.data[i] = 240; img.data[i + 1] = 240; img.data[i + 2] = 240; img.data[i + 3] = 255;
          }
        }
        oledCtx.putImageData(img, 0, 0);
      }
    } catch {}
  } else { try { oledFb = emu.i2cOledFb('I2C1', 0x3C); } catch {} }
  if (lcdCtx) {
    try {
      const fb = emu.lcdFb('SPI1');
      lcdCtx.clearRect(0, 0, 128, 64);
      if (fb && fb.length) {
        const img = lcdCtx.createImageData(128, 64);
        for (let i = 0; i < 8192 && i < fb.length; i++) { const v = fb[i] ? 240 : 24; img.data[i * 4] = v; img.data[i * 4 + 1] = v; img.data[i * 4 + 2] = v; img.data[i * 4 + 3] = 255; }
        lcdCtx.putImageData(img, 0, 0);
      }
    } catch {}
  } else { try { lcdFb = emu.lcdFb('SPI1'); } catch {} }
  try { rgbDuty = [emu.pwmDuty(0x40000000, 0), emu.pwmDuty(0x40000000, 1), emu.pwmDuty(0x40000000, 2)]; } catch {}
  try { buzz = !!emu.gpioReadOutput(1, 14); } catch {}
  // GPIO snapshot for the main-thread grid (worker path has no local emu):
  // 96 entries [odr0,idr0, odr1,idr1, ...] ports A-C. ~96 cheap crossings.
  let gpioSnap = null;
  try {
    gpioSnap = [];
    for (let port = 0; port < 3; port++) for (let pin = 0; pin < 16; pin++)
      gpioSnap.push(emu.gpioReadOutput(port, pin) ? 1 : 0, emu.gpioReadInput(port, pin) ? 1 : 0);
  } catch { gpioSnap = null; }

  post('frame', {
    instCount: lastResult ? lastResult.instCount : 0,
    runSteps,
    stepsThisFrame,
    pc: regs.PC,
    sp: regs.SP,
    uartOut,
    pins,
    oledFb: oledFb && oledFb.length ? oledFb.slice(0) : null,
    lcdFb: lcdFb && lcdFb.length ? lcdFb.slice(0) : null,
    rgbDuty,
    buzz,
    gpio: gpioSnap,
    stopped: lastResult ? lastResult.stopped : false,
  });

  if (lastResult && lastResult.stopped) {
    running = false;
    post('stopped');
    return;
  }
  // SAB fast path: when crossOriginIsolated, queueMicrotask is ~0ms vs setTimeout 4ms clamp
  if (typeof crossOriginIsolated !== 'undefined' && crossOriginIsolated) queueMicrotask(loop);
  else setTimeout(loop, 0);
}
