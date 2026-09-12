use crate::system::System;
use super::Peripheral;

pub struct Pwr {
    cr: u32,
    csr: u32,
    /// Modeled supply in mV (default 3300). Test/firmware entry point:
    /// ramp it across a PLS threshold to exercise PVD edges.
    supply_mv: u32,
}

/// PVD falling/rising thresholds in mV for PLS[7:5] = 0..7 (RM0008).
const PVD_MV: [u32; 8] = [2200, 2300, 2400, 2500, 2600, 2700, 2800, 2900];

impl Pwr {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "PWR" {
            Some(Box::new(Pwr { cr: 0x0000_0020, csr: 0x0000_0008, supply_mv: 3300 }))
        } else {
            None
        }
    }

    /// PVD output: set when PVDE (CR.4) is enabled and the supply sits
    /// below the PLS-selected threshold.
    fn pvdo(&self) -> bool {
        if self.cr & (1 << 4) == 0 {
            return false;
        }
        let th = PVD_MV[((self.cr >> 5) & 7) as usize];
        self.supply_mv < th
    }

    /// EWUP (CSR.8): WKUP pin (PA0) enabled as a standby wakeup source.
    fn ewup(&self) -> bool { self.csr & (1 << 8) != 0 }

    /// Set the modeled supply; fans a PVDO edge to EXTI line 16 like a
    /// CR write does. Returns the new PVDO level.
    pub fn set_supply(&mut self, sys: &System, mv: u32) -> bool {
        let old = self.pvdo();
        self.supply_mv = mv;
        let new = self.pvdo();
        if new != old {
            sys.p.exti_line_edge(sys, 16, new);
        }
        new
    }

    /// WKUP-pin (PA0) rising edge: with EWUP set, latch WUF (CSR.0).
    /// Returns true when this edge wakes standby (caller gates on mode).
    pub fn wkup_edge(&mut self, rising: bool) -> bool {
        if rising && self.ewup() {
            self.csr |= 1;
            return true;
        }
        false
    }
}

impl Peripheral for Pwr {
    fn pwr_standby_selected(&self) -> bool { self.cr & (1 << 1) != 0 }
    fn pwr_regulator_low_power(&self) -> bool { self.cr & (1 << 0) != 0 }
    fn pwr_ewup(&self) -> bool { self.ewup() }
    fn pwr_wkup_edge(&mut self, _sys: &System, rising: bool) {
        self.wkup_edge(rising);
    }
    fn pwr_set_supply(&mut self, sys: &System, mv: u32) -> bool {
        self.set_supply(sys, mv)
    }
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
                // PVD edge (PVDE or PLS change crossing the supply):
                // report it to EXTI line 16 (gating lives there).
                if new != old {
                    sys.p.exti_line_edge(sys, 16, new);
                }
            }
            // PVDO (bit 2) is read-only status: never writable. WUF (bit 0)
            // clears on any CSR write; EWUP (bit 8) is a plain RW bit.
            // Other bits are sticky state (nothing sets SBF yet).
            0x04 => self.csr = (self.csr & 0xFE & !0x100) | (value & 0x100),
            _ => {}
        }
    }
}
