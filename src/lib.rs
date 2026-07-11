use std::sync::atomic::Ordering;
use wasm_bindgen::prelude::*;

mod system;
pub mod peripherals;
pub mod ext_devices;

use system::WasmSystem;

// We use static mut since WASM is single-threaded — this allows re-initialization.
static mut SYS: Option<WasmSystem> = None;

#[allow(static_mut_refs)]
fn sys() -> &'static WasmSystem {
    unsafe { SYS.as_ref().expect("WasmSystem not initialized") }
}

/// Initialize the emulator with hardcoded peripheral map.
/// Must be called after adding all ext devices (add_spi_flash, add_i2c_eeprom).
/// Can be called multiple times to reset emulator state.
#[wasm_bindgen]
pub fn init() {
    console_error_panic_hook::set_once();
    system::INSTRUCTION_COUNT.store(0, Ordering::Relaxed);
    unsafe { SYS = Some(WasmSystem::new()); }
}

/// Initialize the emulator from an SVD XML string (e.g., STM32F407.svd).
/// Must be called after adding all ext devices (add_spi_flash, add_i2c_eeprom).
#[wasm_bindgen]
pub fn init_svd(svd_xml: &str) {
    console_error_panic_hook::set_once();
    system::INSTRUCTION_COUNT.store(0, Ordering::Relaxed);
    unsafe { SYS = Some(WasmSystem::new_svd(svd_xml)); }
}

#[wasm_bindgen]
pub fn periph_read(addr: u32, width: u32) -> u32 {
    sys().p.read(&*sys(), addr, width as u8)
}

#[wasm_bindgen]
pub fn periph_write(addr: u32, width: u32, value: u32) {
    sys().p.write(&*sys(), addr, width as u8, value);
}

#[wasm_bindgen]
pub fn tick() {
    use std::sync::atomic::Ordering;
    system::INSTRUCTION_COUNT.fetch_add(1, Ordering::Relaxed);
    sys().tick();
}

/// Check if any interrupt is pending (non-consuming).
#[wasm_bindgen]
pub fn has_pending_interrupt() -> bool {
    sys().p.nvic.borrow().has_pending()
}

#[wasm_bindgen]
pub fn get_next_pending_interrupt() -> i32 {
    sys().p.nvic.borrow_mut().get_and_clear_next_intr_pending()
        .unwrap_or(-255)
}

#[wasm_bindgen]
pub fn dma_get_pending_count() -> u32 {
    sys().pending_dma_count() as u32
}

#[wasm_bindgen]
pub fn dma_get_pending(index: u32) -> Vec<u32> {
    sys().take_pending_dma_transfer(index as usize)
        .map(|t| t.to_u32_vec())
        .unwrap_or_default()
}

#[wasm_bindgen]
pub fn dma_set_completed(stream_idx: u32, success: bool) {
    sys().mark_dma_completed(stream_idx as usize, success);
}

#[wasm_bindgen]
pub fn gpio_read_output(port: u32, pin: u32) -> bool {
    sys().p.gpio.borrow().read_output_pin(port as u8, pin as u8)
}

#[wasm_bindgen]
pub fn gpio_set_input(port: u32, pin: u32, value: bool) {
    sys().p.gpio.borrow_mut().set_input_pin(port as u8, pin as u8, value);
}

#[wasm_bindgen]
pub fn gpio_read_input(port: u32, pin: u32) -> bool {
    sys().p.gpio.borrow().read_input_pin(port as u8, pin as u8)
}

#[wasm_bindgen]
pub fn is_watchdog_reset_requested() -> bool {
    system::is_watchdog_reset_requested()
}

/// Inject a received byte into the UART at the given peripheral base address.
/// Returns true if a peripheral was found at that address.
#[wasm_bindgen]
pub fn uart_rx_byte(addr: u32, byte: u8) -> bool {
    sys().p.rx_byte(&*sys(), addr, byte)
}

/// Inject a CAN message into the CAN peripheral at the given address.
/// Returns true if the message was accepted (matched a filter and placed in a FIFO).
#[wasm_bindgen]
pub fn can_inject_message(addr: u32, tir: u32, tdtr: u32, tdlr: u32, tdhr: u32) -> bool {
    sys().p.can_inject_message(&*sys(), addr, tir, tdtr, tdlr, tdhr)
}

