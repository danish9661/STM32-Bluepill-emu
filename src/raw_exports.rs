//! Raw `extern "C"` FFI layer for the `wasm32-unknown-emscripten` target.
//!
//! This module mirrors the `#[wasm_bindgen]` exports in `lib.rs`, but with a C
//! ABI so the crate can be linked by `emcc` into a single wasm module together
//! with the Unicorn C engine (Path A / "mergedwasm"). The shipping
//! `wasm32-unknown-unknown` `cdylib` build does NOT compile this module
//! (gated off), so it is completely unaffected.
//!
//! Conventions (to be extended as more exports land):
//!   * Scalars (`u32/i32/bool/u8/u16/void`) map 1:1 to `extern "C"`.
//!   * `&str` / `&[u8]` / `Vec<_>` / `String` exports are TODO (need a
//!     caller-buffer / ptr+len+free convention) — see `docs/NEXT_PHASE.md §4`.

#![cfg(target_os = "emscripten")]

use std::sync::atomic::Ordering;

use crate::peripherals;
use crate::system;
use crate::{set_sys, sys};

#[no_mangle]
pub extern "C" fn init() {
    system::INSTRUCTION_COUNT.store(0, Ordering::Relaxed);
    peripherals::gpio::clear_pin_events();
    set_sys(system::WasmSystem::new());
}

#[no_mangle]
pub extern "C" fn init_svd(xml_ptr: *const u8, xml_len: u32) {
    let xml = read_str(xml_ptr, xml_len).to_string();
    system::INSTRUCTION_COUNT.store(0, Ordering::Relaxed);
    peripherals::gpio::clear_pin_events();
    set_sys(crate::system::WasmSystem::new_svd(&xml));
}

#[no_mangle]
pub extern "C" fn reset_ext_devices() {
    let mut ext = system::get_ext_devices().lock().unwrap();
    ext.spi_flashes.clear();
    ext.i2c_eeproms.clear();
    ext.usart_probes.clear();
    ext.lcds.clear();
    ext.touchscreens.clear();
    ext.displays.clear();
    ext.i2c_oleds.clear();
    ext.fsmc_nors.clear();
    drop(ext);
    system::get_software_spi_configs().lock().unwrap().clear();
}

#[no_mangle]
pub extern "C" fn periph_read(addr: u32, width: u32) -> u32 {
    sys().p.read(&*sys(), addr, width as u8)
}

#[no_mangle]
pub extern "C" fn periph_write(addr: u32, width: u32, value: u32) {
    sys().p.write(&*sys(), addr, width as u8, value);
}

#[no_mangle]
pub extern "C" fn tick() {
    system::INSTRUCTION_COUNT.fetch_add(1, Ordering::Relaxed);
    sys().tick();
}

#[no_mangle]
pub extern "C" fn step(primask: u32, basepri: u32) -> u32 {
    system::INTR_MASK_PRIMASK.store(primask, Ordering::Relaxed);
    system::INTR_MASK_BASEPRI.store(basepri, Ordering::Relaxed);
    system::INSTRUCTION_COUNT.fetch_add(1, Ordering::Relaxed);
    sys().intr.borrow_mut().reset_budget();
    sys().tick();
    if is_watchdog_reset_requested() {
        return 1;
    }
    if sys().pending_dma_count() > 0 {
        return 2;
    }
    if sys().p.nvic.borrow().has_pending_masked(primask, basepri) {
        return 3;
    }
    0
}

