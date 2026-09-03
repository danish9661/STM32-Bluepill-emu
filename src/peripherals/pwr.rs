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

    /// PVD output with the modeled fixed 3.3 V supply: every PLS threshold
    /// (2.2–2.9 V) sits below it, so the output follows PVDE (CR bit 4).
    fn pvdo(&self) -> bool { self.cr & (1 << 4) != 0 }
}

impl Peripheral for Pwr {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => (self.csr & !0x4) | if self.pvdo() { 0x4 } else { 0 },
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                let old = self.pvdo();
                self.cr = value & 0x1FF;
                let new = self.pvdo();
                // PVDE edge with the fixed supply above threshold: report it
                // to EXTI line 16 (RTSR/FTSR + IMR gating lives there).
                if new != old {
                    sys.p.exti_line_edge(sys, 16, new);
                }
            }
            // PVDO (bit 2) is read-only status: never writable.
            0x04 => self.csr = (self.csr & 0x100) | (value & 0xFB),
            _ => {}
        }
    }
}
