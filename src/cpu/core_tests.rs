//! Core-level proofs the firmware suite can't cover: WFI sleep/wake and
//! PSP task switching (the FreeRTOS primitives), plus inline IRQ delivery.
//! Synthetic programs in a small FlatMemory; no firmware image needed.

use super::{
    mem::{FlatMemory, Memory},
    Cpu,
};
use crate::{init, set_intr_masks, sys};

/// WFI with nothing pending halts the run; a later dispatch (the driver's
/// take_exception path) wakes the core and the handler runs to return.
#[test]
fn wfi_sleeps_and_wakes_on_dispatch() {
    let _held = crate::test_util::lock();
    init();
    let sys = sys();
    // IRQ6 vector -> 2-instruction handler (movs r0,#0x42; bx lr).
    // Thread: wfi, then a landing pad (movs r1,#7).
    let mut mem = FlatMemory::new(0x100, 0x100);
    mem.load(&[0x81u8, 0x00, 0x00, 0x08], 0x08000000 + 22 * 4);
    mem.load(&[0x42, 0x20, 0x70, 0x47], 0x08000080);
    mem.load(&[0x30, 0xBF, 0x07, 0x21], 0x080000C0);
    let mut cpu = Cpu::new(0x20000100, 0x080000C1);
    cpu.dsp = false;
    cpu.deliver_irqs = true;
    set_intr_masks(0, 0);

    // Nothing pending: exactly the wfi executes, then the run halts asleep.
    let n = cpu.run(sys, &mut mem, 10);
    assert!(cpu.fault.is_none(), "{:?}", cpu.fault);
    assert_eq!(n, 1);
    assert!(cpu.sleeping, "wfi should sleep with nothing pending");
    assert_eq!(cpu.regs.r[15] & !1, 0x080000C2, "resume is staged after wfi");

    // Driver-style dispatch: pend IRQ6, pop it (clearing the NVIC pending
    // bit, like intr_next does), enter (must wake), run to return.
    sys.p.nvic.borrow_mut().enable_irq(6);
    sys.p.nvic.borrow_mut().set_intr_pending(6);
    let irq = sys.p.nvic.borrow_mut().get_next_pending_intr();
    assert_eq!(irq, Some(6));
    cpu.take_exception(sys, &mut mem, irq.unwrap());
    assert!(!cpu.sleeping, "exception entry must wake the core");
    assert_eq!(cpu.ipsr, 22);
    let mut hd = 0u32;
    while cpu.ipsr != 0 && hd < 100 && cpu.fault.is_none() {
        set_intr_masks(0, 0);
        hd += cpu.run(sys, &mut mem, 1);
    }
    assert!(cpu.fault.is_none(), "{:?}", cpu.fault);
    assert_eq!(hd, 2, "handler body (movs; bx lr) ran exactly once");
    assert_eq!(cpu.ipsr, 0);
    assert_eq!(cpu.regs.msp, 0x20000100, "MSP restored past the frame");
    // Thread resumes into the landing pad.
    cpu.run(sys, &mut mem, 1);
    assert_eq!(cpu.regs.r[1], 7);
    assert_eq!(cpu.regs.r[15] & !1, 0x080000C4);
}

