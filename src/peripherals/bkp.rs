use crate::system::System;
use super::Peripheral;

pub struct Bkp {
    dr: [u16; 20],
    rtccr: u32,
    cr: u32,
    csr: u32,
}

impl Bkp {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "BKP" {
            Some(Box::new(Self { dr: [0; 20], rtccr: 0, cr: 0, csr: 0 }))
        } else {
            None
        }
    }
}

impl Peripheral for Bkp {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.rtccr,
            0x04..=0x50 if (offset - 0x04) % 4 == 0 => {
                let i = ((offset - 0x04) / 4) as usize;
                self.dr.get(i).copied().unwrap_or(0) as u32
            }
            0x58 => self.cr,
            0x5C => self.csr,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.rtccr = value & 0x0373,
            0x04..=0x50 if (offset - 0x04) % 4 == 0 => {
                let i = ((offset - 0x04) / 4) as usize;
                if let Some(r) = self.dr.get_mut(i) { *r = value as u16; }
            }
            0x58 => self.cr = value & 0x03,
            0x5C => self.csr = value,
            _ => {}
        }
    }
}
