pub mod regs;
pub mod mem;
pub(crate) mod thumb;
// NOTE: vendor `tests` module excluded for now — its harness needs firmware
// .bins and helper fns that don't exist in this tree (monox SVD, blinky.bin,
// init_svd_for_test, WasmCpu). Port selected tests once the core boots here.
// #[cfg(test)]
// mod tests;
#[cfg(test)]
mod smoke;
#[cfg(test)]
mod core_tests;
pub use regs::Regs;
pub use mem::Memory;
use crate::system::WasmSystem;

/// EXC_RETURN values we support (thread mode). F1 (return to handler) is
/// nested-interrupt territory and faults loudly for now.
pub const EXC_RETURN_MSP: u32 = 0xFFFFFFF9;
pub const EXC_RETURN_PSP: u32 = 0xFFFFFFFD;
pub const EXC_RETURN_HANDLER: u32 = 0xFFFFFFF1;

/// The CPU stopped because of this (unknown instruction, BKPT, branch to
/// ARM state, ...). `pc` is the faulting instruction address (without thumb
/// bit); `op1`/`op2` are the raw halfwords; `len` is 2 or 4.
#[derive(Clone, Copy, Debug, Default)]
pub struct CpuFault {
    pub pc: u32,
    pub op1: u16,
    pub op2: u16,
    pub len: u8,
}

/// Saved IT-block state across an exception (pushed on entry, popped on
/// return — the handler runs with a clean ITSTATE, per ARM).
#[derive(Clone, Copy, Debug, Default)]
struct SavedIt {
    cond: u8,
    mask: u8,
    n: u8,
    idx: u8,
}

pub struct Cpu {
    pub regs: Regs,
    pub cycles: u64,
    pub fault: Option<CpuFault>,
    // IT-block state. `n == 0` means no active block. `idx` counts consumed
    // instructions (1-based). See the IT rule in thumb.rs (`it_ok`): j>=2
    // uses `cond` iff mask bit (5-j) equals cond bit 0, else the inverse.
    pub it_cond: u8,
    pub it_mask: u8,
    pub it_n: u8,
    pub it_idx: u8,
    /// Exception number currently executing (0 = thread mode). Mirrors IPSR.
    pub ipsr: u32,
    /// IRQ numbers of entered exceptions (for NVIC active-bit hygiene).
    exc_stack: Vec<i32>,
    /// Saved IT states, parallel to exc_stack.
    it_stack: Vec<SavedIt>,
    /// Break `run()` when the model has a pending interrupt, so a driver
    /// with guest exception delivery can take it. Off by default: polling
    /// firmware (and the current JS wasm driver, which has no ISR pump)
    /// must run full budgets exactly like the Unicorn path, where pending
    /// model IRQs never stop `emu_start`.
    pub deliver_irqs: bool,
    /// Halted in WFI/WFE (low-power). JS advances virtual time and wakes
    /// via `wake()` when an interrupt is pending. Only set when
    /// `deliver_irqs` is on; otherwise WFI is a nop.
    pub sleeping: bool,
    /// DSP extension present (Cortex-M4). Cortex-M3 drivers set false, which
    /// faults SMLAXY/SMULXY/SMLAD/SMUAD/SMULW/SMLAW as UNDEFINED. Defaults
    /// true so the M4-verified behavior is unchanged.
    pub dsp: bool,
    /// Predicated 16-bit data-processing must not update APSR (ARM rule:
    /// 16-bit instructions in an IT block, other than CMP/CMN/TST, do not
    /// set flags). Set fresh by exec16/exec32 on every instruction; helpers
    /// (nz/add_flags/sub_flags) and T1 carry lines consult it, CMP/CMN/TST
    /// arms and all T2 arms ignore it.
    pub it_suppress: bool,
}

