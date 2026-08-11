use wasm_bindgen::prelude::*;

use crate::system::WasmSystem;

/// Interrupt-dispatch policy state that lives in Rust so cli.mjs and
/// emulator.js share ONE implementation (they kept drifting apart — e.g. the
/// xPSR-restore bug was cli-only). Unicorn register/memory transport stays in
/// JS (CPU not reachable from Rust); this module owns everything else:
/// batch budget, the SVC frame mirror, and the Cortex-M frame layout.
#[derive(Default)]
pub struct IntrDispatch {
    /// Mirrored SVC frames (Cortex-M ABI). Depth-capped like the old JS
    /// `svcStack`. Each frame is written to the real stack too, so handler
    /// code can inspect it — this mirror is what the JS catch pops on
    /// `bx lr` to EXC_RETURN (Unicorn faults fetching 0xFFFFFFFx).
    svc_stack: Vec<SvcFrame>,
    /// IRQs served so far this batch (reset by step/step_batch). Caps the
    /// dispatch loop at 64 per batch, matching the old JS for-loop.
    budget: u32,
}

#[derive(Clone, Copy)]
struct SvcFrame {
    sp: u32,
    r0: u32,
    r1: u32,
    r2: u32,
    r3: u32,
    r12: u32,
    lr: u32,
    pc: u32,
    xpsr: u32,
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

/// Enter an SVC: push a mirror of the interrupted context (depth-capped at 8)
/// and return the 32-byte Cortex-M exception frame to write to the real stack.
/// Empty vec when the cap is hit (SVC ignored, like the old JS guard).
/// Frame layout: [xpsr, pc, lr, r12, r3, r2, r1, r0] little-endian.
#[wasm_bindgen]
pub fn intr_svc_enter(
    r0: u32, r1: u32, r2: u32, r3: u32, r12: u32,
    lr: u32, pc: u32, xpsr: u32, sp: u32,
) -> Vec<u8> {
    let mut intr = crate::sys().intr.borrow_mut();
    if intr.svc_stack.len() >= 8 {
        return Vec::new();
    }
    intr.svc_stack.push(SvcFrame {
        sp, r0, r1, r2, r3, r12, lr, pc, xpsr,
    });
    let mut frame = Vec::with_capacity(32);
    for v in [xpsr, pc, lr, r12, r3, r2, r1, r0] {
        frame.extend_from_slice(&v.to_le_bytes());
    }
    frame
}

/// Pop the top SVC mirror: [r0, r1, r2, r3, r12, lr, pc, sp]. Empty vec when
/// the stack is empty (no SVC in flight).
#[wasm_bindgen]
pub fn intr_svc_leave() -> Vec<u32> {
    let mut intr = crate::sys().intr.borrow_mut();
    match intr.svc_stack.pop() {
        Some(f) => vec![f.r0, f.r1, f.r2, f.r3, f.r12, f.lr, f.pc, f.sp],
        None => Vec::new(),
    }
}

/// Number of SVC frames currently in flight (guard for the JS catch).
#[wasm_bindgen]
pub fn intr_svc_depth() -> u32 {
    crate::sys().intr.borrow().svc_stack.len() as u32
}
