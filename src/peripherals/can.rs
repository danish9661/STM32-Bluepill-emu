use crate::system::System;
use super::Peripheral;

#[derive(Clone, Copy)]
struct Mailbox {
    tir: u32, tdtr: u32, tdlr: u32, tdhr: u32,
}

pub struct Can {
    mcr: u32, msr: u32, tsr: u32, rf0r: u32, rf1r: u32,
    ier: u32, esr: u32, btr: u32,
    tx: [Mailbox; 3],
    rx: [Mailbox; 2],
    fmr: u32, fm1r: u32, fs1r: u32, ffa1r: u32, fa1r: u32,
    filter: [u32; 56],
    irq_base: i32,
}

impl Can {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        let irq_base = match name {
            "CAN1" => Some(19),
            "CAN2" => Some(63),
            _ => None,
        }?;
        Some(Box::new(Can {
            mcr: 0x0001_0002, msr: 0x0000_0C02, tsr: 0x1C00_0000,
            irq_base,
            ..Self::default()
        }))
    }

    fn fire_interrupts(&mut self, sys: &System) {
        let base = self.irq_base;
        // TX (TMEIE bit 0)
        if self.ier & 0x01 != 0 && self.tsr & 0x0700_0000 != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(base);
        }
        // RX0 (FMPIE0 bit 1, FFIE0 bit 2, FOVIE0 bit 3)
        if self.ier & 0x0E != 0 && self.rf0r & 0x03 != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(base + 1);
        }
        // RX1 (FMPIE1 bit 4, FFIE1 bit 5, FOVIE1 bit 6)
        if self.ier & 0x70 != 0 && self.rf1r & 0x03 != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(base + 2);
        }
        // SCE (EWGIE bit 7, EPVIE bit 8, BOFIE bit 9, LECIE bit 10, ERRIE bit 11)
        if self.ier & 0xF80 != 0 && self.esr != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(base + 3);
        }
    }
}

impl Default for Can {
    fn default() -> Self {
        Self {
            mcr: 0, msr: 0, tsr: 0, rf0r: 0, rf1r: 0, ier: 0, esr: 0, btr: 0,
            tx: [Mailbox { tir: 0, tdtr: 0, tdlr: 0, tdhr: 0 }; 3],
            rx: [Mailbox { tir: 0, tdtr: 0, tdlr: 0, tdhr: 0 }; 2],
            fmr: 0x2A1C_0E01, fm1r: 0, fs1r: 0xFFFF_FFFF, ffa1r: 0, fa1r: 0,
            filter: [0; 56],
            irq_base: 0,
        }
    }
}

impl Peripheral for Can {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x000 => self.mcr,
            0x004 => self.msr,
            0x008 => self.tsr,
            0x00C => self.rf0r,
            0x010 => self.rf1r,
            0x014 => self.ier,
            0x018 => self.esr,
            0x01C => self.btr,
            0x180..=0x1AC => {
                let i = ((offset - 0x180) / 0x10) as usize;
                if i >= 3 { return 0; }
                match (offset - 0x180) % 0x10 {
                    0x00 => self.tx[i].tir,
                    0x04 => self.tx[i].tdtr,
                    0x08 => self.tx[i].tdlr,
                    0x0C => self.tx[i].tdhr,
                    _ => 0,
                }
            }
            0x1B0..=0x1CC => {
                let i = ((offset - 0x1B0) / 0x10) as usize;
                if i >= 2 { return 0; }
                let val = match (offset - 0x1B0) % 0x10 {
                    0x00 => self.rx[i].tir,
                    0x04 => self.rx[i].tdtr,
                    0x08 => self.rx[i].tdlr,
                    0x0C => self.rx[i].tdhr,
                    _ => 0,
                };
                // Decrement FMP on read of first RX mailbox register
                if (offset - 0x1B0) % 0x10 == 0 && i == 0 && self.rf0r & 0x3 != 0 {
                    self.rf0r = (self.rf0r & !0x3) | ((self.rf0r & 0x3) - 1);
                    self.rf0r |= 1 << 3; // RFOM flag
                }
                val
            }
            0x200 => self.fmr,
            0x204 => self.fm1r,
            0x20C => self.fs1r,
            0x214 => self.ffa1r,
            0x21C => self.fa1r,
            0x240..=0x31C => {
                let i = ((offset - 0x240) / 4) as usize;
                self.filter.get(i).copied().unwrap_or(0)
            }
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x000 => {
                let mask = 0x7F3F;
                self.mcr = (self.mcr & !mask) | (value & mask);
                let inrq = value & 1;
                let sleep = (value >> 1) & 1;
                if inrq != 0 {
                    self.msr |= 1; self.msr &= !2;
                } else {
                    self.msr &= !1; self.msr |= 2;
                }
                if sleep != 0 { self.msr |= 2; }
                else if inrq == 0 { self.msr &= !2; }
            }
            0x004 => self.msr = (self.msr & !0x0C0B) | (value & 0x0C0B),
            0x008 => self.tsr &= !(value & 0x0007_0707),
            0x00C => {
                self.rf0r = (self.rf0r & 0xFFFF_0000) | (value & 0x3F);
                self.fire_interrupts(sys);
            }
            0x010 => {
                self.rf1r = (self.rf1r & 0xFFFF_0000) | (value & 0x3F);
                self.fire_interrupts(sys);
            }
            0x014 => {
                self.ier = value & 0x7FF;
                self.fire_interrupts(sys);
            }
            0x01C => self.btr = value & 0x3FFF_FFFF,
            0x180..=0x1AC => {
                let i = ((offset - 0x180) / 0x10) as usize;
                if i >= 3 { return; }
                match (offset - 0x180) % 0x10 {
                    0x00 => self.tx[i].tir = value,
                    0x04 => self.tx[i].tdtr = value,
                    0x08 => self.tx[i].tdlr = value,
                    0x0C => self.tx[i].tdhr = value,
                    _ => {}
                }
                if (offset - 0x180) % 0x10 == 0 && value & 1 != 0 {
                    self.tsr |= 1 << i;
                    self.tsr |= 0x2 << (8 + i * 4);
                    let rqcp = (self.tsr >> 26) & 7;
                    self.tsr = (self.tsr & !(7 << 26)) | ((rqcp & !(1 << i)) << 26);
                    self.tsr |= 1 << (16 + i);
                    self.fire_interrupts(sys);
                }
            }
            0x1B0..=0x1CC => {
                let i = ((offset - 0x1B0) / 0x10) as usize;
                if i >= 2 { return; }
                match (offset - 0x1B0) % 0x10 {
                    0x00 => self.rx[i].tir = value,
                    0x04 => self.rx[i].tdtr = value,
                    0x08 => self.rx[i].tdlr = value,
                    0x0C => self.rx[i].tdhr = value,
                    _ => {}
                }
            }
            0x200 => {
                if value & 1 != 0 {
                    self.fm1r = 0; self.fs1r = 0xFFFF_FFFF; self.ffa1r = 0; self.fa1r = 0;
                }
                self.fmr = value & 0x3F;
            }
            0x204 => self.fm1r = value,
            0x20C => self.fs1r = value,
            0x214 => self.ffa1r = value,
            0x21C => self.fa1r = value,
            0x240..=0x31C => {
                let i = ((offset - 0x240) / 4) as usize;
                if let Some(f) = self.filter.get_mut(i) { *f = value; }
            }
            _ => {}
        }
    }
}