impl Cpu {
    pub fn new(sp: u32, pc: u32) -> Self {
        Self {
            regs: Regs::new(sp, pc),
            cycles: 0,
            fault: None,
            it_cond: 0,
            it_mask: 0,
            it_n: 0,
            it_idx: 0,
            ipsr: 0,
            exc_stack: Vec::new(),
            it_stack: Vec::new(),
            deliver_irqs: false,
            sleeping: false,
            dsp: true,
            it_suppress: false,
        }
    }
    pub fn reset(&mut self, sp: u32, pc: u32) {
        self.regs = Regs::new(sp, pc);
        self.fault = None;
        self.it_cond = 0;
        self.it_mask = 0;
        self.it_n = 0;
        self.it_idx = 0;
        self.ipsr = 0;
        self.exc_stack.clear();
        self.it_stack.clear();
        self.sleeping = false;
        // NOTE: `dsp` is configuration, not CPU state — preserved across reset.
        // it_suppress is per-instruction transient (recomputed at each entry).
        self.it_suppress = false;
    }

    /// Current stack pointer (r13 always mirrors it).
    #[inline]
    pub fn sp(&self) -> u32 {
        self.regs.r[13]
    }

    /// Read the MSP (banked). In handler mode MSP == r13; in thread+MSP
    /// mode r13 == msp too. Only thread+PSP mode differs.
    pub fn read_msp(&self) -> u32 {
        if self.ipsr == 0 && self.regs.control & 2 != 0 {
            self.regs.msp
        } else {
            self.regs.r[13]
        }
    }
    /// Read the PSP (banked).
    pub fn read_psp(&self) -> u32 {
        if self.ipsr == 0 && self.regs.control & 2 != 0 {
            self.regs.r[13]
        } else {
            self.regs.psp
        }
    }
    /// Write the MSP (banked): updates r13 too when MSP is current.
    pub fn write_msp(&mut self, v: u32) {
        self.regs.msp = v;
        if self.ipsr != 0 || self.regs.control & 2 == 0 {
            self.regs.r[13] = v;
        }
    }
    /// Write the PSP (banked): updates r13 too when PSP is current.
    /// This is the FreeRTOS task-switch primitive (`msr psp, rX` in the
    /// PendSV handler while in handler mode only updates the bank).
    pub fn write_psp(&mut self, v: u32) {
        self.regs.psp = v;
        if self.ipsr == 0 && self.regs.control & 2 != 0 {
            self.regs.r[13] = v;
        }
    }

    /// Take an exception: stack the context, load the handler from the
    /// vector table (via VTOR), set EXC_RETURN. Works for system exceptions
    /// (negative irq) and external IRQs. No nesting in v1 (only called from
    /// thread mode), but the stacking is fully hardware-shaped so ISRs run
    /// unmodified, including FreeRTOS SVC/PendSV/SysTick handlers.
    pub fn take_exception(&mut self, sys: &WasmSystem, mem: &mut dyn Memory, irq: i32) {
        let vector = (16 + irq) as u32;
        // Exception entry wakes the core (WFI sleeps until an interrupt is
        // delivered; without this a sleeping core never resumes, since the
        // driver pumps take_exception for every dispatch).
        // Save IT state; the handler starts with a clean ITSTATE.
        self.it_stack.push(SavedIt {
            cond: self.it_cond,
            mask: self.it_mask,
            n: self.it_n,
            idx: self.it_idx,
        });
        self.it_n = 0;
        self.it_idx = 0;
        self.sleeping = false;
        self.exc_stack.push(irq);
        // Bank the thread stack, then run the handler on MSP. The frame
        // goes onto the CURRENT stack (PSP if thread+PSP, else MSP) — this
        // is what makes FreeRTOS task stacks work.
        let was_psp = self.ipsr == 0 && self.regs.control & 2 != 0;
        if was_psp {
            self.regs.psp = self.regs.r[13];
        } else if self.ipsr == 0 {
            self.regs.msp = self.regs.r[13];
        }
        let mut sp = self.regs.r[13];
        // Push xPSR (T-bit set), PC, LR, R12, R3-R0.
        let xpsr = self.regs.xpsr | 0x01000000;
        sp = sp.wrapping_sub(4);
        mem.write32(sp, xpsr);
        sp = sp.wrapping_sub(4);
        mem.write32(sp, self.regs.r[15]);
        sp = sp.wrapping_sub(4);
        mem.write32(sp, self.regs.r[14]);
        sp = sp.wrapping_sub(4);
        mem.write32(sp, self.regs.r[12]);
        sp = sp.wrapping_sub(4);
        mem.write32(sp, self.regs.r[3]);
        sp = sp.wrapping_sub(4);
        mem.write32(sp, self.regs.r[2]);
        sp = sp.wrapping_sub(4);
        mem.write32(sp, self.regs.r[1]);
        sp = sp.wrapping_sub(4);
        mem.write32(sp, self.regs.r[0]);
        // Handler mode always runs on MSP.
        self.regs.r[13] = self.regs.msp;
        // LR = EXC_RETURN selecting the thread stack we came from.
        self.regs.r[14] = if was_psp { EXC_RETURN_PSP } else { EXC_RETURN_MSP };
        // ^ BUG: r13 must be the POST-PUSH sp, not stale msp! Fix below.
        self.regs.r[13] = sp;
        self.regs.msp = sp;
        // Hardware also advances the THREAD bank past the pushed frame, so a
        // later `mrs psp` (PendSV save) points BELOW the entry frame and the
        // stmdb doesn't overwrite it. Without this the entry frame is
        // clobbered and the switch-back unstacks garbage (FreeRTOS slide).
        if was_psp {
            self.regs.psp = sp;
        }
        self.ipsr = vector;
        // NOTE: the vendor NVIC had set_in_interrupt() here; this tree's
        // NVIC tracks the active stack via the get_next_pending_intr() push /
        // clear_current_interrupt() pop pair instead, so there is nothing to
        // set (take_exception runs with the entry already pushed by the pop,
        // or standalone for SVC with an empty stack — both consistent).
        // Load handler PC through VTOR (model SCB, default 0x08000000).
        let vtor = sys.p.read(sys, 0xE000ED08, 4);
        let handler = mem.read32(vtor.wrapping_add(vector * 4));
        self.regs.r[15] = handler | 1;
    }