#[no_mangle]
pub extern "C" fn step_batch(count: u32) -> u32 {
    let s = sys();
    system::INSTRUCTION_COUNT.fetch_add(count as u64, Ordering::Relaxed);
    s.intr.borrow_mut().reset_budget();
    s.tick();
    if is_watchdog_reset_requested() {
        1
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn process_batch(count: u32) -> u32 {
    let s = sys();
    system::INSTRUCTION_COUNT.fetch_add(count as u64, Ordering::Relaxed);
    s.intr.borrow_mut().reset_budget();
    s.tick();
    if is_watchdog_reset_requested() {
        return 0x8000_0000;
    }
    let primask = system::INTR_MASK_PRIMASK.load(Ordering::Relaxed);
    let basepri = system::INTR_MASK_BASEPRI.load(Ordering::Relaxed);
    if s.p.nvic.borrow().has_pending_masked(primask, basepri) {
        0x4000_0000
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn has_pending_interrupt() -> bool {
    let primask = system::INTR_MASK_PRIMASK.load(Ordering::Relaxed);
    let basepri = system::INTR_MASK_BASEPRI.load(Ordering::Relaxed);
    sys().p.nvic.borrow().has_pending_masked(primask, basepri)
}

#[no_mangle]
pub extern "C" fn get_next_pending_interrupt() -> i32 {
    sys().p.nvic.borrow_mut().get_next_pending_intr().unwrap_or(-255)
}

#[no_mangle]
pub extern "C" fn set_intr_masks(primask: u32, basepri: u32) {
    system::INTR_MASK_PRIMASK.store(primask, Ordering::Relaxed);
    system::INTR_MASK_BASEPRI.store(basepri, Ordering::Relaxed);
}

#[no_mangle]
pub extern "C" fn clear_current_interrupt() {
    sys().p.nvic.borrow_mut().clear_current_interrupt();
}

#[no_mangle]
pub extern "C" fn finish_interrupt(irq: i32) {
    let mut nvic = sys().p.nvic.borrow_mut();
    nvic.clear_current_interrupt();
    if irq == -1 {
        while nvic.systick_take() {}
    }
}

#[no_mangle]
pub extern "C" fn nvic_systick_take() -> bool {
    sys().p.nvic.borrow_mut().systick_take()
}

#[no_mangle]
pub extern "C" fn dma_get_pending_count() -> u32 {
    sys().pending_dma_count() as u32
}

#[no_mangle]
pub extern "C" fn dma_set_completed(stream_idx: u32, success: bool) {
    sys().mark_dma_completed(stream_idx as usize, success);
}

#[no_mangle]
pub extern "C" fn dma_set_completed_many(bits: u32) {
    for stream in 0..8 {
        if bits & (1 << stream) != 0 {
            sys().mark_dma_completed(stream, true);
        }
    }
}

#[no_mangle]
pub extern "C" fn gpio_read_output(port: u32, pin: u32) -> bool {
    sys().p.gpio.borrow_mut().read_output_pin(&*sys(), port as u8, pin as u8)
}

#[no_mangle]
pub extern "C" fn gpio_set_slew(inst: u32) {
    peripherals::gpio::set_gpio_slew(inst);
}

#[no_mangle]
pub extern "C" fn gpio_set_input(port: u32, pin: u32, value: bool) {
    let s = sys();
    s.p.gpio.borrow_mut().set_input_pin(&s, port as u8, pin as u8, value);
}

#[no_mangle]
pub extern "C" fn gpio_read_input(port: u32, pin: u32) -> bool {
    sys().p.gpio.borrow().read_input_pin(port as u8, pin as u8)
}

#[no_mangle]
pub extern "C" fn gpio_set_analog(port: u32, pin: u32, level: u32) {
    sys().p.gpio.borrow_mut().set_analog(port as u8, pin as u8, level as u16);
}

#[no_mangle]
pub extern "C" fn adc_set_rc_tau(cycles: u32) {
    peripherals::adc::set_adc_rc_tau(cycles.min(0xFFFF) as u16);
}

#[no_mangle]
pub extern "C" fn pwm_duty(addr: u32, channel: u32) -> u32 {
    sys().p.pwm_duty(addr, channel)
}

#[no_mangle]
pub extern "C" fn is_watchdog_reset_requested() -> bool {
    system::is_watchdog_reset_requested()
}

#[no_mangle]
pub extern "C" fn uart_rx_byte(addr: u32, byte: u8) -> bool {
    sys().p.rx_byte(&*sys(), addr, byte)
}

#[no_mangle]
pub extern "C" fn uart_rx_pending(addr: u32) -> u32 {
    sys().p.rx_pending(addr)
}

#[no_mangle]
pub extern "C" fn can_inject_message(addr: u32, tir: u32, tdtr: u32, tdlr: u32, tdhr: u32) -> bool {
    sys().p.can_inject_message(&*sys(), addr, tir, tdtr, tdlr, tdhr)
}

#[no_mangle]
pub extern "C" fn raise_fault(kind: u32, addr: u32) {
    sys().p.raise_fault(&*sys(), kind, addr);
}

#[no_mangle]
pub extern "C" fn adc_set_sim_value(val: u16) {
    peripherals::adc::set_adc_value(val);
}

// --- interrupts.rs scalars ---

#[no_mangle]
pub extern "C" fn intr_next() -> i32 {
    let s = sys();
    s.intr.borrow_mut().next(s)
}

#[no_mangle]
pub extern "C" fn intr_svc_depth() -> u32 {
    sys().intr.borrow().svc_stack.len() as u32
}

// --- string / Vec / &[u8] / String exports (Path A integration) ---
//
// Calling convention:
//   * `&str` / `&[u8]` inputs arrive as `(ptr: *const u8, len: u32)`; the
//     caller (JS) owns the buffer and frees it after the call.
//   * `Option<String>` inputs arrive as `(ptr: *const u8, len: u32)` where
//     `len == 0` means `None`.
//   * `Vec<u8>` / `Vec<u32>` returns are leaked (`Vec::leak`) and returned via
//     two out-pointers `(out_ptr: *mut u32, out_len: *mut u32)` carrying
//     `(heap_ptr, len)`. The caller reads the bytes/words then frees `heap_ptr`
//     with the module's `_free`.

fn read_str(ptr: *const u8, len: u32) -> &'static str {
    if ptr.is_null() || len == 0 {
        ""
    } else {
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len as usize)) }
    }
}

