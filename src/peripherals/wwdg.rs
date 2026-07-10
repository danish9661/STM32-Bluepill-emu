use std::sync::atomic::Ordering;
use crate::system::{System, INSTRUCTION_COUNT, request_watchdog_reset};
use super::Peripheral;

pub struct Wwdg {
    cr: u32,
    cfr: u32,
    sr: u32,
    last_tick: u64,
    initialized: bool,
}

impl Wwdg {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "WWDG" {
            Some(Box::new(Wwdg { cr: 0x7F, cfr: 0x7F, sr: 0, last_tick: 0, initialized: false }))
        } else { None }
    }

    fn wdga_enabled(&self) -> bool { self.cr & 0x80 != 0 }
    fn prescaler(&self) -> u32 {
        match (self.cfr >> 7) & 3 { 0 => 1, 1 => 2, 2 => 4, 3 => 8, _ => 1 }
    }
    fn tick_instructions(&self) -> u64 { 256 * self.prescaler() as u64 }

    fn elapsed_ticks(&mut self) -> u32 {
        let now = INSTRUCTION_COUNT.load(Ordering::Relaxed);
        if !self.initialized { self.last_tick = now; self.initialized = true; return 0; }
        let elapsed = now.saturating_sub(self.last_tick);
        let ticks = elapsed / self.tick_instructions();
        if ticks > 0 { self.last_tick = now; }
        ticks.min(0x7F) as u32
    }

    fn decrement_counter(&mut self, sys: &System) {
        let ticks = self.elapsed_ticks();
        if ticks == 0 { return; }
        let counter = self.cr & 0x7F;
        if counter < ticks {
            self.cr = (self.cr & !0x7F) | 0x3F;
            self.sr |= 1;
            if self.cfr & 0x200 != 0 { sys.p.nvic.borrow_mut().set_intr_pending(0); }
            if self.wdga_enabled() { request_watchdog_reset(); }
            return;
        }
        let new_counter = counter - ticks;
        self.cr = (self.cr & !0x7F) | new_counter;
        if new_counter <= 0x3F && counter > 0x3F {
            self.sr |= 1;
            if self.cfr & 0x200 != 0 { sys.p.nvic.borrow_mut().set_intr_pending(0); }
        }
    }

    fn refresh(&mut self, value: u32) {
        self.last_tick = INSTRUCTION_COUNT.load(Ordering::Relaxed);
        self.initialized = true;
        self.cr = value & 0xFF;
    }
}

impl Peripheral for Wwdg {
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        self.decrement_counter(sys);
        match offset { 0x00 => self.cr, 0x04 => self.cfr, 0x08 => self.sr, _ => 0 }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                self.decrement_counter(sys);
                if value & 0x80 != 0 && self.cfr & 0x7F != 0 {
                    if (self.cr & 0x7F) > (self.cfr & 0x7F) { request_watchdog_reset(); }
                }
                self.refresh(value);
            }
            0x04 => self.cfr = value & 0x7FFF,
            0x08 => self.sr &= !(value & 1),
            _ => {}
        }
    }
}