/// Nested preemption: a low-priority handler pends a higher-priority IRQ
/// mid-handler (EXTI SWIER, like real firmware); with inline delivery the
/// high IRQ preempts before the low handler finishes. The shift-register log
/// reads 0x123 only under true preemption (0x132 if merely sequential).
/// Also observes LR == EXC_RETURN_HANDLER on the nested take.
#[test]
fn nested_irq_preempts_mid_handler() {
    let _held = crate::test_util::lock();
    init();
    let sys = sys();
    let mut mem = FlatMemory::new(0x100, 0x200);
    // Outer (IRQ7) @ 0x80: log(1); SWIER=bit0 (pend line 0); log(3); bx lr.
    mem.load(
        &[
            0x07, 0x48, 0x01, 0x68, 0x09, 0x01, 0x01, 0x31, 0x01, 0x60, //
            0x06, 0x48, 0x01, 0x21, 0x01, 0x60, //
            0x03, 0x48, 0x01, 0x68, 0x09, 0x01, 0x03, 0x31, 0x01, 0x60, //
            0x70, 0x47, 0x00, 0xBF, 0x00, 0xBF, //
            0x70, 0x01, 0x00, 0x20, // SEQ @ 0x20000170
            0x10, 0x04, 0x01, 0x40, // SWIER @ 0x40010410
        ],
        0x08000080,
    );
    // Inner (IRQ6) @ 0xB0: log(2); bx lr.
    mem.load(
        &[0x02, 0x48, 0x01, 0x68, 0x09, 0x01, 0x02, 0x31, 0x01, 0x60, 0x70, 0x47, 0x70, 0x01, 0x00, 0x20],
        0x080000B0,
    );
    mem.load(&[0xB1u8, 0x00, 0x00, 0x08], 0x08000000 + 22 * 4); // IRQ6 -> inner
    mem.load(&[0x81u8, 0x00, 0x00, 0x08], 0x08000000 + 23 * 4); // IRQ7 -> outer
    let mut cpu = Cpu::new(0x200001C0, 0x080000C1);
    cpu.dsp = false;
    cpu.deliver_irqs = true;
    set_intr_masks(0, 0);
    // EXTI IMR lines 0+1; IRQ6 high prio (0x40), IRQ7 low (0xC0).
    sys.p.write(sys, 0x40010400, 4, 0x3);
    sys.p.write(sys, 0xE000E404, 4, 0xC0400000);
    sys.p.nvic.borrow_mut().enable_irq(6);
    sys.p.nvic.borrow_mut().enable_irq(7);
    // Pend the LOW irq first (via the model SWIER path, like firmware).
    sys.p.write(sys, 0x40010410, 4, 1 << 1);
    let irq = sys.p.nvic.borrow_mut().get_next_pending_intr();
    assert_eq!(irq, Some(7));
    cpu.take_exception(sys, &mut mem, 7);
    assert_eq!(cpu.ipsr, 23);
    // Single-step: watch the inner take happen mid-handler.
    let mut saw_nested_take = false;
    let mut guard = 0u32;
    while cpu.ipsr != 0 && guard < 200 && cpu.fault.is_none() {
        set_intr_masks(0, 0);
        cpu.run(sys, &mut mem, 1);
        guard += 1;
        if cpu.ipsr == 22 && !saw_nested_take {
            saw_nested_take = true;
            assert_eq!(
                cpu.regs.r[14], 0xFFFFFFF1,
                "nested entry must use EXC_RETURN_HANDLER"
            );
        }
    }
    assert!(cpu.fault.is_none(), "{:?}", cpu.fault);
    assert!(saw_nested_take, "high IRQ never preempted the low handler");
    assert_eq!(cpu.ipsr, 0);
    assert_eq!(mem.read32(0x20000170), 0x123, "log order must be outer,inner,outer");
    assert!(!sys.p.nvic.borrow().is_in_interrupt(), "active stack balanced");
    assert_eq!(cpu.regs.r[13], 0x200001C0, "MSP fully unwound");
    assert_eq!(cpu.regs.msp, 0x200001C0);
}

