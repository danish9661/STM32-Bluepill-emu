use crate::system::System;
use super::Peripheral;

pub struct Pwr {
    cr: u32,
    csr: u32,
}

impl Pwr {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "PWR" {
            Some(Box::new(Pwr { cr: 0x0000_0020, csr: 0x0000_0008 }))
        } else {
            None
        }
    }
}

impl Peripheral for Pwr {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.csr,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.cr = value & 0x1FF,
            0x04 => self.csr = (self.csr & 0x100) | (value & 0xFF),
            _ => {}
        }
    }
}
