use crate::system::System;
use super::Peripheral;

pub struct Rtc {
    crh: u32,
    crl: u32,
    prlh: u32,
    prll: u32,
    cnth: u32,
    cntl: u32,
    alrh: u32,
    alrl: u32,
    cnt: u32,
}

impl Rtc {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "RTC" {
            Some(Box::new(Rtc {
                crh: 0,
                crl: 0x0020,
                prlh: 0,
                prll: 0x7FFF,
                cnth: 0,
                cntl: 0,
                alrh: 0xFFFF,
                alrl: 0xFFFF,
                cnt: 0,
            }))
        } else {
            None
        }
    }
}

impl Peripheral for Rtc {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.crh,
            0x04 => self.crl,
            0x08 => self.prlh,
            0x0C => self.prll,
            0x10 => self.cnth,
            0x14 => self.cntl,
            0x18 => self.alrh,
            0x1C => self.alrl,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.crh = value & 0x03,
            0x04 => {
                if value & (1 << 4) != 0 {
                    self.crl = (self.crl & !(1 << 4)) | (value & (1 << 4));
                }
                let rtoff = value & (1 << 5);
                if rtoff != 0 {
                    self.crl = (self.crl & !(1 << 5) & !(1 << 4)) | (value & (1 << 5));
                }
                if value & (1 << 3) != 0 { self.crl |= 1 << 3; }
                self.crl = (self.crl & !0x07) | (value & 0x07);
            }
            0x08 => self.prlh = value & 0xFFFF,
            0x0C => self.prll = value & 0xFFFF,
            0x10 => self.cnth = value & 0xFFFF,
            0x14 => self.cntl = value & 0xFFFF,
            0x18 => self.alrh = value & 0xFFFF,
            0x1C => self.alrl = value & 0xFFFF,
            _ => {}
        }
    }
}
