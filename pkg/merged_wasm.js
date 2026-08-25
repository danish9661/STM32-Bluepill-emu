// pkg/merged_wasm.js
//
// Drop-in replacement for `pkg/stm32_bluepill_wasm.js` (the wasm-bindgen glue),
// backed by the SINGLE merged wasm module (Unicorn C engine + Rust peripherals
// linked together by emcc — Path A / "mergedwasm").
//
// It exposes the SAME module-level exports the unit tests (tests/test_all.mjs)
// and the rest of the codebase expect from the wasm-bindgen module, so swap
// `stm32_bluepill_wasm.js` for `merged_wasm.js` and the peripheral layer keeps
// working unchanged.
//
// The raw merged module (merged_unicorn_arm.cjs) is a single emscripten wasm
// that contains BOTH the Unicorn engine and the Rust peripherals, linked by a
// raw `#[no_mangle]` C ABI (see src/raw_exports.rs). There is no js_sys on
// emscripten, so JS-peripheral ranges are intercepted in this wrapper.

import { createRequire } from 'module';
import { readFileSync } from 'fs';
import { fileURLToPath } from 'url';

const require = createRequire(import.meta.url);
const MUnicorn = require(fileURLToPath(new URL('./merged_unicorn_arm.cjs', import.meta.url)));
const u = await MUnicorn({});

// --- buffer / string / Vec helpers (caller owns malloc'd buffers) ---

const readU32Array = (ptr, len) => {
  const a = new Array(len);
  for (let i = 0; i < len; i++) {
    let v = u.getValue(ptr + i * 4, 'i32');
    if (v < 0) v += 4294967296;
    a[i] = v;
  }
  return a;
};
const readU8Array = (ptr, len) => {
  const a = new Uint8Array(len);
  for (let i = 0; i < len; i++) a[i] = u.getValue(ptr + i, 'i8');
  return a;
};
const strParam = (s) => {
  const b = new TextEncoder().encode(s);
  const p = u._malloc(b.length);
  for (let i = 0; i < b.length; i++) u.setValue(p + i, b[i], 'i8');
  return { ptr: p, len: b.length };
};
const optParam = (s) => (s == null ? { ptr: 0, len: 0 } : strParam(s));
const bytesParam = (d) => {
  const p = u._malloc(d.length);
  for (let i = 0; i < d.length; i++) u.setValue(p + i, d[i], 'i8');
  return { ptr: p, len: d.length };
};
const outVecU32 = (fn) => {
  const o = u._malloc(8);
  fn(o, o + 4);
  let ptr = u.getValue(o, 'i32');
  let len = u.getValue(o + 4, 'i32');
  if (ptr < 0) ptr += 4294967296;
  if (len < 0) len += 4294967296;
  u._free(o);
  const v = readU32Array(ptr, len);
  if (len > 0) u._free(ptr);
  return v;
};
const outVecU8 = (fn) => {
  const o = u._malloc(8);
  fn(o, o + 4);
  let ptr = u.getValue(o, 'i32');
  const len = u.getValue(o + 4, 'i32');
  if (ptr < 0) ptr += 4294967296;
  u._free(o);
  const v = readU8Array(ptr, len);
  if (len > 0) u._free(ptr);
  return v;
};

// --- JS-peripheral registry (no js_sys on emscripten; intercepted in JS) ---
const jsPeripherals = [];
const jsPeriphFor = (addr) => {
  for (const jp of jsPeripherals) {
    if (addr >= jp.base && addr < jp.base + jp.size) return jp;
  }
  return null;
};

// --- peripheral read/write (JS-peripheral override) ---
const periph_read = (a, s) => {
  const jp = jsPeriphFor(a);
  if (jp) return jp.read(a, s) >>> 0;
  return u._periph_read(a, s) >>> 0;
};
const periph_write = (a, s, v) => {
  const jp = jsPeriphFor(a);
  if (jp) { jp.write(a, v, s); return; }
  u._periph_write(a, s, v);
};

// --- exported API (mirrors stm32_bluepill_wasm.js) ---

export const reset_ext_devices = () => u._reset_ext_devices();
export const init = () => { jsPeripherals.length = 0; u._init(); };
export const init_svd = (xml) => { const p = strParam(xml); u._init_svd(p.ptr, p.len); u._free(p.ptr); };
export const periph_read_export = periph_read;
export const periph_write_export = periph_write;
export const tick = () => u._tick();
export const step_batch = (c) => u._step_batch(c);
export const process_batch = (c) => u._process_batch(c);
export const has_pending_interrupt = () => !!u._has_pending_interrupt();
export const get_next_pending_interrupt = () => u._get_next_pending_interrupt();
export const set_intr_masks = (p, b) => u._set_intr_masks(p, b);
export const clear_current_interrupt = () => u._clear_current_interrupt();
export const finish_interrupt = (irq) => u._finish_interrupt(irq);
export const nvic_systick_take = () => !!u._nvic_systick_take();
export const dma_get_pending_count = () => u._dma_get_pending_count();
export const dma_set_completed = (s, ok) => u._dma_set_completed(s, ok);
export const dma_set_completed_many = (b) => u._dma_set_completed_many(b);
export const gpio_read_output = (p, pin) => !!u._gpio_read_output(p, pin);
export const gpio_set_slew = (i) => u._gpio_set_slew(i);
export const gpio_set_input = (p, pin, v) => u._gpio_set_input(p, pin, v);
export const gpio_read_input = (p, pin) => !!u._gpio_read_input(p, pin);
export const gpio_set_analog = (p, pin, l) => u._gpio_set_analog(p, pin, l);
export const gpio_take_pin_events = () => outVecU32((o, o2) => u._gpio_take_pin_events(o, o2));
export const adc_set_rc_tau = (c) => u._adc_set_rc_tau(c);
export const pwm_duty = (a, c) => u._pwm_duty(a, c);
export const is_watchdog_reset_requested = () => !!u._is_watchdog_reset_requested();
export const uart_rx_byte = (a, b) => !!u._uart_rx_byte(a, b);
export const uart_rx_pending = (a) => u._uart_rx_pending(a);
export const can_inject_message = (a, t, td, tdl, tdh) => !!u._can_inject_message(a, t, td, tdl, tdh);
export const raise_fault = (k, a) => u._raise_fault(k, a);
export const adc_set_sim_value = (v) => u._adc_set_sim_value(v);
export const intr_next = () => u._intr_next();
export const intr_svc_depth = () => u._intr_svc_depth();
export const intr_svc_enter = (r0, r1, r2, r3, r12, lr, pc, xpsr, sp) =>
  outVecU8((o, o2) => u._intr_svc_enter(r0, r1, r2, r3, r12, lr, pc, xpsr, sp, o, o2));
