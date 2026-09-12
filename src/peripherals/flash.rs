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
        Self { acr: 0, keyr: 0, optkeyr: 0, sr: 0, cr: 0x80, ar: 0, obr: 0x03FF_FFFF, wrpr: 0, key_step: 0 }
    }
}

impl Peripheral for Flash {
    fn flash_latency(&self) -> u32 { self.acr & 7 }
    fn flash_wdg_hw(&self) -> bool { (self.obr >> 2) & 1 == 0 }
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.acr,
            0x04 => self.keyr,
            0x08 => self.sr,
            0x0C => self.cr,
            0x10 => self.ar,
            0x14 => self.optkeyr,
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
            0x08 => self.sr = value & 0xB3,
            0x0C => {
                let is_locked = self.cr & (1 << 7) != 0;
                let mut cr = value & 0x3FFF;
                if is_locked {
                    cr &= !(1 << 7);
                }
                let start_was_set = (value & (1 << 6)) != 0 && (self.cr & (1 << 6)) == 0;
                self.cr = cr;
                if start_was_set {
                    // Write protection: WRPR bit n guards 4KB block n
                    // (AR[15:12]); programming/erasing a protected page
                    // raises WRPRTERR (SR.4) instead of going busy. (Flash
                    // contents themselves are immutable to the guest —
                    // programming has no memory effect to gate.)
                    let op = cr & ((1 << 2) | (1 << 1) | (1 << 0));
                    if op != 0 {
                        let block = (self.ar >> 12) & 0xF;
                        let in_flash = (0x0800_0000..0x0801_0000).contains(&self.ar);
                        if in_flash && (self.wrpr >> block) & 1 != 0 {
                            self.sr |= 1 << 4; // WRPRTERR
                        } else if cr & ((1 << 2) | (1 << 1) | (1 << 0)) != 0 {
                            self.sr |= 1 << 0;
                        }
                    }
                    self.cr &= !(1 << 6);
                }
                self.update_sr();
            }
            0x10 => self.ar = value,
            0x14 => self.optkeyr = value,
            0x1C => {
                // USER option byte [9:2] (WDG_SW=2, nRST_STOP=3,
                // nRST_STDBY=4): directly settable under the lenient
                // option-programming model; other status bits accumulate.
                // Default erased state (all set) = software watchdog.
                self.obr = ((self.obr | value) & !0x3FC) | (value & 0x3FC);
            }
            0x20 => self.wrpr = value,
            _ => {}
        }
    }
}
