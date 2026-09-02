// site/worker.js — off-main-thread emulation (type: module Worker)
// Runs createEmulator + step loop, posts frames to main thread.
// Main thread keeps all DOM/canvas work; this thread never touches DOM.

import { createEmulator } from './emulator.js';

let emu = null;
let running = false;
let runSteps = 0;
let totalInstBase = 0;
let autoBytes = [];
let canInjected = false;
const CAN_RAM_FLAG = 0x200000b8;

// Pin activity buffer drained per frame
let pinBuf = [];

function post(type, extra = {}) {
  self.postMessage({ type, ...extra });
}

self.onmessage = async (e) => {
  const msg = e.data;
  switch (msg.type) {
    case 'init': {
      if (emu) { try { emu.close(); } catch {} emu = null; }
      running = false;
      runSteps = 0;
      totalInstBase = 0;
      autoBytes = msg.autoBytes ? msg.autoBytes.slice() : [];
      canInjected = false;
      pinBuf = [];
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
        emu.onPinChange((port, pin, level) => pinBuf.push(port, pin, level));
        if (msg.symbols) emu.setSymbols(msg.symbols);
        const regs = emu.getRegisters();
        post('ready', { pc: regs.PC, sp: regs.SP });
      } catch (err) {
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
  }
};

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
          if (emu.memRead32(CAN_RAM_FLAG) !== 0) {
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
  // light showcase snapshot for main thread (optional, main can ignore)
  let oledFb = null, lcdFb = null, segDigits = null, rgbDuty = null, buzz = null, wsInfo = null;
  try { oledFb = emu.i2cOledFb('I2C1', 0x3C); } catch {}
  try { lcdFb = emu.lcdFb('SPI1'); } catch {}
  try { rgbDuty = [emu.pwmDuty(0x40000000, 0), emu.pwmDuty(0x40000000, 1), emu.pwmDuty(0x40000000, 2)]; } catch {}
  try { buzz = !!emu.gpioReadOutput(1, 14); } catch {}

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
    stopped: lastResult ? lastResult.stopped : false,
  });

  if (lastResult && lastResult.stopped) {
    running = false;
    post('stopped');
    return;
  }
  // schedule next frame off main thread — use setTimeout(0) to yield to message queue
  setTimeout(loop, 0);
}
