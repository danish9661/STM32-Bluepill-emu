use crate::system::System;
use super::Peripheral;

fn stm32_crc32(crc: u32, data: u32) -> u32 {
    let mut c = crc ^ data;
    for _ in 0..32 {
        if c & 0x80000000 != 0 { c = (c << 1) ^ 0x04C11DB7; } else { c <<= 1; }
    }
    c
}

pub struct Crc { dr: u32, idr: u32, cr: u32 }

impl Crc {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "CRC" { Some(Box::new(Crc { dr: 0xFFFF_FFFF, idr: 0, cr: 0 })) } else { None }
    }
}

impl Peripheral for Crc {
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => { let v = self.dr; sys.push_event(crate::system::VmEvent::CrcResult { value: v }); v }
            0x04 => self.idr,
            0x08 => self.cr,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.dr = stm32_crc32(self.dr, value),
            0x04 => self.idr = value & 0xFF,
            0x08 => { self.cr = value & 0xFFFFFFFE; if value & 1 != 0 { self.dr = 0xFFFF_FFFF; } }
            _ => {}
        }
    }
}
