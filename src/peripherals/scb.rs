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

    fn write_shpr(&mut self, sys: &System, offset: u32, value: u32) {
        let mut nvic = sys.p.nvic.borrow_mut();
        match offset {
            0x18 => { // SHPR1: MemManage(4), BusFault(5), UsageFault(6)
                nvic.set_sys_handler_prio(4, (value & 0xFF) as u8);
                nvic.set_sys_handler_prio(5, ((value >> 8) & 0xFF) as u8);
                nvic.set_sys_handler_prio(6, ((value >> 16) & 0xFF) as u8);
            }
            0x1C => { // SHPR2: SVCall(11)
                nvic.set_sys_handler_prio(11, (value & 0xFF) as u8);
            }
            0x20 => { // SHPR3: PendSV(14), SysTick(15)
                nvic.set_sys_handler_prio(14, (value & 0xFF) as u8);
                nvic.set_sys_handler_prio(15, ((value >> 8) & 0xFF) as u8);
            }
            _ => {}
        }
        self.shpr[(offset - 0x18) as usize / 4] = value;
    }

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
    /// STOP/STANDBY: SCR SLEEPDEEP (bit 2) selects deep sleep on WFI/WFE.
    fn in_deep_sleep(&self) -> bool {
        self.scr & (1 << 2) != 0
    }

    /// Raise a fault with full SCB bookkeeping and NVIC escalation.
    /// kind: 0 = instruction fetch, 1 = data read, 2 = data write, 3 = undefined instruction.
    /// The fault status (CFSR/BFAR) is always recorded; the pending target
    /// depends on the SHCSR enable bits (else the fault escalates to HardFault).
    fn raise_fault(&mut self, sys: &System, kind: u32, addr: u32) {
        use crate::peripherals::nvic::irq;
        let (target, enabled) = match kind {
            0 | 1 | 2 => {
                self.cfsr |= if kind == 0 { 1 << 8 } else { 1 << 9 }; // IBUSERR | PRECISERR
                self.bfar = addr;
                self.cfsr |= 1 << 15; // BFARVALID
                (irq::BUS_FAULT, self.shcsr & (1 << 18) != 0) // BUSFAULTENA
            }
            _ => {
                self.cfsr |= 1 << 16; // UNDEFINSTR
                (irq::USAGE_FAULT, self.shcsr & (1 << 16) != 0) // USGFAULTENA
            }
        };
        let mut nvic = sys.p.nvic.borrow_mut();
        if enabled {
            nvic.set_intr_pending(target);
        } else {
            // fault exception disabled: escalate to HardFault
            self.hfsr |= 1 << 30; // FORCED
            nvic.set_intr_pending(irq::HARD_FAULT);
        }
    }

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
            0x28 => self.cfsr | _sys.mpu.get().mmfsr as u32,
            0x2C => self.hfsr,
            0x30 => self.dfsr,
            0x34 => {
                let m = _sys.mpu.get();
                if m.mmfar_valid {
                    m.mmfar
                } else {
                    self.mmfar
                }
            }
            0x38 => self.bfar,
            0x3C => self.afsr,
            // 0x40-0x84 reserved
            0x88 => self.cpacr,
            // MPU (delegated to WasmSystem state; see docs/CPU.md).
            0x90 => 0x0800, // TYPE: 8 regions, unified
            0x94 => _sys.mpu.get().ctrl,
            0x98 => _sys.mpu.get().rnr as u32,
            0x9C => _sys.mpu.get().regions[_sys.mpu.get().sel()].rbar,
            0xA0 => _sys.mpu.get().regions[_sys.mpu.get().sel()].rasr,
            0xA4 => _sys.mpu.get().regions[1].rbar,
            0xA8 => _sys.mpu.get().regions[1].rasr,
            0xAC => _sys.mpu.get().regions[2].rbar,
            0xB0 => _sys.mpu.get().regions[2].rasr,
            0xB4 => _sys.mpu.get().regions[3].rbar,
            0xB8 => _sys.mpu.get().regions[3].rasr,
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
             0x18 => self.write_shpr(sys, 0x18, value),
             0x1C => self.write_shpr(sys, 0x1C, value),
             0x20 => self.write_shpr(sys, 0x20, value),
            0x24 => self.shcsr = value & 0x7FFFF,
            0x28 => {
                // CFSR is write-1-to-clear (real HW semantics).
                self.cfsr &= !value;
                let mut m = sys.mpu.get();
                m.mmfsr &= !(value as u8);
                if m.mmfsr == 0 {
                    m.mmfar_valid = false;
                }
                sys.mpu.set(m);
            }
            0x2C => self.hfsr = value & 0x7FFF,
            0x30 => self.dfsr = value & 0xFFFF,
            0x34 => self.mmfar = value,
            0x38 => self.bfar = value,
            0x3C => self.afsr = value,
            0x88 => self.cpacr = value & 0x0F00_0000,
            // MPU register file (state lives on WasmSystem; RNR-selected
            // plus A1-A3 aliases, VALID latches RNR first per ARM ordering).
            0x94 => {
                let mut m = sys.mpu.get();
                m.ctrl = value & 7;
                sys.mpu.set(m);
            }
            0x98 => {
                let mut m = sys.mpu.get();
                m.rnr = (value & 7) as u8;
                sys.mpu.set(m);
            }
            0x9C | 0xA4 | 0xAC | 0xB4 => {
                let mut m = sys.mpu.get();
                let idx = match offset {
                    0x9C => {
                        if value & (1 << 4) != 0 {
                            m.rnr = (value & 7) as u8;
                        }
                        m.sel()
                    }
                    0xA4 => 1,
                    0xAC => 2,
                    _ => 3,
                };
                m.regions[idx].rbar = value;
                sys.mpu.set(m);
            }
            0xA0 | 0xA8 | 0xB0 | 0xB8 => {
                let mut m = sys.mpu.get();
                let idx = match offset {
                    0xA0 => m.sel(),
                    0xA8 => 1,
                    0xB0 => 2,
                    _ => 3,
                };
                m.regions[idx].rasr = value;
                sys.mpu.set(m);
            }
            _ => {}
        }
    }
}
