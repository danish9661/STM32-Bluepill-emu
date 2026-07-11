use crate::system::System;
use super::ExtDevice;

pub struct I2cOledConfig {
    pub peripheral: String,
    pub address: u8,
    pub width: u16,
    pub height: u16,
}

pub struct I2cOled {
    pub config: I2cOledConfig,
    name: String,
    fb: Vec<u8>,
    cmd_mode: bool,
    col: u16,
    page: u16,
    width: u16,
    pages: u16,
    display_on: bool,
    inverted: bool,
}

impl I2cOled {
    pub fn new(config: I2cOledConfig) -> Self {
        let pages = config.height / 8;
        Self {
            name: String::new(),
            fb: vec![0; (config.width as usize) * (pages as usize)],
            cmd_mode: true,
            col: 0, page: 0,
            width: config.width,
            pages,
            display_on: true,
            inverted: false,
            config,
        }
    }

    fn cmd_set_page(&mut self, v: u8) {
        self.page = (v & 0x07) as u16;
    }

    fn cmd_set_col_low(&mut self, v: u8) {
        self.col = (self.col & 0xF0) | (v & 0x0F) as u16;
    }

    fn cmd_set_col_high(&mut self, v: u8) {
        self.col = (self.col & 0x0F) | ((v as u16 & 0x0F) << 4);
    }

    fn write_pixel(&mut self, v: u8) {
        let idx = (self.page as usize) * (self.width as usize) + (self.col as usize);
        if idx < self.fb.len() {
            self.fb[idx] = if self.inverted { !v } else { v };
        }
        self.col += 1;
        if self.col >= self.width {
            self.col = 0;
            self.page += 1;
            if self.page >= self.pages {
                self.page = 0;
            }
        }
    }
}

impl ExtDevice<(), u8> for I2cOled {
    fn connect_peripheral(&mut self, peri_name: &str) -> String {
        self.name = format!("{} i2c-oled", peri_name);
        self.name.clone()
    }

    fn read(&mut self, _sys: &System, _addr: ()) -> u8 { 0 }

    fn write(&mut self, _sys: &System, _addr: (), v: u8) {
        if v == 0x00 {
            self.cmd_mode = true;
        } else if v == 0x40 {
            self.cmd_mode = false;
        } else if self.cmd_mode {
            match v {
                0xAE => self.display_on = false,
                0xAF => self.display_on = true,
                0xA6 => self.inverted = false,
                0xA7 => self.inverted = true,
                0x81 => {} // contrast: next byte is value
                0x20..=0x21 => {} // memory mode
                0x22 => {} // page start addr
                0xB0..=0xB7 => self.cmd_set_page(v - 0xB0),
                0x00..=0x0F => self.cmd_set_col_low(v),
                0x10..=0x1F => self.cmd_set_col_high(v),
                0x40 => {} // display start line (same value as data ctrl byte, only relevant in cmd mode)
                _ => {}
            }
        } else {
            self.write_pixel(v);
        }
    }

    fn reset(&mut self) {
        self.cmd_mode = true;
        self.col = 0;
        self.page = 0;
        self.display_on = true;
        self.inverted = false;
        self.fb.fill(0);
    }
}