    /// Perform an exception return for an EXC_RETURN value in `exc`.
    /// Returns false (with fault recorded) for unsupported returns.
    pub fn exception_return(
        &mut self,
        sys: &WasmSystem,
        mem: &mut dyn Memory,
        exc: u32,
        pc: u32,
    ) -> bool {
        if exc == EXC_RETURN_HANDLER {
            // Return to handler mode (nested) — not supported in v1.
            self.fault = Some(CpuFault { pc, op1: 0x4770, op2: 0, len: 2 });
            return false;
        }
        if exc != EXC_RETURN_MSP && exc != EXC_RETURN_PSP {
            self.fault = Some(CpuFault { pc, op1: 0x4770, op2: 0, len: 2 });
            return false;
        }
        // Unstack from the bank selected by EXC_RETURN (using CURRENT bank
        // values — a PendSV task switch updates PSP mid-handler).
        let mut sp = if exc == EXC_RETURN_PSP { self.regs.psp } else { self.regs.msp };
        // In handler mode r13 == MSP; if returning to MSP it must match.
        // (If a buggy handler moved MSP, trust the bank per ARM.)
        let r0 = mem.read32(sp);
        let r1 = mem.read32(sp.wrapping_add(4));
        let r2 = mem.read32(sp.wrapping_add(8));
        let r3 = mem.read32(sp.wrapping_add(12));
        let r12 = mem.read32(sp.wrapping_add(16));
        let lr = mem.read32(sp.wrapping_add(20));
        let retpc = mem.read32(sp.wrapping_add(24));
        let xpsr = mem.read32(sp.wrapping_add(28));
        sp = sp.wrapping_add(32);
        self.regs.r[0] = r0;
        self.regs.r[1] = r1;
        self.regs.r[2] = r2;
        self.regs.r[3] = r3;
        self.regs.r[12] = r12;
        self.regs.r[14] = lr;
        // Restore flags (APSR) + IT/ICI bits live in xPSR; T-bit stays set.
        self.regs.xpsr = (xpsr & 0xF8000000) | 0x01000000;
        self.regs.r[13] = sp;
        if exc == EXC_RETURN_PSP {
            self.regs.psp = sp;
        } else {
            self.regs.msp = sp;
        }
        // Exception return selects the thread stack AND updates CONTROL.SPSEL
        // to match (hardware keeps them coherent; without this every
        // CONTROL-gated bank decision after the first return is wrong and
        // PendSV saves to a stale PSP — the FreeRTOS wedge). Bit0
        // (privilege) is preserved.
        if exc == EXC_RETURN_PSP {
            self.regs.control |= 2;
        } else {
            self.regs.control &= !2;
        }
        // Restore the pre-exception IT state.
        if let Some(saved) = self.it_stack.pop() {
            self.it_cond = saved.cond;
            self.it_mask = saved.mask;
            self.it_n = saved.n;
            self.it_idx = saved.idx;
        }
        let irq = self.exc_stack.pop();
        self.ipsr = 0;
        // Balance the active-priority push that get_next_pending_intr()
        // performed on entry (mirrors finish_interrupt(); a standalone SVC
        // pops an empty stack, which is a safe no-op).
        sys.p.nvic.borrow_mut().clear_current_interrupt();
        // SysTick debt drain (mirrors finish_interrupt()): re-pend each
        // unconsumed 1ms tick so millis() tracks instruction time.
        if irq == Some(crate::peripherals::nvic::irq::SYSTICK) {
            while sys.p.nvic.borrow_mut().systick_take() {}
        }
        // Chained PendSV/SVC tail? No tail-chaining in v1; the run loop
        // delivers the next pending exception on the next iteration.
        self.regs.r[15] = retpc | 1;
        // NOTE: no even-retpc fault here. FreeRTOS's M4 port deliberately
        // stores the task entry with bit0 CLEAR (`bic r1, #1` in
        // pxPortInitialiseStack) and relies on exception return forcing
        // Thumb state; Unicorn accepts this and the firmware is proven on
        // it, so we force |1 like hardware does for the PC load.
        true
    }

