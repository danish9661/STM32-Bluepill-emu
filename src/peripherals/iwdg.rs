use std::sync::atomic::Ordering;
use crate::system::{System, INSTRUCTION_COUNT, request_watchdog_reset};
use super::Peripheral;

pub struct Iwdg {
    kr: u32, pr: u32, rlr: u32, sr: u32,
    counter: u32, last_tick: u64, enabled: bool, sr_tick: u64,
}

impl Iwdg {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "IWDG" {
            Some(Box::new(Iwdg { kr: 0, pr: 0, rlr: 0xFFF, sr: 0, counter: 0xFFF, last_tick: 0, enabled: false, sr_tick: 0 }))
        } else { None }
    }

    fn prescaler_div(&self) -> u64 {
        match self.pr & 7 { 0 => 4, 1 => 8, 2 => 16, 3 => 32, 4 => 64, 5 => 128, 6 => 256, 7 => 256, _ => 4 }
    }
    fn tick_instructions(&self) -> u64 { 128 * self.prescaler_div() }

    fn elapsed_ticks(&mut self) -> u32 {
        let now = INSTRUCTION_COUNT.load(Ordering::Relaxed);
        let elapsed = now.saturating_sub(self.last_tick);
        let ticks = elapsed / self.tick_instructions();
        if ticks > 0 { self.last_tick = now; }
        (ticks as u64).min(self.counter as u64) as u32
    }

    fn decrement_counter(&mut self) {
        let now = INSTRUCTION_COUNT.load(Ordering::Relaxed);
        if self.sr != 0 && now.saturating_sub(self.sr_tick) >= self.tick_instructions() { self.sr = 0; }
        if !self.enabled { return; }
        let elapsed = now.saturating_sub(self.last_tick);
        let ticks = elapsed / self.tick_instructions();
        if ticks == 0 { return; }
        self.last_tick = now;
        if self.counter <= ticks as u32 { self.counter = 0; request_watchdog_reset(); }
        else { self.counter -= ticks as u32; }
    }
}

impl Peripheral for Iwdg {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        self.decrement_counter();
        match offset {
            0x04 => self.pr, 0x08 => self.rlr,
            0x0C => { let sr = self.sr; self.sr = 0; sr }
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        self.decrement_counter();
        match offset {
            0x00 => {
                self.kr = value;
                match value & 0xFFFF {
                    0x5555 => {}
                    0xAAAA => { self.counter = self.rlr & 0xFFF; self.last_tick = INSTRUCTION_COUNT.load(Ordering::Relaxed); }
                    0xCCCC => { self.enabled = true; self.counter = self.rlr & 0xFFF; self.last_tick = INSTRUCTION_COUNT.load(Ordering::Relaxed); }
                    _ => {}
                }
            }
            0x04 => { if self.kr == 0x5555 { self.pr = value & 7; self.sr |= 1; self.sr_tick = INSTRUCTION_COUNT.load(Ordering::Relaxed); } }
            0x08 => { if self.kr == 0x5555 { self.rlr = value & 0xFFF; self.sr |= 2; self.sr_tick = INSTRUCTION_COUNT.load(Ordering::Relaxed); } }
            _ => {}
        }
    }
}
