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
let uartAddr = 0; // USART base for uartRx (0 = default USART1)
let canInjected = false;
const CAN_RAM_FLAG = 0x200000b8; // fallback when main thread sends no address
let canFlagAddr = CAN_RAM_FLAG;
let oledOff = null, lcdOff = null, oledCtx = null, lcdCtx = null;

// Pin activity buffer drained per frame
let pinBuf = [];
// USB host taps (page CDC preset): when usbListen, each frame drains the
// VmEvent queue and forwards UsbIn (type 18) packets to the main thread.
// hostTraceListen (page otg_host preset) additionally forwards HostTx (20)
// / HostRx (21) plus the HCD RAM trace so the page can play the device.
let usbListen = false;
let hostTraceListen = false;
let usbAck = null;
// Control messages that arrive while `await createEmulator` is still
// pending (e.g. the otg_host preset's load-time attach) must not run
// against a null emu: worker messages interleave across the await, so a
// pre-ready otgHostAttach would throw, get swallowed by its try/catch,
// and the firmware would boot with no device (HCD trace 0xE0). Defer
// them and flush after successful init.
let pendingPreInit = [];

function post(type, extra = {}) {
  self.postMessage({ type, ...extra });
}

async function handleMessage(e) {
  try { self.postMessage({ type: 'debug', msg: 'handleMessage '+e.data.type }); } catch {}
  const msg = e.data;
  switch (msg.type) {
    case 'init': {
      if (emu) { try { emu.close(); } catch {} emu = null; }
      pendingPreInit = [];
      running = false;
      runSteps = 0;
      totalInstBase = 0;
      autoBytes = msg.autoBytes ? msg.autoBytes.slice() : [];
      uartAddr = msg.uartAddr || 0;
      canInjected = false;
      usbListen = !!msg.usbListen;
      hostTraceListen = !!msg.hostTrace;
      usbAck = null;
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
          vector_table: 0x08000000,
          ext_devices: msg.ext_devices || {},
        });
        try { self.postMessage({ type: 'debug', msg: 'createEmulator done' }); } catch {}
        emu.onPinChange((port, pin, level) => pinBuf.push(port, pin, level));
        if (msg.symbols) emu.setSymbols(msg.symbols);
        for (const q of pendingPreInit.splice(0)) {
          try { await handleMessage({ data: q }); } catch {}
        }
        const regs = emu.getRegisters();
        let idcode = null;
        try { idcode = emu.periphRead(0xE0042000, 4) >>> 0; } catch {}
        post('ready', { pc: regs.PC, sp: regs.SP, idcode });
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
      if (emu) {
        const b = msg.byte & 0xFF;
        if (uartAddr) emu.uartRxAddr(uartAddr, b);
        else emu.uartRx(b);
      }
      break;
    }
    case 'gpioSetInput': {
      if (emu) emu.gpioSetInput(msg.port, msg.pin, !!msg.value);
      break;
    }
    case 'usbSetup': {
      let a = false;
      try { a = !!emu.usbInjectSetup(msg.bytes); } catch {}
      usbAck = a;
      break;
    }
    case 'otgSetup': {
      let a = false;
      try { a = !!emu.otgInjectSetup(msg.bytes); } catch {}
      usbAck = a;
      break;
    }
    case 'otgReset': {
      let a = false;
      try { a = !!emu.otgBusReset(); } catch {}
      usbAck = a;
      
break;
    }
    case 'usbReset': {
      let a = false;
      try { a = !!emu.usbBusReset(); } catch {}
      usbAck = a;
      break;
    }
    case 'i2cStart': {
      let a = false;
      try { a = !!emu.i2cInjectStart(msg.channel, msg.addr, !!msg.isRead); } catch {}
      usbAck = a;
      break;
    }
    case 'i2cWrite': {
      let a = false;
      try { a = !!emu.i2cInjectWrite(msg.channel, msg.byte); } catch {}
      usbAck = a;
      break;
    }
    case 'i2cRead': {
      let a = -1;
      try { a = emu.i2cInjectRead(msg.channel); } catch {}
      usbAck = a;
      break;
    }
    case 'i2cStop': {
      let a = false;
      try { a = !!emu.i2cInjectStop(msg.channel); } catch {}
      usbAck = a;
      break;
    }
    case 'usbOut': {
      let a = false;
      try { a = !!emu.usbInjectOut(msg.ep, msg.bytes); } catch {}
      usbAck = a;
      break;
    }
    case 'otgOut': {
      let a = false;
      try { a = !!emu.otgInjectOut(msg.ep, msg.bytes); } catch {}
      usbAck = a;
      break;
    }
    case 'otgHostAttach': {
      if (!emu) { pendingPreInit.push(msg); break; }
      let a = false;
      try { a = !!emu.otgHostAttach(true); } catch {}
      usbAck = a;
      break;
    }
    case 'otgHostFeed': {
      if (!emu) { pendingPreInit.push(msg); break; }
      let a = false;
      try { a = !!emu.otgHostFeedIn(msg.ep, msg.bytes || [], false); } catch {}
      usbAck = a;
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
  // USB tap: forward UsbIn packets (skip other discriminants by length).
  // In host-firmware mode (hostTraceListen) also forward HostTx/HostRx
  // pairs plus the HCD RAM trace so the page can play the USB device.
  let usbIn = null, hostTx = null, hostRx = null, hostTrace = null;
  if (usbListen) {
    try {
      const flat = emu.drainEvents();
      const pkts = [];
      let i = 0;
      const skipLen = (t, j) => {
        switch (t) {
          case 1: return 3 + (flat[j+2]||0) + (flat[j+3]||0);
          case 14: case 15: return 12;
          case 16: return 4; case 17: return 6;
          case 18: return 3 + (flat[j+2]||0);
          case 19: return 3; // I2cAlert [ch, asserted]
          case 20: return 5 + (flat[j+4]||0);
          case 21: return 4;
          case 2: case 3: case 6: case 8: case 10: return 3;
          default: return 2; // 4,5,7,9,11,12,13: single-arg events
        }
      };
      while (i < flat.length) {
        const t = flat[i];
        if (t === 18) {
          const ep = flat[i+1], len = flat[i+2] || 0;
          pkts.push([ep, Array.from(flat.slice(i+3, i+3+len))]);
        } else if (t === 20 && hostTraceListen) {
          const ch = flat[i+1], ep = flat[i+2], su = flat[i+3], ln = flat[i+4] || 0;
          (hostTx || (hostTx = [])).push([ch, ep, su, Array.from(flat.slice(i+5, i+5+ln))]);
        } else if (t === 21 && hostTraceListen) {
          (hostRx || (hostRx = [])).push([flat[i+1], flat[i+2], flat[i+3] || 0]);
        }
        const adv = skipLen(t, i);
        if (adv <= 0) break;
        i += adv;
      }
      if (pkts.length) usbIn = pkts;
    } catch {}
  }
  const usbAckOut = usbAck; usbAck = null;
  // HCD trace for the host preset (otg_host.elf trace[] at 0x20000080):
  // trace_n + up to 8 slots so the page can show live HCD progress.
  if (hostTraceListen && emu) {
    try {
      const n = emu.memRead32(0x20000080) >>> 0;
      if (n > 0 && n <= 32) {
        const slots = [];
        for (let k = 0; k < n && k < 8; k++) slots.push(emu.memRead32(0x20000084 + k * 4) >>> 0);
        hostTrace = { n, slots };
      }
    } catch {}
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
    usbIn, usbAck: usbAckOut,
    hostTx, hostRx, hostTrace,
  });

  if (lastResult && lastResult.stopped) {
    running = false;
    post('stopped');
    return;
  }
  // Always yield via macrotask so control messages (stop/uartRx/
  // gpioSetInput) interleave with emulation while running.
  setTimeout(loop, 0);
}
