use crate::system::System;
use super::Peripheral;

const CRC32_POLY: u32 = 0x04C1_1DB7;

fn crc32_byte(crc: u32, byte: u8) -> u32 {
    let mut crc = crc ^ (byte as u32) << 24;
    for _ in 0..8 { if crc & 0x8000_0000 != 0 { crc = (crc << 1) ^ CRC32_POLY; } else { crc <<= 1; } }
    crc
}

fn crc32_word(crc: u32, word: u32) -> u32 {
    let mut crc = crc;
    for i in (0..4).rev() { crc = crc32_byte(crc, (word >> (i * 8)) as u8); }
    crc
}

pub struct Crc { dr: u32, idr: u32, cr: u32 }

impl Crc {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "CRC" { Some(Box::new(Crc { dr: 0xFFFF_FFFF, idr: 0, cr: 0 })) } else { None }
    }
}

impl Peripheral for Crc {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset { 0x00 => self.dr, 0x04 => self.idr, 0x08 => self.cr, _ => 0 }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.dr = crc32_word(self.dr, value),
            0x04 => self.idr = value & 0xFF,
            0x08 => { self.cr = value & 0xFFFFFFFE; if value & 1 != 0 { self.dr = 0xFFFF_FFFF; } }
            _ => {}
        }
    }
}
