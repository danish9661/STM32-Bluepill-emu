use crate::system::System;
use super::Peripheral;

pub struct SysTick {
    csr: u32,
    rvr: u32,
    cvr: u32,
    calib: u32,
    val_toggle: bool,
}

impl SysTick {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "SysTick" || name == "STK" {
            Some(Box::new(Self { csr: 0, rvr: 0, cvr: 0, calib: 0, val_toggle: false }))
        } else {
            None
        }
    }
}

impl Peripheral for SysTick {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.csr,
            0x04 => self.rvr,
            0x08 => {
                self.val_toggle = !self.val_toggle;
                if self.val_toggle { self.cvr } else { self.rvr }
            }
            0x0C => self.calib,
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                self.csr = value & 0x10007;
                if value & 1 != 0 && self.rvr != 0 {
                    sys.p.nvic.borrow_mut().systick_period = Some(self.rvr);
                } else {
                    sys.p.nvic.borrow_mut().systick_period = None;
                }
            }
            0x04 => self.rvr = value & 0x00FF_FFFF,
            0x08 => {
                self.cvr = 0;
                sys.p.nvic.borrow_mut().last_systick_trigger = crate::system::INSTRUCTION_COUNT.load(std::sync::atomic::Ordering::Relaxed);
            }
            0x0C => {}
            _ => {}
        }
    }
}
