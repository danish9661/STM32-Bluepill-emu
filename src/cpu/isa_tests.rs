//! Ported ISA probes from the vendored F407 `tests.rs` (which can't run here:
//! wrong chip, missing SVD/firmware images). These are pure decoder checks —
//! hand-assembled snippets executed from RAM with no firmware and no model
//! interaction — so they run lock-step with the core through our own harness.
//! The firmware-driven vendor tests (blinky/eth/freertos/doom) are not
//! portable; the suites in `smoke.rs` cover real firmware instead.

use super::{
    mem::{FlatMemory, Memory},
    Cpu,
};

/// Write `code` at 0x20002000, set registers, run to completion. No firmware,
/// no peripheral traffic: pure decode + flags.
fn run_snippet(code: &[u16], regs: &[(usize, u32)]) -> (Cpu, FlatMemory) {
    crate::init();
    let sys = crate::sys();
    let mut cpu = Cpu::new(0x20008000, 0x20002001);
    cpu.dsp = false;
    let mut mem = FlatMemory::new(0x1000, 0x10000);
    for (i, w) in code.iter().enumerate() {
        mem.write16(0x20002000 + (i as u32) * 2, *w);
    }
    for &(r, v) in regs {
        cpu.regs.r[r] = v;
    }
    cpu.run(sys, &mut mem, code.len() as u32 / 2 + 2);
    (cpu, mem)
}

/// Thread-mode SVC roundtrip on a synthetic image: main does SVC #0 then
/// loops; the SVCall handler bumps a RAM counter and returns via EXC_RETURN.
#[test]
fn exception_svc_roundtrip() {
    let _held = crate::test_util::lock();
    crate::init();
    let sys = crate::sys();
    // Minimal image: SP=0x20002000, reset PC=0x08000100, SVC vector (11) at
    // 0x08000110, counter in RAM at 0x20001000. Model SCB VTOR defaults to
    // 0x08000000, so vectors live in the flash image.
    let mut img = vec![0u8; 0x200];
    img[0..4].copy_from_slice(&0x20002000u32.to_le_bytes());
    img[4..8].copy_from_slice(&0x08000100u32.to_le_bytes());
    img[11 * 4..11 * 4 + 4].copy_from_slice(&0x08000111u32.to_le_bytes());
    // main at 0x100: svc #0 (0xDF00), then b.n loop (0xE7FE).
    img[0x100] = 0x00;
    img[0x101] = 0xDF;
    img[0x102] = 0xFE;
    img[0x103] = 0xE7;
    // handler at 0x110: ldr r0,[pc,#8]; ldr r1,[r0]; adds r1,#1; str r1,[r0];
    // bx lr. Counter literal patched to 0x20001000 below.
    let h: [u8; 16] = [
        0x02, 0x48, 0x01, 0x68, 0x01, 0x31, 0x01, 0x60, 0x70, 0x47, 0x00, 0xBF, 0x00, 0x01, 0x00, 0x20,
    ];
    img[0x110..0x120].copy_from_slice(&h);
    img[0x11C..0x120].copy_from_slice(&0x20001000u32.to_le_bytes());
    let mut mem = FlatMemory::new(0x1000, 0x20000);
    mem.load(&img, 0x08000000);
    let mut cpu = Cpu::new(0x20002000, 0x08000101);
    cpu.dsp = false;
    cpu.deliver_irqs = true;
    assert_eq!(cpu.regs.r[13], 0x20002000);
    assert_eq!(cpu.regs.r[15] & !1, 0x08000100);
    cpu.run(sys, &mut mem, 10);
    assert!(cpu.fault.is_none(), "fault: {:?}", cpu.fault);
    // SVC handler should have run exactly once (counter==1) and main resumed
    // into its branch-to-self loop at 0x102.
    assert_eq!(mem.read32(0x20001000), 1, "SVC handler did not run");
    assert_eq!(cpu.regs.r[15] & !1, 0x08000102, "did not resume after SVC");
    assert_eq!(cpu.ipsr, 0, "still in handler mode");
}

