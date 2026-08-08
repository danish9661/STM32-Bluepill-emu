use crate::system::System;
use super::Peripheral;
use std::sync::atomic::{AtomicU16, Ordering};

const ADC_IRQ: i32 = 18;

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
    #[allow(dead_code)]
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

    fn fire_interrupts(&self, sys: &System) {
        if self.sr & 1 != 0 && self.cr1 & (1 << 6) != 0 { // AWD + AWDIE
            sys.p.nvic.borrow_mut().set_intr_pending(ADC_IRQ);
        }
        if self.sr & (1 << 1) != 0 && self.cr1 & (1 << 5) != 0 { // EOC + EOCIE
            sys.p.nvic.borrow_mut().set_intr_pending(ADC_IRQ);
        }
        if self.sr & (1 << 2) != 0 && self.cr1 & (1 << 7) != 0 { // JEOC + JEOCIE
            sys.p.nvic.borrow_mut().set_intr_pending(ADC_IRQ);
        }
    }
}

impl Peripheral for Adc {
    fn tick(&mut self, sys: &System) {
        self.fire_interrupts(sys);
    }

    fn read(&mut self, sys: &System, offset: u32) -> u32 {
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
                self.fire_interrupts(sys);
                self.dr
            }
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.sr = value & 0x3F,
            0x04 => { self.cr1 = value; self.fire_interrupts(sys); }
            0x08 => {
                // CAL (bit 2) and RSTCAL (bit 3) self-clear on real hardware
                self.cr2 = value & !((1 << 2) | (1 << 3));
                if value & (1 << 2) != 0 {
                    // calibration completes instantly: produce a sample + EOC
                    self.dr = ADC_SIM_VALUE.load(Ordering::Relaxed) as u32;
                    self.sr |= 1 << 1; // EOC
                    self.fire_interrupts(sys);
                }
                if value & (1 << 22) != 0 {
                    self.dr = ADC_SIM_VALUE.load(Ordering::Relaxed) as u32;
                    self.sr |= 1 << 1; // EOC
                    self.fire_interrupts(sys);
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
