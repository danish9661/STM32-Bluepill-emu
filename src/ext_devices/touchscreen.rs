use crate::system::System;
use crate::peripherals::gpio::{GpioPorts, Pin};
use super::ExtDevice;

pub struct TouchscreenConfig {
    pub peripheral: String,
    pub framebuffer: String,
    pub flip_x: Option<bool>,
    pub flip_y: Option<bool>,
    pub swap_x_y: Option<bool>,
    pub touch_detected_pin: Option<String>,
    pub scale_down: Option<u32>,
    pub cs: Option<String>,
}

pub struct Touchscreen {
    pub config: TouchscreenConfig,
    name: String,
}

impl Touchscreen {
    pub fn new(config: TouchscreenConfig, gpio: &mut GpioPorts) -> Self {
        if let Some(ref touch_detected_pin) = config.touch_detected_pin {
            let pin = Pin::from_str(touch_detected_pin);
            gpio.add_read_callback(pin, move |_sys| true);
        }
        Self { config, name: String::new() }
    }
}

impl ExtDevice<(), u8> for Touchscreen {
    fn connect_peripheral(&mut self, peri_name: &str) -> String {
        self.name = format!("{} touchscreen", peri_name);
        self.name.clone()
    }

    fn read(&mut self, _sys: &System, _addr: ()) -> u8 { 0 }

    fn write(&mut self, _sys: &System, _addr: (), _v: u8) {}
}