    pub fn run(&mut self, sys: &WasmSystem, mem: &mut dyn Memory, budget: u32) -> u32 {
        let mut done = 0;
        while done < budget {
            if self.fault.is_some() {
                break;
            }
            if self.sleeping {
                break;
            }
            let pc = self.regs.r[15] & !1;
            let op = mem.read16(pc);
            let l = thumb::len(op);
            let ok = if l == 2 {
                thumb::exec16(self, sys, mem, op, pc)
            } else {
                let o2 = mem.read16(pc + 2);
                thumb::exec32(self, sys, mem, op, o2, pc)
            };
            if !ok {
                if self.fault.is_none() {
                    self.fault = Some(CpuFault { pc, op1: op, op2: 0, len: 2 });
                }
                break;
            }
            done += 1;
            // Keep the inactive... no — keep the CURRENT stack bank in sync
            // with r13 after every thread-mode instruction. PUSH/POP/ADD-SP
            // and LDM/STM writeback move r13 directly; without this the bank
            // goes stale and the next `mrs psp` (PendSV context switch) saves
            // r4-r11 at the wrong address, stranding the live stack (this
            // wedged FreeRTOS: high_top saved as stale_psp-32). Handler mode
            // (ipsr != 0) is skipped: take_exception/exception_return manage
            // the banks explicitly there, and r13 == MSP throughout.
            if self.ipsr == 0 {
                if self.regs.control & 2 != 0 {
                    self.regs.psp = self.regs.r[13];
                } else {
                    self.regs.msp = self.regs.r[13];
                }
            }
            self.cycles += 1;
            // Inline interrupt delivery (no JS pump needed): take the next
            // deliverable exception when in thread mode with PRIMASK clear.
            // (Unicorn needs the memWriteHook+processInterrupts dance because
            // it cannot do Cortex-M entry/return; here stacking is exact, so
            // the mid-`str` PENDSVSET hazard of AGENTS.md §9 cannot occur —
            // the store completes, PC advances, then we stack the next PC.)
            if self.deliver_irqs && self.ipsr == 0 && self.regs.primask == 0 {
                let pending = sys.p.nvic.borrow().has_pending();
                if pending {
                    // Bind first: `if let` would extend the borrow_mut guard
                    // through the body and take_exception would re-borrow.
                    // get_next_pending_intr() pops the highest-priority
                    // pending IRQ (PRIMASK/BASEPRI via the INTR_MASK statics
                    // the driver maintains), clears it and pushes the active
                    // stack entry that exception_return balances.
                    let next = sys.p.nvic.borrow_mut().get_next_pending_intr();
                    if let Some(irq) = next {
                        self.take_exception(sys, mem, irq);
                    }
                }
            }
        }
        done
    }
}
