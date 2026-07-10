use crate::system::System;
use super::Peripheral;

pub struct Flash {
    acr: u32,
    keyr: u32,
    sr: u32,
    cr: u32,
    ar: u32,
    obr: u32,
    wrpr: u32,
}

impl Flash {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "FLASH" || name == "FLASH" {
            Some(Box::new(Flash {
                acr: 0x0000_0030,
                ..Self::default()
            }))
        } else {
            None
        }
    }
}

impl Default for Flash {
    fn default() -> Self {
        Self { acr: 0, keyr: 0, sr: 0, cr: 0, ar: 0, obr: 0, wrpr: 0 }
    }
}

impl Peripheral for Flash {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.acr,
            0x04 => self.keyr,
            0x08 => self.sr,
            0x0C => self.cr,
            0x10 => self.ar,
            0x14 => self.obr,
            0x18 => self.wrpr,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.acr = value & 0xFF,
            0x04 => self.keyr = value,
            0x08 => self.sr = value & 0xB3,
            0x0C => self.cr = value & 0x3FFF,
            0x10 => self.ar = value,
            0x14 => self.obr |= value,
            0x18 => self.wrpr = value,
            _ => {}
        }
    }
}
