use crate::system::System;
use super::Peripheral;

pub struct Scb {
    vtor: u32,       // 0x08
    icsr: u32,       // 0x04
    aircr: u32,      // 0x0C
    scr: u32,        // 0x10
    ccr: u32,        // 0x14
    shpr: [u32; 3],  // 0x18-0x20 system handler priorities (SHPR1-SHPR3)
    shcsr: u32,      // 0x24
    cfsr: u32,       // 0x28
    hfsr: u32,       // 0x2C
    dfsr: u32,       // 0x30
    mmfar: u32,      // 0x34
    bfar: u32,       // 0x38
    afsr: u32,       // 0x3C
    cpacr: u32,      // 0x88
}

impl Default for Scb {
    fn default() -> Self {
        Self {
            vtor: 0x0800_0000,
            aircr: 0xFA05_0000,
            shcsr: 0x0000_0000,
            ..unsafe { std::mem::zeroed() }
        }
    }
}

impl Scb {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "SCB" || name == "SCB_Trusted" { Some(Box::new(Self::default())) } else { None }
    }

    pub fn vtor(&self) -> u32 { self.vtor }

    fn write_aircr(&mut self, value: u32) {
        if (value & 0xFFFF) == 0x05FA {
            self.aircr = (value & 0xFFFF_0000) | 0x05FA_0000;
            let vectkey = (value >> 16) & 0xFFFF;
            if vectkey == 0x05FA {
                let sysreset = (value >> 2) & 1;
                if sysreset == 1 {
                    crate::system::request_watchdog_reset();
                }
            }
        }
    }

    fn write_icsr(&mut self, value: u32, sys: &System) {
        use crate::peripherals::nvic::irq;
        // Set-pending
        if value & (1 << 28) != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(irq::PENDSV);
        }
        if value & (1 << 26) != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(irq::SYSTICK);
        }
        // Clear-pending
        if value & (1 << 25) != 0 {
            sys.p.nvic.borrow_mut().clear_pending(irq::SYSTICK);
        }
        if value & (1 << 27) != 0 {
            sys.p.nvic.borrow_mut().clear_pending(irq::PENDSV);
        }
        self.icsr = (self.icsr & 0xE01F_FFFF) | (value & 0x1FE0_0000) | (value & 0x1FF);
    }
}

impl Peripheral for Scb {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => {
                // CPUID - r1p0 of Cortex-M3
                let implementer = 0x41; // ARM
                let variant = 1;
                let part = 0xC23; // Cortex-M3
                let revision = 0;
                (implementer << 24) | (variant << 20) | (part << 4) | revision
            }
            0x04 => {
                // ICSR: current pending vector, etc.
                let mut v = self.icsr & 0xE01F_FFFF;
                v |= (_sys.p.nvic.borrow().get_pending_vector()) << 16;
                v
            }
            0x08 => self.vtor,
            0x0C => self.aircr,
            0x10 => self.scr,
            0x14 => self.ccr,
            0x18 => self.shpr[0],
            0x1C => self.shpr[1],
            0x20 => self.shpr[2],
            0x24 => self.shcsr,
            0x28 => self.cfsr,
            0x2C => self.hfsr,
            0x30 => self.dfsr,
            0x34 => self.mmfar,
            0x38 => self.bfar,
            0x3C => self.afsr,
            // 0x40-0x84 reserved
            0x88 => self.cpacr,
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x04 => self.write_icsr(value, sys),
            0x08 => self.vtor = value & 0xFFFF_FC00,
            0x0C => self.write_aircr(value),
            0x10 => self.scr = value & 0x1E,
            0x14 => self.ccr = value & 0xFFFF,
            0x18 => self.shpr[0] = value,
            0x1C => self.shpr[1] = value,
            0x20 => self.shpr[2] = value,
            0x24 => self.shcsr = value & 0xFFFF,
            0x28 => self.cfsr = value & 0xFFFF_FFFF,
            0x2C => self.hfsr = value & 0x7FFF,
            0x30 => self.dfsr = value & 0xFFFF,
            0x34 => self.mmfar = value,
            0x38 => self.bfar = value,
            0x3C => self.afsr = value,
            0x88 => self.cpacr = value & 0x0F00_0000,
            _ => {}
        }
    }
}
