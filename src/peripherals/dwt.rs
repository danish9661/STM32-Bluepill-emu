use crate::system::System;
use super::Peripheral;

/// Minimal ARM DWT (Data Watchpoint and Trace) cycle counter.
///
/// Arduino-STM32's `micros()` and `TwoWire::recoverBus()` spin on
/// `DWT->CYCCNT` (0xE0001004), which QEMU/Unicorn implements but this
/// peripheral model did not — reads returned 0, so any CYCCNT delay loop
/// hung forever on the native Rust-CPU path. CYCCNT tracks the global
/// instruction counter (1 instr = 1 cycle, matching the TIM/ADC model);
/// CTRL.CYCCNTENA is stored but not gated (lenient, like the rest of the
/// model). Other DWT registers read 0 / ignore writes.
#[derive(Default)]
pub struct Dwt {
    ctrl: u32,
    /// Written CYCCNT base: reads return INSTRUCTION_COUNT + offset, so a
    /// guest write takes effect immediately and then keeps counting.
    cyccnt_offset: i64,
}

impl Dwt {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "DWT" { Some(Box::new(Self::default())) } else { None }
    }

    fn cyccnt(&self) -> u32 {
        let count = crate::system::INSTRUCTION_COUNT
            .load(std::sync::atomic::Ordering::Relaxed) as i64;
        count.wrapping_add(self.cyccnt_offset) as u32
    }
}

impl Peripheral for Dwt {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.ctrl,
            0x04 => self.cyccnt(),
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.ctrl = value,
            0x04 => {
                let count = crate::system::INSTRUCTION_COUNT
                    .load(std::sync::atomic::Ordering::Relaxed) as i64;
                self.cyccnt_offset = (value as i64).wrapping_sub(count);
            }
            _ => {}
        }
    }
}
