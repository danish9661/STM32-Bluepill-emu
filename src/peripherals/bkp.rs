use crate::system::System;
use super::Peripheral;

pub struct Bkp {
    dr: [u16; 10],
    rtccr: u32,
    cr: u32,
    csr: u32,
}

impl Bkp {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "BKP" {
            Some(Box::new(Self { dr: [0; 10], rtccr: 0, cr: 0, csr: 0 }))
        } else {
            None
        }
    }

    /// Tamper-pin (PC13) edge with TAMPER enabled (CR.TPE): active level set
    /// by CR.TPAL (0 = high level, 1 = low level). A tamper event clears all
    /// backup registers, sets TEF (+TIF) and pends the TAMPER IRQ (2).
    fn tamper_edge(&mut self, sys: &System, rising: bool) {
        if self.cr & 1 == 0 {
            return; // TPE off.
        }
        let tpal = self.cr & 2 != 0;
        if (rising && !tpal) || (!rising && tpal) {
            self.dr = [0; 10];
            self.csr |= (1 << 8) | (1 << 9); // TEF + TIF
            sys.p.nvic.borrow_mut().set_intr_pending(2);
        }
    }
}

impl Peripheral for Bkp {
    fn bkp_tamper(&mut self, sys: &System, rising: bool) -> bool {
        self.tamper_edge(sys, rising);
        true
    }

    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x2C => self.rtccr,
            0x04..=0x28 if (offset - 0x04) % 4 == 0 => {
                let i = ((offset - 0x04) / 4) as usize;
                self.dr.get(i).copied().unwrap_or(0) as u32
            }
            0x30 => self.cr,
            0x34 => self.csr,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x2C => self.rtccr = value & 0x3FF,
            0x04..=0x28 if (offset - 0x04) % 4 == 0 => {
                let i = ((offset - 0x04) / 4) as usize;
                if let Some(r) = self.dr.get_mut(i) { *r = value as u16; }
            }
            0x30 => self.cr = value & 0x03, // TPE + TPAL
            0x34 => {
                // W1C: CTEF (bit 0) clears TEF, CTI (bit 1) clears TIF.
                if value & 1 != 0 { self.csr &= !(1 << 8); }
                if value & 2 != 0 { self.csr &= !(1 << 9); }
            }
            _ => {}
        }
    }
}
