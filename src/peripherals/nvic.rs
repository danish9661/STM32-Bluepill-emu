use crate::system::{System, INSTRUCTION_COUNT};
use std::sync::atomic::Ordering;
use super::Peripheral;

const IRQ_COUNT: usize = 97;
const IRQ_OFFSET: i32 = 16;
const REG_WORDS: usize = (IRQ_COUNT + 31) / 32;

pub mod irq {
    pub const PENDSV: i32 = -2;
    pub const SYSTICK: i32 = -1;
}

#[derive(Clone)]
pub struct Nvic {
    pub systick_period: Option<u32>,
    pub last_systick_trigger: u64,
    pending: u128,
    in_interrupt: bool,
    enable: [u32; REG_WORDS],
    pending_reg: [u32; REG_WORDS],
    active: [u32; REG_WORDS],
    priority: [u8; IRQ_COUNT],
}

impl Default for Nvic {
    fn default() -> Self {
        Self {
            systick_period: None,
            last_systick_trigger: 0,
            pending: 0,
            in_interrupt: false,
            enable: [0; REG_WORDS],
            pending_reg: [0; REG_WORDS],
            active: [0; REG_WORDS],
            priority: [0; IRQ_COUNT],
        }
    }
}

impl Nvic {
    /// Map irq to pending_reg index (IRQ-based, no offset)
    fn irq_reg_idx(irq: i32) -> Option<(usize, u32)> {
        if irq < 0 { return None; }
        let idx = (irq as usize) / 32;
        let mask = 1u32 << (irq as usize % 32);
        if idx >= REG_WORDS { None } else { Some((idx, mask)) }
    }

    pub fn set_intr_pending(&mut self, irq: i32) {
        if irq < 0 {
            // System exception
            self.pending |= 1u128 << (IRQ_OFFSET + irq);
        } else if let Some((idx, mask)) = Self::irq_reg_idx(irq) {
            self.pending |= 1u128 << (IRQ_OFFSET + irq);
            self.pending_reg[idx] |= mask;
        }
    }

    pub fn clear_pending(&mut self, irq: i32) {
        self.pending &= !(1u128 << (IRQ_OFFSET + irq));
        if let Some((idx, mask)) = Self::irq_reg_idx(irq) {
            self.pending_reg[idx] &= !mask;
        }
    }

    pub fn has_pending(&self) -> bool { self.pending != 0 }

    pub fn get_pending_vector(&self) -> u32 {
        if self.pending != 0 {
            let bit = self.pending.trailing_zeros();
            bit - IRQ_OFFSET as u32
        } else {
            0
        }
    }

    pub fn get_and_clear_next_intr_pending(&mut self) -> Option<i32> {
        if self.pending != 0 {
            let bit = self.pending.trailing_zeros();
            let irq = (bit as i32) - IRQ_OFFSET;
            // External IRQs (bit >= 16) need ISER enable; system exceptions always fire
            if bit >= 16 {
                let irq_num = irq as usize;
                let idx = irq_num / 32;
                let mask = 1u32 << (irq_num % 32);
                if self.enable[idx] & mask == 0 {
                    self.pending &= !(1u128 << bit);
                    self.pending_reg[idx] &= !mask;
                    return None;
                }
            }
            self.pending &= !(1u128 << bit);
            if irq >= 0 {
                let idx = (irq as usize) / 32;
                let mask = 1u32 << (irq as usize % 32);
                self.pending_reg[idx] &= !mask;
                self.active[idx] |= mask;
            }
            Some(irq)
        } else {
            None
        }
    }

    pub fn maybe_set_systick_intr_pending(&mut self) {
        if let Some(systick_period) = self.systick_period {
            let n = INSTRUCTION_COUNT.load(Ordering::Relaxed);
            let delta = n - self.last_systick_trigger;
            if delta > systick_period as u64 {
                self.last_systick_trigger = n;
                self.set_intr_pending(irq::SYSTICK);
            }
        }
    }

