use wasm_bindgen::prelude::*;

use crate::system::WasmSystem;

/// Interrupt-dispatch policy state: the per-batch 64-IRQ budget shared by
/// every driver loop (cli.mjs, emulator.js, the native backend). SVC needs
/// no mirror anymore: the native core stacks_take/returns on the real stack
/// inline (there is no second CPU that can't do Cortex-M entry/return).
#[derive(Default)]
pub struct IntrDispatch {
    /// IRQs served so far this batch (reset by step/step_batch). Caps the
    /// dispatch loop at 64 per batch so one hot IRQ can't starve others.
    budget: u32,
}

impl IntrDispatch {
    pub fn reset_budget(&mut self) {
        self.budget = 0;
    }

    /// Return the next pending IRQ number, honoring the 64-IRQ-per-batch
    /// fairness cap (-255 when exhausted or nothing pending).
    pub fn next(&mut self, sys: &WasmSystem) -> i32 {
        if self.budget >= 64 {
            return -255;
        }
        self.budget += 1;
        sys.p.nvic.borrow_mut()
            .get_next_pending_intr()
            .unwrap_or(-255)
    }
}

/// Next pending IRQ within the batch budget (like get_next_pending_interrupt,
/// but capped at 64 per step/step_batch so one hot IRQ can't starve others).
#[wasm_bindgen]
pub fn intr_next() -> i32 {
    let sys = crate::sys();
    sys.intr.borrow_mut().next(sys)
}
