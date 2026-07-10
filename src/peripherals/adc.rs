use crate::system::System;
use super::Peripheral;
use std::sync::atomic::{AtomicU16, Ordering};

static ADC_SIM_VALUE: AtomicU16 = AtomicU16::new(0x3FF);

pub fn set_adc_value(val: u16) {
    ADC_SIM_VALUE.store(val, Ordering::Relaxed);
}

#[derive(Default)]
pub struct Adc {
    sr: u32,
    cr1: u32,
    cr2: u32,
    smpr1: u32,
    smpr2: u32,
    jofr: [u32; 4],
    htr: u32,
    ltr: u32,
    sqr1: u32,
    sqr2: u32,
    sqr3: u32,
    jsqr: u32,
    jdata: [u32; 4],
    dr: u32,
    ofr: [u32; 4],
}

impl Adc {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name.starts_with("ADC") {
            Some(Box::new(Self::default()))
        } else {
            None
        }
    }
}

impl Peripheral for Adc {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.sr,
            0x04 => self.cr1,
            0x08 => self.cr2,
            0x0C => self.smpr1,
            0x10 => self.smpr2,
            0x14 => self.jofr[0],
            0x18 => self.jofr[1],
            0x1C => self.jofr[2],
            0x20 => self.jofr[3],
            0x24 => self.htr,
            0x28 => self.ltr,
            0x2C => self.sqr1,
            0x30 => self.sqr2,
            0x34 => self.sqr3,
            0x38 => self.jsqr,
            0x3C => self.jdata[0],
            0x40 => self.jdata[1],
            0x44 => self.jdata[2],
            0x48 => self.jdata[3],
            0x4C => {
                self.sr &= !(1 << 1); // clear EOC on DR read
                self.dr
            }
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.sr = value & 0x3F,
            0x04 => self.cr1 = value,
            0x08 => {
                self.cr2 = value;
                if value & (1 << 22) != 0 {
                    // SWSTART: simulate a conversion
                    self.dr = ADC_SIM_VALUE.load(Ordering::Relaxed) as u32;
                    self.sr |= 1 << 1; // EOC
                }
            }
            0x0C => self.smpr1 = value,
            0x10 => self.smpr2 = value,
            0x14 => self.jofr[0] = value,
            0x18 => self.jofr[1] = value,
            0x1C => self.jofr[2] = value,
            0x20 => self.jofr[3] = value,
            0x24 => self.htr = value,
            0x28 => self.ltr = value,
            0x2C => self.sqr1 = value,
            0x30 => self.sqr2 = value,
            0x34 => self.sqr3 = value,
            0x38 => self.jsqr = value,
            _ => {}
        }
    }
}
