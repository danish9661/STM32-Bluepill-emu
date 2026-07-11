use crate::system::System;
use super::Peripheral;

const FLASH_KEY1: u32 = 0x45670123;
const FLASH_KEY2: u32 = 0xCDEF89AB;

pub struct Flash {
    acr: u32,
    keyr: u32,
    optkeyr: u32,
    sr: u32,
    cr: u32,
    ar: u32,
    obr: u32,
    wrpr: u32,
    key_step: u8,
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

    fn update_sr(&mut self) {
        if self.cr & (1 << 6) != 0 { // STRT
            self.sr |= 1 << 0; // BSY
            self.cr &= !(1 << 6);
        } else if self.cr & ((1 << 2) | (1 << 1) | (1 << 0)) != 0 {
            self.sr |= 1 << 0; // BSY
        } else {
            self.sr &= !(1 << 0); // BSY clear when no operation
        }
    }
}

impl Default for Flash {
    fn default() -> Self {
        Self { acr: 0, keyr: 0, optkeyr: 0, sr: 0, cr: 0x80, ar: 0, obr: 0, wrpr: 0, key_step: 0 }
    }
}

impl Peripheral for Flash {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.acr,
            0x04 => self.keyr,
            0x08 => self.optkeyr,
            0x0C => self.sr,
            0x10 => self.cr,
            0x14 => self.ar,
            0x1C => self.obr,
            0x20 => self.wrpr,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.acr = value & 0xFF,
            0x04 => {
                self.keyr = value;
                match self.key_step {
                    0 => {
                        if value == FLASH_KEY1 { self.key_step = 1; }
                        else { self.key_step = 0; }
                    }
                    1 => {
                        if value == FLASH_KEY2 {
                            self.cr &= !(1 << 7); // clear LOCK
                        }
                        self.key_step = 0;
                    }
                    _ => self.key_step = 0,
                }
            }
            0x08 => self.optkeyr = value,
            0x0C => self.sr = value & 0xB3,
            0x10 => {
                let is_locked = self.cr & (1 << 7) != 0;
                let mut cr = value & 0x3FFF;
                if is_locked {
                    cr &= !(1 << 7);
                }
                let start_was_set = (value & (1 << 6)) != 0 && (self.cr & (1 << 6)) == 0;
                self.cr = cr;
                if start_was_set {
                    if cr & ((1 << 2) | (1 << 1) | (1 << 0)) != 0 {
                        self.sr |= 1 << 0;
                    }
                    self.cr &= !(1 << 6);
                }
                self.update_sr();
            }
            0x14 => self.ar = value,
            0x1C => self.obr |= value,
            0x20 => self.wrpr = value,
            _ => {}
        }
    }
}