/// Collect UART output since last call.
#[wasm_bindgen]
pub fn get_uart_output() -> String {
    use std::mem::take;
    take(&mut *system::get_uart_output().lock().unwrap())
}

/// Add an SPI flash device. Must be called before init().
#[wasm_bindgen]
pub fn add_spi_flash(peripheral: &str, jedec_id: u32, data: &[u8], cs: Option<String>) {
    use crate::ext_devices::spi_flash::{SpiFlash, SpiFlashConfig};
    let config = SpiFlashConfig {
        peripheral: peripheral.to_string(),
        jedec_id,
        content: data.to_vec(),
        size: data.len(),
        cs,
    };
    let flash = SpiFlash::new(config);
    system::get_ext_devices().lock().unwrap().spi_flashes
        .push(std::rc::Rc::new(std::cell::RefCell::new(flash)));
}

/// Add an I2C EEPROM device. Must be called before init().
#[wasm_bindgen]
pub fn add_i2c_eeprom(peripheral: &str, address: u8, data: &[u8]) {
    use crate::ext_devices::i2c_eeprom::{I2cEeprom, I2cEepromConfig};
    let config = I2cEepromConfig {
        peripheral: peripheral.to_string(),
        address,
        content: data.to_vec(),
        size: data.len(),
    };
    let eeprom = I2cEeprom::new(config);
    system::get_ext_devices().lock().unwrap().i2c_eeproms
        .push(std::rc::Rc::new(std::cell::RefCell::new(eeprom)));
}

#[wasm_bindgen]
pub fn adc_set_sim_value(val: u16) {
    peripherals::adc::set_adc_value(val);
}

/// Register a software SPI device. Must be called before init().
#[wasm_bindgen]
pub fn add_software_spi(name: &str, cs: Option<String>, clk: &str, miso: &str, mosi: &str) {
    system::get_software_spi_configs().lock().unwrap()
        .push((name.to_string(), cs, clk.to_string(), miso.to_string(), mosi.to_string()));
}

/// Add an SPI LCD display device (e.g. ST7789, ILI9341). Must be called before init().
#[wasm_bindgen]
pub fn add_lcd(peripheral: &str, cs: Option<String>) {
    use crate::ext_devices::lcd::{Lcd, LcdConfig};
    let config = LcdConfig {
        peripheral: peripheral.to_string(),
        framebuffer: String::new(),
        cs,
    };
    let lcd = Lcd::new(config);
    system::get_ext_devices().lock().unwrap().lcds
        .push(std::rc::Rc::new(std::cell::RefCell::new(lcd)));
}

/// Add an I2C OLED display device (e.g. SSD1306). Must be called before init().
#[wasm_bindgen]
pub fn add_i2c_oled(peripheral: &str, address: u8, width: u16, height: u16) {
    use crate::ext_devices::i2c_oled::{I2cOled, I2cOledConfig};
    let config = I2cOledConfig {
        peripheral: peripheral.to_string(),
        address,
        width,
        height,
    };
    let oled = I2cOled::new(config);
    system::get_ext_devices().lock().unwrap().i2c_oleds
        .push(std::rc::Rc::new(std::cell::RefCell::new(oled)));
}

/// Register a touchscreen device. Must be called before init().
#[wasm_bindgen]
pub fn add_touchscreen(peripheral: &str, touch_detected_pin: Option<String>, cs: Option<String>) {
    use crate::ext_devices::touchscreen::{Touchscreen, TouchscreenConfig};
    let config = TouchscreenConfig {
        peripheral: peripheral.to_string(),
        framebuffer: String::new(),
        flip_x: None,
        flip_y: None,
        swap_x_y: None,
        touch_detected_pin,
        scale_down: None,
        cs,
    };
    let ts = Touchscreen::new(config);
    system::get_ext_devices().lock().unwrap().touchscreens
        .push(std::rc::Rc::new(std::cell::RefCell::new(ts)));
}

/// Set touch coordinates on a touchscreen device. Must be called after init().
#[wasm_bindgen]
pub fn touchscreen_set_touch(peripheral: &str, x: u16, y: u16, pressure: u16) {
    let et = system::get_ext_devices().lock().unwrap();
    for ts in &et.touchscreens {
        if ts.borrow().config.peripheral == peripheral {
            ts.borrow_mut().set_touch(x, y, pressure);
            break;
        }
    }
}