    pub fn is_in_interrupt(&self) -> bool {
        self.in_interrupt
    }

    pub fn set_in_interrupt(&mut self, v: bool) {
        self.in_interrupt = v;
    }
}

impl Peripheral for Nvic {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00..=0x1C if offset < 4 * REG_WORDS as u32 => {
                let i = (offset / 4) as usize;
                self.enable[i]
            }
            0x80..=0x9C if offset < 0x80 + 4 * REG_WORDS as u32 => {
                let i = ((offset - 0x80) / 4) as usize;
                self.enable[i]
            }
            0x100..=0x11C if offset < 0x100 + 4 * REG_WORDS as u32 => {
                let i = ((offset - 0x100) / 4) as usize;
                self.pending_reg[i]
            }
            0x200..=0x21C if offset < 0x200 + 4 * REG_WORDS as u32 => {
                // IABR - Active Bit Register
                let i = ((offset - 0x200) / 4) as usize;
                self.active[i]
            }
            0x280..=0x29C if offset < 0x280 + 4 * REG_WORDS as u32 => {
                // IABR alternate alias
                let i = ((offset - 0x280) / 4) as usize;
                self.active[i]
            }
            0x300..=0x4EF => {
                let byte_idx = (offset - 0x300) as usize;
                if byte_idx < IRQ_COUNT {
                    self.priority[byte_idx] as u32
                } else { 0 }
            }
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00..=0x1C if offset < 4 * REG_WORDS as u32 => {
                let i = (offset / 4) as usize;
                let was = self.enable[i];
                self.enable[i] |= value;
                let newly_enabled = self.enable[i] & !was;
                let newly_pending = self.pending_reg[i] & newly_enabled;
                for b in 0..32 {
                    if newly_pending & (1 << b) != 0 {
                        self.pending |= 1u128 << (IRQ_OFFSET as u32 + i as u32 * 32 + b) as u128;
                    }
                }
            }
            0x80..=0x9C if offset < 0x80 + 4 * REG_WORDS as u32 => {
                let i = ((offset - 0x80) / 4) as usize;
                self.enable[i] &= !value;
            }
             0x100..=0x11C if offset < 0x100 + 4 * REG_WORDS as u32 => {
                // ISPR - Set Pending Register
                let i = ((offset - 0x100) / 4) as usize;
                let new_pending = value & !self.pending_reg[i];
                self.pending_reg[i] |= value;
                for b in 0..32 {
                    if new_pending & (1 << b) != 0 {
                        self.pending |= 1u128 << (IRQ_OFFSET as u32 + i as u32 * 32 + b) as u128;
                    }
                }
            }
            0x180..=0x19C if offset < 0x180 + 4 * REG_WORDS as u32 => {
                // ICPR - Clear Pending Register
                let i = ((offset - 0x180) / 4) as usize;
                let cleared = self.pending_reg[i] & value;
                self.pending_reg[i] &= !value;
                for b in 0..32 {
                    if cleared & (1 << b) != 0 {
                        self.pending &= !(1u128 << (IRQ_OFFSET as u32 + i as u32 * 32 + b) as u128);
                    }
                }
            }
            0x200..=0x21C if offset < 0x200 + 4 * REG_WORDS as u32 => {
                // IABR - Active Bit Register (read-only by software writes)
            }
            0x300..=0x4EF => {
                let byte_idx = (offset - 0x300) as usize;
                if byte_idx < IRQ_COUNT {
                    self.priority[byte_idx] = (value & 0xFF) as u8;
                }
            }
            _ => {}
        }
    }
}

pub struct NvicWrapper;

impl NvicWrapper {
    pub fn new(_name: &str) -> Option<Box<dyn Peripheral>> {
        None // NVIC is handled by shortcut in Peripherals::read/write, not as a peripheral slot
    }
}