export const intr_svc_leave = () =>
  outVecU32((o, o2) => u._intr_svc_leave(o, o2));
export const dma_pump_all = () => outVecU32((o, o2) => u._dma_pump_all(o, o2));
export const dma_take_absorbed = (off, len) =>
  outVecU8((o, o2) => u._dma_take_absorbed(off, len, o, o2));
export const dma_push_periph = (addr, data) => {
  const p = bytesParam(data);
  u._dma_push_periph(addr, p.ptr, p.len);
  u._free(p.ptr);
};
export const dma_absorb_periph = (addr, size) =>
  outVecU8((o, o2) => u._dma_absorb_periph(addr, size, o, o2));
export const get_uart_output = () => {
  const b = outVecU8((o, o2) => u._get_uart_output(o, o2));
  return String.fromCharCode.apply(null, b);
};

// --- external devices ---
export const add_i2c_eeprom = (peripheral, address, data) => {
  const p = strParam(peripheral);
  const d = bytesParam(data);
  u._add_i2c_eeprom(p.ptr, p.len, address, d.ptr, d.len);
  u._free(p.ptr); u._free(d.ptr);
};
export const add_spi_flash = (peripheral, jedec, data, cs) => {
  const p = strParam(peripheral);
  const d = bytesParam(data);
  const c = optParam(cs);
  u._add_spi_flash(p.ptr, p.len, jedec, d.ptr, d.len, c.ptr, c.len);
  u._free(p.ptr); u._free(d.ptr);
  if (c.ptr) u._free(c.ptr);
};
export const add_touchscreen = (peripheral, touch_detected_pin, cs) => {
  const p = strParam(peripheral);
  const t = optParam(touch_detected_pin);
  const c = optParam(cs);
  u._add_touchscreen(p.ptr, p.len, t.ptr, t.len, c.ptr, c.len);
  u._free(p.ptr);
  if (t.ptr) u._free(t.ptr);
  if (c.ptr) u._free(c.ptr);
};
export const add_lcd = (peripheral, cs) => {
  const p = strParam(peripheral);
  const c = optParam(cs);
  u._add_lcd(p.ptr, p.len, c.ptr, c.len);
  u._free(p.ptr);
  if (c.ptr) u._free(c.ptr);
};
export const add_i2c_oled = (peripheral, address, width, height) => {
  const p = strParam(peripheral);
  u._add_i2c_oled(p.ptr, p.len, address, width, height);
  u._free(p.ptr);
};
export const add_software_spi = (name, cs, clk, miso, mosi) => {
  const n = strParam(name);
  const c = optParam(cs);
  const k = strParam(clk);
  const mi = strParam(miso);
  const mo = strParam(mosi);
  u._add_software_spi(n.ptr, n.len, c.ptr, c.len, k.ptr, k.len, mi.ptr, mi.len, mo.ptr, mo.len);
  u._free(n.ptr);
  if (c.ptr) u._free(c.ptr);
  u._free(k.ptr); u._free(mi.ptr); u._free(mo.ptr);
};
export const add_fsmc_bank = (name, data) => {
  const p = strParam(name);
  const d = bytesParam(data);
  u._add_fsmc_bank(p.ptr, p.len, d.ptr, d.len);
  u._free(p.ptr);
  u._free(d.ptr);
};

// --- JS peripherals (no js_sys) ---
export const register_js_peripheral = (base, size, read, write) => {
  jsPeripherals.push({ base, size, read, write });
  return true;
};

// --- framebuffers / touch ---
export const i2c_oled_fb = (peripheral, address) => {
  const p = strParam(peripheral);
  const v = outVecU8((o, o2) => u._i2c_oled_fb(p.ptr, p.len, address, o, o2));
  u._free(p.ptr);
  return v;
};
export const lcd_fb = (peripheral) => {
  const p = strParam(peripheral);
  const v = outVecU8((o, o2) => u._lcd_fb(p.ptr, p.len, o, o2));
  u._free(p.ptr);
  return v;
};
export const touchscreen_set_touch = (peripheral, x, y, pressure) => {
  const p = strParam(peripheral);
  u._touchscreen_set_touch(p.ptr, p.len, x, y, pressure);
  u._free(p.ptr);
};

// Self-contained wasm: nothing to initialize here (already loaded above).
export const initSync = () => {};