fn read_bytes(ptr: *const u8, len: u32) -> Vec<u8> {
    if ptr.is_null() || len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(ptr, len as usize).to_vec() }
    }
}

fn opt_string(ptr: *const u8, len: u32) -> Option<String> {
    if ptr.is_null() || len == 0 {
        None
    } else {
        Some(read_str(ptr, len).to_string())
    }
}

fn return_leaked_u8(v: Vec<u8>, out_ptr: *mut u32, out_len: *mut u32) {
    let v = v.leak();
    unsafe {
        *out_ptr = v.as_ptr() as u32;
        *out_len = v.len() as u32;
    }
}

fn return_leaked_u32(v: Vec<u32>, out_ptr: *mut u32, out_len: *mut u32) {
    let v = v.leak();
    unsafe {
        *out_ptr = v.as_ptr() as u32;
        *out_len = v.len() as u32;
    }
}

#[no_mangle]
pub extern "C" fn intr_svc_enter(
    r0: u32, r1: u32, r2: u32, r3: u32, r12: u32,
    lr: u32, pc: u32, xpsr: u32, sp: u32,
    out_ptr: *mut u32, out_len: *mut u32,
) {
    use crate::interrupts::SvcFrame;
    let mut intr = crate::sys().intr.borrow_mut();
    if intr.svc_stack.len() >= 8 {
        return_leaked_u8(Vec::new(), out_ptr, out_len);
        return;
    }
    intr.svc_stack.push(SvcFrame { sp, r0, r1, r2, r3, r12, lr, pc, xpsr });
    let mut frame = Vec::with_capacity(32);
    for v in [xpsr, pc, lr, r12, r3, r2, r1, r0] {
        frame.extend_from_slice(&v.to_le_bytes());
    }
    return_leaked_u8(frame, out_ptr, out_len);
}

#[no_mangle]
pub extern "C" fn intr_svc_leave(out_ptr: *mut u32, out_len: *mut u32) {
    let mut intr = crate::sys().intr.borrow_mut();
    let v = match intr.svc_stack.pop() {
        Some(f) => vec![f.r0, f.r1, f.r2, f.r3, f.r12, f.lr, f.pc, f.sp, f.xpsr],
        None => Vec::new(),
    };
    return_leaked_u32(v, out_ptr, out_len);
}

#[no_mangle]
pub extern "C" fn dma_pump_all(out_ptr: *mut u32, out_len: *mut u32) {
    use crate::DmaDir;
    let sys = crate::sys();
    sys.dma_absorb_reset();
    let mut plan: Vec<u32> = Vec::new();
    let mut done_bits = 0u32;
    for t in sys.take_pending_dma_transfers() {
        done_bits |= 1 << t.stream_idx;
        if t.direction == DmaDir::MemCopy || !t.peripheral {
            plan.extend([0, t.src, t.dst, t.size as u32]);
        } else if t.direction == DmaDir::Read {
            let off = sys.dma_absorb_store(t.peri_addr, t.size);
            plan.extend([1, t.dst, t.size as u32, off as u32]);
        } else {
            plan.extend([2, t.src, t.size as u32, t.peri_addr]);
        }
    }
    if done_bits != 0 {
        plan.extend([3, done_bits, 0, 0]);
    }
    return_leaked_u32(plan, out_ptr, out_len);
}

#[no_mangle]
pub extern "C" fn dma_take_absorbed(offset: u32, len: u32, out_ptr: *mut u32, out_len: *mut u32) {
    let v = crate::sys().dma_absorb_take(offset as usize, len as usize);
    return_leaked_u8(v, out_ptr, out_len);
}

#[no_mangle]
pub extern "C" fn dma_push_periph(addr: u32, data_ptr: *const u8, data_len: u32) {
    let data = read_bytes(data_ptr, data_len);
    let mut j = 0usize;
    while j < data.len() {
        let chunk = std::cmp::min(4, data.len() - j);
        let mut val = 0u32;
        for k in 0..chunk {
            val |= (data[j + k] as u32) << (k * 8);
        }
        crate::sys().p.write(&*crate::sys(), addr, chunk as u8, val);
        j += chunk;
    }
}

