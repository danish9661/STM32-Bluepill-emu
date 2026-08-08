use crate::system::System;
use crate::peripherals::gpio::{GpioPorts, Pin};
use super::ExtDevice;
use std::rc::Rc;
use std::cell::RefCell;

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
    pub name: String,
    pub touch_x: u16,
    pub touch_y: u16,
    pub touch_pressure: u16,
    pub is_touching: Rc<RefCell<bool>>,
    cmd_byte: u8,
    data_high: u8,
    data_low: u8,
    reply_state: u8,
    deferred_reply: bool,
}

impl Touchscreen {
    pub fn new(config: TouchscreenConfig) -> Self {
        Self {
            config,
            name: String::new(),
            touch_x: 2048,
            touch_y: 2048,
            touch_pressure: 0,
            is_touching: Rc::new(RefCell::new(false)),
            cmd_byte: 0,
            data_high: 0,
            data_low: 0,
            reply_state: 0,
            deferred_reply: false,
        }
    }

    pub fn setup_gpio(&mut self, gpio: &mut GpioPorts) {
        if let Some(ref touch_detected_pin) = self.config.touch_detected_pin {
            let pin = Pin::from_str(touch_detected_pin);
            let touching = self.is_touching.clone();
            gpio.add_read_callback(pin, move |_sys| *touching.borrow());
        }
    }

    pub fn set_touch(&mut self, x: u16, y: u16, pressure: u16) {
        self.touch_x = x;
        self.touch_y = y;
        self.touch_pressure = pressure;
        *self.is_touching.borrow_mut() = pressure > 0;
    }

    fn prepare_reply(&mut self) {
        let channel = (self.cmd_byte >> 4) & 0x07;
        let is_8bit = (self.cmd_byte >> 3) & 0x01;
        let val = match self.cmd_byte {
            0x94 => self.touch_pressure,
            _ => match channel {
                1 => self.touch_y,
                5 => self.touch_x,
                3 | 4 => self.touch_pressure,
                _ => 0,
            },
        };
        self.data_high = (val >> 4) as u8;
        self.data_low = ((val & 0x0F) << 4) as u8;
        self.reply_state = if is_8bit != 0 { 2 } else { 1 };
    }
}

impl ExtDevice<(), u8> for Touchscreen {
    fn connect_peripheral(&mut self, peri_name: &str) -> String {
        self.name = format!("{} touchscreen", peri_name);
        self.name.clone()
    }

    fn read(&mut self, _sys: &System, _addr: ()) -> u8 {
        if self.deferred_reply {
            self.deferred_reply = false;
            return 0;
        }
        match self.reply_state {
            1 => { self.reply_state = 2; self.data_high }
            2 => { self.reply_state = 0; self.data_low }
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, _addr: (), v: u8) {
        if v & 0x80 != 0 {
            self.cmd_byte = v;
            self.prepare_reply();
            self.deferred_reply = true;
        }
    }

    fn reset(&mut self) {
        self.cmd_byte = 0;
        self.reply_state = 0;
    }
}
