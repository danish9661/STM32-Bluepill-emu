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

// TODO (next step): string/Vec/&[u8]/String exports:
//   init_svd, register_js_peripheral, dma_absorb_periph, dma_push_periph,
//   dma_pump_all, dma_take_absorbed, dma_get_pending, dma_get_all_pending,
//   gpio_take_pin_events, get_uart_output, add_spi_flash, add_i2c_eeprom,
//   add_fsmc_bank, add_software_spi, add_lcd, add_i2c_oled, add_touchscreen,
//   touchscreen_set_touch, lcd_fb, i2c_oled_fb, i2c_oled_writes,
//   intr_svc_enter, intr_svc_leave