#[no_mangle]
pub extern "C" fn dma_absorb_periph(addr: u32, size: u32, out_ptr: *mut u32, out_len: *mut u32) {
    let mut out = Vec::with_capacity(size as usize);
    let mut j = 0u32;
    while j < size {
        let chunk = std::cmp::min(4, size - j);
        let val = crate::sys().p.read(&*crate::sys(), addr, chunk as u8);
        for k in 0..chunk {
            out.push(((val >> (k * 8)) & 0xFF) as u8);
        }
        j += chunk;
    }
    return_leaked_u8(out, out_ptr, out_len);
}

#[no_mangle]
pub extern "C" fn get_uart_output(out_ptr: *mut u32, out_len: *mut u32) {
    let s = std::mem::take(&mut *crate::system::get_uart_output().lock().unwrap());
    return_leaked_u8(s.into_bytes(), out_ptr, out_len);
}

#[no_mangle]
pub extern "C" fn add_i2c_eeprom(
    periph_ptr: *const u8, periph_len: u32, address: u8,
    data_ptr: *const u8, data_len: u32,
) {
    use crate::ext_devices::i2c_eeprom::{I2cEeprom, I2cEepromConfig};
    let peripheral = read_str(periph_ptr, periph_len).to_string();
    let data = read_bytes(data_ptr, data_len);
    let config = I2cEepromConfig {
        peripheral,
        address,
        content: data.clone(),
        size: data.len(),
    };
    let eeprom = I2cEeprom::new(config);
    crate::system::get_ext_devices().lock().unwrap().i2c_eeproms
        .push(std::rc::Rc::new(std::cell::RefCell::new(eeprom)));
}

#[no_mangle]
pub extern "C" fn add_spi_flash(
    periph_ptr: *const u8, periph_len: u32, jedec_id: u32,
    data_ptr: *const u8, data_len: u32,
    cs_ptr: *const u8, cs_len: u32,
) {
    use crate::ext_devices::spi_flash::{SpiFlash, SpiFlashConfig};
    let peripheral = read_str(periph_ptr, periph_len).to_string();
    let data = read_bytes(data_ptr, data_len);
    let config = SpiFlashConfig {
        peripheral,
        jedec_id,
        content: data.clone(),
        size: data.len(),
        cs: opt_string(cs_ptr, cs_len),
    };
    let flash = SpiFlash::new(config);
    crate::system::get_ext_devices().lock().unwrap().spi_flashes
        .push(std::rc::Rc::new(std::cell::RefCell::new(flash)));
}

#[no_mangle]
pub extern "C" fn add_lcd(
    periph_ptr: *const u8, periph_len: u32,
    cs_ptr: *const u8, cs_len: u32,
) {
    use crate::ext_devices::lcd::{Lcd, LcdConfig};
    let peripheral = read_str(periph_ptr, periph_len).to_string();
    let config = LcdConfig {
        peripheral,
        framebuffer: String::new(),
        cs: opt_string(cs_ptr, cs_len),
    };
    let lcd = Lcd::new(config);
    crate::system::get_ext_devices().lock().unwrap().lcds
        .push(std::rc::Rc::new(std::cell::RefCell::new(lcd)));
}

#[no_mangle]
pub extern "C" fn add_i2c_oled(
    periph_ptr: *const u8, periph_len: u32,
    address: u8, width: u16, height: u16,
) {
    use crate::ext_devices::i2c_oled::{I2cOled, I2cOledConfig};
    let peripheral = read_str(periph_ptr, periph_len).to_string();
    let config = I2cOledConfig {
        peripheral,
        address,
        width,
        height,
    };
    let oled = I2cOled::new(config);
    crate::system::get_ext_devices().lock().unwrap().i2c_oleds
        .push(std::rc::Rc::new(std::cell::RefCell::new(oled)));
}

#[no_mangle]
pub extern "C" fn add_touchscreen(
    periph_ptr: *const u8, periph_len: u32,
    td_ptr: *const u8, td_len: u32,
    cs_ptr: *const u8, cs_len: u32,
) {
    use crate::ext_devices::touchscreen::{Touchscreen, TouchscreenConfig};
    let peripheral = read_str(periph_ptr, periph_len).to_string();
    let config = TouchscreenConfig {
        peripheral,
        framebuffer: String::new(),
        flip_x: None,
        flip_y: None,
        swap_x_y: None,
        touch_detected_pin: opt_string(td_ptr, td_len),
        scale_down: None,
        cs: opt_string(cs_ptr, cs_len),
    };
    let ts = Touchscreen::new(config);
    crate::system::get_ext_devices().lock().unwrap().touchscreens
        .push(std::rc::Rc::new(std::cell::RefCell::new(ts)));
}