/// SVC inside a handler balances the NVIC active stack: after the inner
/// return the outer entry must still be active (observed via
/// is_in_interrupt), and the end state is fully unwound.
#[test]
fn svc_in_handler_balances_active() {
    let _held = crate::test_util::lock();
    init();
    let sys = sys();
    let mut mem = FlatMemory::new(0x100, 0x200);
    // Outer (IRQ7) @ 0x80: log(1); svc #0; log(3); bx lr.
    mem.load(
        &[
            0x05, 0x48, 0x01, 0x68, 0x09, 0x01, 0x01, 0x31, 0x01, 0x60, //
            0x00, 0xDF, // svc #0
            0x02, 0x48, 0x01, 0x68, 0x09, 0x01, 0x03, 0x31, 0x01, 0x60, //
            0x70, 0x47, //
            0x70, 0x01, 0x00, 0x20, // SEQ @ 0x20000170
        ],
        0x08000080,
    );
    // SVC handler @ 0xA0: log(2); bx lr.
    mem.load(
        &[0x02, 0x48, 0x01, 0x68, 0x09, 0x01, 0x02, 0x31, 0x01, 0x60, 0x70, 0x47, 0x70, 0x01, 0x00, 0x20],
        0x080000A0,
    );
    mem.load(&[0x81u8, 0x00, 0x00, 0x08], 0x08000000 + 23 * 4); // IRQ7 -> outer
    mem.load(&[0xA1u8, 0x00, 0x00, 0x08], 0x08000000 + 11 * 4); // SVCall -> svc handler
    let mut cpu = Cpu::new(0x200001C0, 0x080000C1);
    cpu.dsp = false;
    cpu.deliver_irqs = true;
    set_intr_masks(0, 0);
    sys.p.write(sys, 0x40010400, 4, 0x2); // IMR line 1
    sys.p.nvic.borrow_mut().enable_irq(7);
    sys.p.write(sys, 0x40010410, 4, 1 << 1);
    let irq = sys.p.nvic.borrow_mut().get_next_pending_intr();
    assert_eq!(irq, Some(7));
    cpu.take_exception(sys, &mut mem, 7);
    // Step until back in the outer handler right after the SVC returned.
    let mut back_in_outer = false;
    let mut guard = 0u32;
    while cpu.ipsr != 0 && guard < 200 && cpu.fault.is_none() {
        set_intr_masks(0, 0);
        cpu.run(sys, &mut mem, 1);
        guard += 1;
        if cpu.ipsr == 23 && (cpu.regs.r[15] & !1) >= 0x0800008C {
            back_in_outer = true;
            break;
        }
    }
    assert!(cpu.fault.is_none(), "{:?}", cpu.fault);
    assert!(back_in_outer, "never returned from nested SVC to outer handler");
    // The outer entry must still be active (without the push_active balance,
    // the inner return popped it and the stack reads empty here).
    assert!(
        sys.p.nvic.borrow().is_in_interrupt(),
        "outer active entry lost across nested SVC return"
    );
    // Run to full return and check end state + log order.
    guard = 0;
    while cpu.ipsr != 0 && guard < 200 && cpu.fault.is_none() {
        set_intr_masks(0, 0);
        cpu.run(sys, &mut mem, 1);
        guard += 1;
    }
    assert!(cpu.fault.is_none(), "{:?}", cpu.fault);
    assert_eq!(cpu.ipsr, 0);
    assert_eq!(mem.read32(0x20000170), 0x123);
    assert!(!sys.p.nvic.borrow().is_in_interrupt(), "active stack balanced");
}
/// stack, LR=EXC_RETURN_PSP), `msr psp` to another task mid-handler (what
/// PendSV does), return unstacks the NEW task. Asserts both frames.
#[test]
fn psp_task_switch_roundtrip() {
    let _held = crate::test_util::lock();
    init();
    let sys = sys();
    let mut mem = FlatMemory::new(0x100, 0x200);
    // IRQ7 vector -> handler that just returns (switch done manually below
    // with the same two primitives: write_psp + bx lr).
    mem.load(&[0x81u8, 0x00, 0x00, 0x08], 0x08000000 + 23 * 4);
    mem.load(&[0x70, 0x47], 0x08000080); // bx lr
    mem.load(&[0x09, 0x22], 0x080000D0); // movs r2,#9 (task-B landing pad)
    // Task-B stack with a crafted entry frame at 0x20000100.
    for (i, w) in [
        0x11111111u32,
        0x22222222,
        0x33333333,
        0x44444444,
        0x55555555,
        0x66666666,
        0x080000D0, // retpc (thumb bit forced by return)
        0x01000000, // xpsr (T-bit)
    ]
    .iter()
    .enumerate()
    {
        let a = 0x20000100u32 + (i as u32) * 4;
        mem.write8(a, (*w & 0xFF) as u8);
        mem.write8(a + 1, ((*w >> 8) & 0xFF) as u8);
        mem.write8(a + 2, ((*w >> 16) & 0xFF) as u8);
        mem.write8(a + 3, ((*w >> 24) & 0xFF) as u8);
    }
    let mut cpu = Cpu::new(0x200001F0, 0x080000C1);
    cpu.dsp = false;
    // Thread + PSP mode on task A (0x200000C0), handler stack on MSP.
    cpu.regs.control = 2;
    cpu.regs.r[13] = 0x200000C0;
    cpu.regs.psp = 0x200000C0;
    cpu.regs.r[0] = 0xAAAAAAAA;
    cpu.regs.r[1] = 0xBBBBBBBB;
    cpu.regs.r[2] = 0xCCCCCCCC;
    cpu.regs.r[3] = 0xDDDDDDDD;
    cpu.regs.r[12] = 0xEEEEEEEE;
    cpu.regs.r[14] = 0x12345678;

    cpu.take_exception(sys, &mut mem, 7);
    assert_eq!(cpu.regs.r[14], 0xFFFFFFFD, "return uses the PSP stack");
    assert_eq!(cpu.regs.r[13], 0x200000A0, "handler runs on MSP past the frame");
    assert_eq!(cpu.regs.msp, 0x200000A0);
    assert_eq!(cpu.regs.psp, 0x200000A0, "thread bank advanced past its frame");
    assert_eq!(cpu.ipsr, 23);
    // Entry frame holds the full pre-take context.
    assert_eq!(mem.read32(0x200000A0), 0xAAAAAAAA);
    assert_eq!(mem.read32(0x200000A4), 0xBBBBBBBB);
    assert_eq!(mem.read32(0x200000B8), 0x080000C1, "stacked return address");

    // PendSV-style switch to task B, then return through bx lr.
    cpu.write_psp(0x20000100);
    assert_eq!(cpu.regs.r[13], 0x200000A0, "current (MSP) stack unaffected");
    set_intr_masks(0, 0);
    let n = cpu.run(sys, &mut mem, 1); // bx lr
    assert_eq!(n, 1);
    assert!(cpu.fault.is_none(), "{:?}", cpu.fault);
    assert_eq!(cpu.ipsr, 0);
    assert_eq!(cpu.regs.r[0], 0x11111111, "task-B context restored");
    assert_eq!(cpu.regs.r[1], 0x22222222);
    assert_eq!(cpu.regs.r[13], 0x20000120, "SP advanced past task-B frame");
    assert_eq!(cpu.regs.psp, 0x20000120);
    assert!(cpu.regs.control & 2 != 0, "still on PSP bank");
    // Task-A frame untouched by the switch.
    assert_eq!(mem.read32(0x200000A0), 0xAAAAAAAA);
    // Landing pad runs on task B.
    cpu.run(sys, &mut mem, 1);
    assert_eq!(cpu.regs.r[2], 9);
}
