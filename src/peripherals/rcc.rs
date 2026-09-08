use crate::system::System;
use super::Peripheral;

pub struct Rcc {
    cr: u32,
    cfgr: u32,
    cir: u32,
    apb2rstr: u32,
    apb1rstr: u32,
    ahbenr: u32,
    apb2enr: u32,
    apb1enr: u32,
    bdcr: u32,
    csr: u32,
}

/// Fixed crystal assumption for HSE configurations (Blue Pill boards ship an
/// 8 MHz crystal). HSI is 8 MHz by definition.
const HSI_HZ: u32 = 8_000_000;
const HSE_HZ: u32 = 8_000_000;

impl Rcc {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "RCC" {
            Some(Box::new(Rcc {
                cr: 0x0000_0083,
                cfgr: 0x0000_0000,
                cir: 0x0000_0000,
                apb2rstr: 0x0000_0000,
                apb1rstr: 0x0000_0000,
                ahbenr: 0x0000_0014,
                apb2enr: 0x0000_0000,
                apb1enr: 0x0000_0000,
                bdcr: 0x0000_0000,
                csr: 0x0000_000C,
            }))
        } else {
            None
        }
    }

    /// HSE clock failure injection (test/firmware entry point): the crystal
    /// dies (HSERDY clears). With CSSON (CR.19) this raises CSSF (CIR.7),
    /// pends an NMI, and falls back to HSI (SWS=00, SW kept); without CSSON
    /// only HSERDY clears. Returns true when CSS fired.
    pub fn fail_hse(&mut self, sys: &crate::system::System) -> bool {
        self.cr &= !(1 << 17); // HSERDY clears: the oscillator is dead
        if self.cr & (1 << 19) == 0 {
            return false;
        }
        self.cir |= 1 << 7; // CSSF
        self.cfgr &= !(3 << 2); // SWS falls back to HSI (SW kept)
        sys.p.nvic.borrow_mut().set_intr_pending(-14); // NMI
        true
    }

    /// STOP-mode exit: the system clock is HSI until firmware re-selects
    /// (SWS=00, SW kept). Called on wake from deep sleep.
    pub fn wake_from_stop(&mut self) {
        self.cfgr &= !(3 << 2);
    }

    /// SYSCLK in Hz decoded from CFGR (SW/PLLSRC/PLLMUL). Deliberately
    /// read-only w.r.t. emulation timing: the instruction budget stays fixed
    /// (1 instr = 1 SYSCLK cycle), so this is for drivers that compute
    /// dividers/frequencies from the configured clocks (e.g. USART BRR).
    /// Follows SWS (CFGR[3:2], the active clock), not SW: STOP wake and
    /// CSS failure both fall back to HSI while SW keeps its setting.
    pub fn sysclk_hz(&self) -> u32 {
        match (self.cfgr >> 2) & 3 {
            0 => HSI_HZ,
            1 => HSE_HZ,
            _ => {
                let fin = if self.cfgr & (1 << 16) != 0 {
                    if self.cfgr & (1 << 17) != 0 { HSE_HZ / 2 } else { HSE_HZ }
                } else {
                    HSI_HZ / 2
                };
                let bits = (self.cfgr >> 18) & 0xF;
                let mul = if bits >= 14 { 16 } else { bits + 2 };
                fin.saturating_mul(mul)
            }
        }
    }

    fn ahb_div(&self) -> u32 {
        match (self.cfgr >> 4) & 0xF {
            0..=7 => 1, 8 => 2, 9 => 4, 10 => 8, 11 => 16,
            12 => 64, 13 => 128, 14 => 256, _ => 512,
        }
    }

    fn apb_div(&self, shift: u32) -> u32 {
        match (self.cfgr >> shift) & 7 {
            0..=3 => 1, 4 => 2, 5 => 4, 6 => 8, _ => 16,
        }
    }

    /// (sysclk, hclk, pclk1, pclk2) in Hz after HPRE/PPRE dividers.
    pub fn clocks_hz(&self) -> (u32, u32, u32, u32) {
        let sys = self.sysclk_hz();
        let hclk = sys / self.ahb_div();
        (sys, hclk, hclk / self.apb_div(8), hclk / self.apb_div(11))
    }
}

impl Default for Rcc {
    fn default() -> Self {
        Self { cr: 0, cfgr: 0, cir: 0, apb2rstr: 0, apb1rstr: 0, ahbenr: 0, apb2enr: 0, apb1enr: 0, bdcr: 0, csr: 0 }
    }
}

impl Peripheral for Rcc {
    fn rcc_clocks(&self) -> Option<(u32, u32, u32, u32)> { Some(self.clocks_hz()) }
    fn rcc_fail_hse(&mut self, sys: &crate::system::System) -> bool {
        self.fail_hse(sys)
    }
    fn rcc_wake_from_stop(&mut self) {
        self.wake_from_stop()
    }

    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.cfgr,
            0x08 => self.cir,
            0x0C => self.apb2rstr,
            0x10 => self.apb1rstr,
            0x14 => self.ahbenr,
            0x18 => self.apb2enr,
            0x1C => self.apb1enr,
            0x20 => self.bdcr,
            0x24 => self.csr,
            _ => 0
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                let mut cr = value;
                if value & (1 << 16) != 0 { cr |= 1 << 17; }
                else { cr &= !(1 << 17); }
                if value & (1 << 24) != 0 { cr |= 1 << 25; }
                else { cr &= !(1 << 25); }
                cr |= 1 << 1;
                self.cr = cr;
            }
            0x04 => self.cfgr = (value & 0xFFFF_FFFC) | ((value & 0x3) << 2) | (value & 0x3),
            0x08 => {
                // CSSC (bit 23) clears the CSSF flag (bit 7).
                if value & (1 << 23) != 0 {
                    self.cir &= !(1 << 7);
                }
                self.cir = self.cir & !(value & 0x0E80_0000) | (value & 0x001F_001F);
            }
            0x0C => self.apb2rstr = value,
            0x10 => self.apb1rstr = value,
            0x14 => self.ahbenr = value,
            0x18 => self.apb2enr = value,
            0x1C => self.apb1enr = value,
            0x20 => {
                let lserdy = if value & 1 != 0 { 1 << 1 } else { 0 };
                self.bdcr = (value & 0x1FF) | lserdy;
                if value & (1 << 15) != 0 { self.bdcr &= !(1 << 15); }
            }
            0x24 => {
                if value & (1 << 24) != 0 { self.csr = 0x0C; }
                else { self.csr = (self.csr & 0xF00) | (value & 0x0FF); }
            }
            _ => {},
        }
    }
}