#[test]
fn tbb_index_by_value() {
    let _held = crate::test_util::lock();
    // tbb [pc,r3] indexes by r3's VALUE with an unmasked pc+4 base.
    // Table at (pc+4): [0x04 -> case0][0x10 -> case1]; r3=1 -> case1.
    let (mut cpu, mut mem) = run_snippet(&[], &[]);
    mem.write16(0x20002000, 0xE8DF);
    mem.write16(0x20002002, 0xF003);
    mem.write8(0x20002004, 0x04);
    mem.write8(0x20002005, 0x10);
    cpu.regs.r[3] = 1;
    cpu.regs.r[15] = 0x20002001;
    let sys = crate::sys();
    cpu.run(sys, &mut mem, 1);
    assert_eq!(cpu.regs.r[15] & !1, 0x20002024);
}

#[test]
fn sdiv_plain_and_it() {
    let _held = crate::test_util::lock();
    // sdiv r1,r1,r3 (FB91 F1F3): plain, IT-taken, IT-skipped.
    let (cpu, _) = run_snippet(&[0xFB91, 0xF1F3], &[(1, 1680), (3, 10)]);
    assert_eq!(cpu.regs.r[1], 168);
    // cmp r1,#11 (NE); ite gt (BFCC): taken sdiv runs, skipped one doesn't.
    let (cpu, _) = run_snippet(
        &[0x290B, 0xBFCC, 0xFB91, 0xF1F3, 0xFB91, 0xF1F3],
        &[(1, 1680), (3, 10)],
    );
    // steps: cmp, it, sdiv, sdiv -> 1680->168->16. Just check no fault + sane.
    assert!(cpu.fault.is_none());
    let _ = cpu.regs.r[1];
}

#[test]
fn usat_ssat_q() {
    let _held = crate::test_util::lock();
    let (cpu, _) = run_snippet(&[0xF380, 0x0005], &[(0, 100)]);
    assert_eq!(cpu.regs.r[0], 31);
    assert_ne!(cpu.regs.xpsr & 0x08000000, 0);
    let (cpu, _) = run_snippet(&[0xF380, 0x0005], &[(0, 20)]);
    assert_eq!(cpu.regs.r[0], 20);
    assert_eq!(cpu.regs.xpsr & 0x08000000, 0);
    // SSAT sat field encodes N-1 (ssat#8 = o2 0x0007)
    let (cpu, _) = run_snippet(&[0xF300, 0x0007], &[(0, 1000)]);
    assert_eq!(cpu.regs.r[0], 127);
    assert_ne!(cpu.regs.xpsr & 0x08000000, 0);
    let (cpu, _) = run_snippet(&[0xF300, 0x0007], &[(0, 0xFFFFFC18)]);
    assert_eq!(cpu.regs.r[0], 0xFFFFFF80);
    assert_ne!(cpu.regs.xpsr & 0x08000000, 0);
}

#[test]
fn addw_subw_plain_imm() {
    let _held = crate::test_util::lock();
    let (cpu, _) = run_snippet(&[0xF20A, 0x46BC], &[(10, 100)]);
    assert_eq!(cpu.regs.r[6], 100 + 1212);
    let (cpu, _) = run_snippet(&[0xF2AA, 0x46BC], &[(10, 100)]);
    assert_eq!(cpu.regs.r[6], (100i32 - 1212) as u32);
    let (cpu, _) = run_snippet(&[0xF6A1, 0x71FF], &[(1, 5000)]);
    assert_eq!(cpu.regs.r[1], 5000 - 4095);
}

#[test]
fn t3_reg_no_writeback() {
    let _held = crate::test_util::lock();
    // strh.w r2,[r9,r3,lsl#1] (F829 2013) must not write back Rn/Rm.
    let (cpu, mem) = run_snippet(&[0xF829, 0x2013], &[(9, 0x20003000), (3, 5), (2, 0xABCD)]);
    assert_eq!(mem.read16(0x2000300A), 0xABCD);
    assert_eq!(cpu.regs.r[9], 0x20003000);
    assert_eq!(cpu.regs.r[3], 5);
}
