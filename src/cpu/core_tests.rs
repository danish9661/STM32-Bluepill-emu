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

/// PSP task switch roundtrip: take from thread+PSP (frame lands on the task
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
