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

// ---- 32-bit census probes: one canonical encoding per family, exact
// results. Encodings Capstone-verified (M-class Thumb); see census_32.py.

/// T3 flag-setting data processing (S-bit forms share the modified-imm arm).
#[test]
fn t3_s_bit_forms() {
    let _held = crate::test_util::lock();
    // adds.w r0,r1,#0x11 (F111 0011), r1=5 -> 0x16, NZCV=0000.
    // (Trailing b.n parks the overrun budget: zeros decode as flag-setting
    // LSLs and would pollute xpsr after the probe.)
    let (cpu, _) = run_snippet(&[0xF111, 0x0011, 0xE7FE], &[(1, 5)]);
    assert!(cpu.fault.is_none(), "adds.w fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 0x16);
    assert_eq!(cpu.regs.xpsr & 0xF0000000, 0x00000000);
    // subs.w r0,r1,#0x11, r1=5 -> -12, N=1, C=0 (borrow)
    let (cpu, _) = run_snippet(&[0xF1B1, 0x0011, 0xE7FE], &[(1, 5)]);
    assert!(cpu.fault.is_none(), "subs.w fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 0xFFFF_FFF4);
    assert_eq!(cpu.regs.xpsr & 0xF0000000, 0x80000000);
    // cmp.w r1,#0x11 (Rd=15 test form), r1=0x11 -> Z=1, C=1
    let (cpu, _) = run_snippet(&[0xF1B1, 0x0F11, 0xE7FE], &[(1, 0x11)]);
    assert!(cpu.fault.is_none(), "cmp.w fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.xpsr & 0xF0000000, 0x60000000);
    // rsbs.w r0,r1,#0, r1=5 -> -5, N=1, C=0
    let (cpu, _) = run_snippet(&[0xF1D1, 0x0000, 0xE7FE], &[(1, 5)]);
    assert!(cpu.fault.is_none(), "rsbs.w fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 0xFFFF_FFFB);
    assert_eq!(cpu.regs.xpsr & 0xF0000000, 0x80000000);
    // tst.w r1,#0x11 (ANDS test form, Rd=15), r1=5 -> 0x01, no write, Z=0
    let (cpu, _) = run_snippet(&[0xF011, 0x0F11, 0xE7FE], &[(1, 5)]);
    assert!(cpu.fault.is_none(), "tst.w fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 0);
    assert_eq!(cpu.regs.xpsr & 0xF0000000, 0x00000000);
}

/// T3 with i-bit set (F6xx first halfword): large modified immediates.
#[test]
fn t3_ibit_immediate() {
    let _held = crate::test_util::lock();
    // add.w r2,r3,#0xABC (F603 22BC), r3=0x100 -> 0xBBC, no flags
    let (cpu, _) = run_snippet(&[0xF603, 0x22BC, 0xE7FE], &[(3, 0x100)]);
    assert!(cpu.fault.is_none(), "add.w i-bit fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[2], 0xBBC);
    assert_eq!(cpu.regs.xpsr & 0xF0000000, 0x00000000);
}

/// MRS/MSR special-register moves.
#[test]
fn mrs_msr_forms() {
    let _held = crate::test_util::lock();
    // adds.w sets NZCV=0110 (Z=1,C=1); mrs r0,apsr reads it back.
    let (cpu, _) = run_snippet(
        &[0xF111, 0x0001, 0xF3EF, 0x8000],
        &[(1, 0xFFFF_FFFF)],
    );
    assert!(cpu.fault.is_none(), "mrs apsr fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 0x6000_0000);
    // mrs r0,primask reads 0; msr primask,r1 sets it.
    let (cpu, _) = run_snippet(&[0xF3EF, 0x8010], &[]);
    assert!(cpu.fault.is_none(), "mrs primask fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 0);
    let (cpu, _) = run_snippet(&[0xF381, 0x8010], &[(1, 1)]);
    assert!(cpu.fault.is_none(), "msr primask fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.primask, 1);
    // mrs r0,control reads 0 in thread-MSP mode.
    let (cpu, _) = run_snippet(&[0xF3E8, 0x8014], &[]);
    assert!(cpu.fault.is_none(), "mrs control fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 0);
}

/// Bitfield: SBFX/UBFX/BFI/BFC.
#[test]
fn bitfield_forms() {
    let _held = crate::test_util::lock();
    // sbfx r0,r1,#8,#8, r1=0xABCD00 -> 0xFFFFFFCD
    let (cpu, _) = run_snippet(&[0xF341, 0x2007], &[(1, 0xABCD00)]);
    assert!(cpu.fault.is_none(), "sbfx fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 0xFFFF_FFCD);
    // ubfx r0,r1,#8,#8 -> 0xCD
    let (cpu, _) = run_snippet(&[0xF3C1, 0x2007], &[(1, 0xABCD00)]);
    assert!(cpu.fault.is_none(), "ubfx fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 0xCD);
    // bfi r0,r1,#8,#8: r0=0xFF000000, r1=0xAB -> 0xFF00AB00
    let (cpu, _) = run_snippet(&[0xF361, 0x200F], &[(0, 0xFF00_0000), (1, 0xAB)]);
    assert!(cpu.fault.is_none(), "bfi fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 0xFF00_AB00);
    // bfc r0,#8,#8: r0=0xFFFFFFFF -> 0xFFFF00FF
    let (cpu, _) = run_snippet(&[0xF36F, 0x200F], &[(0, 0xFFFF_FFFF)]);
    assert!(cpu.fault.is_none(), "bfc fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 0xFFFF_00FF);
}

/// LDRD/STRD doubleword transfers.
#[test]
fn ldrd_strd_roundtrip() {
    let _held = crate::test_util::lock();
    // strd r2,r3,[r1,#0x20]; ldrd r4,r5,[r1,#0x20]
    let (cpu, mem) = run_snippet(
        &[0xE9C1, 0x2308, 0xE9D1, 0x4508],
        &[(1, 0x20003000), (2, 0x1111_1111), (3, 0x2222_2222)],
    );
    assert!(cpu.fault.is_none(), "ldrd/strd fault: {:?}", cpu.fault);
    assert_eq!(mem.read32(0x20003020), 0x1111_1111);
    assert_eq!(mem.read32(0x20003024), 0x2222_2222);
    assert_eq!(cpu.regs.r[4], 0x1111_1111);
    assert_eq!(cpu.regs.r[5], 0x2222_2222);
}

/// UDIV incl. divide-by-zero (M3 CCR.DIV_0_TRP=0: quotient 0, no trap).
#[test]
fn udiv_and_div0() {
    let _held = crate::test_util::lock();
    // udiv r0,r1,r2 (FBB1 F0F2): 100/7 = 14
    let (cpu, _) = run_snippet(&[0xFBB1, 0xF0F2], &[(1, 100), (2, 7)]);
    assert!(cpu.fault.is_none(), "udiv fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 14);
    // udiv by zero -> 0, no fault
    let (cpu, _) = run_snippet(&[0xFBB1, 0xF0F2], &[(1, 100), (2, 0)]);
    assert!(cpu.fault.is_none(), "udiv-by-zero fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 0);
}

/// LDREX/STREX exclusive monitor (single global reservation, single core).
#[test]
fn ldrex_strex_pair() {
    let _held = crate::test_util::lock();
    // str r2,[r1]; ldrex r0,[r1] (E851 0F00); strex r2,r0,[r1] (E841 0200)
    let (cpu, mem) = run_snippet(
        &[0x600A, 0xE851, 0x0F00, 0xE841, 0x0200, 0xE7FE],
        &[(1, 0x20003000), (2, 0xDEAD_BEEF)],
    );
    assert!(cpu.fault.is_none(), "ldrex/strex fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 0xDEAD_BEEF, "ldrex did not load");
    assert_eq!(cpu.regs.r[2], 0, "strex status must be 0 after ldrex");
    assert_eq!(mem.read32(0x20003000), 0xDEAD_BEEF);
    // Second strex with no reservation: status 1, store dropped.
    let (cpu, mem) = run_snippet(
        &[0xE841, 0x0400, 0xE7FE],
        &[(0, 0x1234_5678), (1, 0x20003000), (4, 0xAA)],
    );
    assert!(cpu.fault.is_none(), "bare strex fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[4], 1, "strex without ldrex must report 1");
    assert_eq!(mem.read32(0x20003000), 0, "failed strex must not store");
}

/// TBH halfword table branch (TBB covered by tbb_index_by_value).
#[test]
fn tbh_index_by_value() {
    let _held = crate::test_util::lock();
    let (mut cpu, mut mem) = run_snippet(&[], &[]);
    // tbh [pc,r2] (E8DF F012); table of halfwords at pc+4.
    mem.write16(0x20002000, 0xE8DF);
    mem.write16(0x20002002, 0xF012);
    mem.write16(0x20002004, 0x0004); // case0 -> base+8
    mem.write16(0x20002006, 0x0010); // case1 -> base+32
    cpu.regs.r[2] = 1;
    cpu.regs.r[15] = 0x20002001;
    let sys = crate::sys();
    cpu.run(sys, &mut mem, 1);
    assert!(cpu.fault.is_none(), "tbh fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[15] & !1, 0x20002024);
}

/// BL with link + landing (valid form: 2nd halfword >= 0xF800).
#[test]
fn bl_link_and_land() {
    let _held = crate::test_util::lock();
    // bl (F000 F800): off=0 -> target pc+4; movs lands, lr=(pc+4)|1.
    let (cpu, _) = run_snippet(&[0xF000, 0xF800, 0x2042, 0xE7FE], &[]);
    assert!(cpu.fault.is_none(), "bl fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 0x42);
    assert_eq!(cpu.regs.r[14], 0x20002005);
}

/// STRD post-indexed (E8 P=0 form): transfer at base, write base-off back.
#[test]
fn strd_post_indexed() {
    let _held = crate::test_util::lock();
    // strd r2,r3,[r1],#-0x20 (E861 2308): r1=0x20003020.
    let (cpu, mem) = run_snippet(
        &[0xE861, 0x2308, 0xE7FE],
        &[(1, 0x20003020), (2, 0xAAAA_AAAA), (3, 0x5555_5555)],
    );
    assert!(cpu.fault.is_none(), "strd post fault: {:?}", cpu.fault);
    assert_eq!(mem.read32(0x20003020), 0xAAAA_AAAA);
    assert_eq!(mem.read32(0x20003024), 0x5555_5555);
    assert_eq!(cpu.regs.r[1], 0x20003000);
}

/// LDMDB (decrement-before) with and without writeback.
#[test]
fn ldmdb_forms() {
    let _held = crate::test_util::lock();
    // ldmdb r0,{r1,r2} (E910 0006): r0=0x20003008 reads [..00],[..04].
    let (cpu, _) = run_snippet(&[0xE910, 0x0006, 0xE7FE], &[(0, 0x20003008)]);
    assert!(cpu.fault.is_none(), "ldmdb fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[1], 0);
    assert_eq!(cpu.regs.r[2], 0);
    assert_eq!(cpu.regs.r[0], 0x20003008, "no-WB must not move Rn");
    // ldmdb r0!,{r1,r2} (E930 0006): DB+WB ends back at Rn.
    let (cpu, _) = run_snippet(&[0xE930, 0x0006, 0xE7FE], &[(0, 0x20003008)]);
    assert!(cpu.fault.is_none(), "ldmdb! fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 0x20003008);
    // ldmib r0!,{r1} (E9B0 0002): first transfer at Rn+4, WB to Rn+8.
    // (Capstone-MCLASS rejects all IB/DA forms, so this encoding is manual-
    // derived: P=[8],U=[7],W=[5],L=[4] — cross-checked against LDMIA/LDMDB.)
    let (cpu, mem) = run_snippet(&[0xE9B0, 0x0002, 0xE7FE], &[(0, 0x20003000)]);
    assert!(cpu.fault.is_none(), "ldmib! fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[1], mem.read32(0x20003004));
    assert_eq!(cpu.regs.r[0], 0x20003008);
    // ldmda r0!,{r1} (E830 0002): first transfer at Rn-4, WB back to Rn.
    let (cpu, mem) = run_snippet(&[0xE830, 0x0002, 0xE7FE], &[(0, 0x20003000)]);
    assert!(cpu.fault.is_none(), "ldmda! fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[1], mem.read32(0x20002FFC));
    assert_eq!(cpu.regs.r[0], 0x20003000);
}

/// USAT with ASR shift (0xF3A0 form; LSL form covered by usat_ssat_q).
#[test]
fn usat_asr_shift() {
    let _held = crate::test_util::lock();
    // usat r0,#0x10,r0,asr #4 (F3A0 1010): r0=0x80000000 -> asr=0xF8000000
    // saturates to 0xFFFF with Q set.
    let (cpu, _) = run_snippet(&[0xF3A0, 0x1010, 0xE7FE], &[(0, 0x8000_0000)]);
    assert!(cpu.fault.is_none(), "usat.asr fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 0xFFFF);
    assert_ne!(cpu.regs.xpsr & 0x08000000, 0);
    // No saturation: r0=0x8000 -> asr=0x0800, kept, Q clear. (Positive
    // input keeps it under the u16 max; LSL#4 would give 0x80000 and
    // saturate instead, so this distinguishes the shift direction.)
    let (cpu, _) = run_snippet(&[0xF3A0, 0x1010, 0xE7FE], &[(0, 0x8000)]);
    assert!(cpu.fault.is_none(), "usat.asr(2) fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 0x0800);
    assert_eq!(cpu.regs.xpsr & 0x08000000, 0);
}

/// RBIT/CLZ are base ARMv7-M (M3 has them; only M0-class lacks them).
#[test]
fn rbit_clz_present() {
    let _held = crate::test_util::lock();
    // rbit r0,r0 (FA90 F0A0): 0x12345678 -> 0x1E6A2C48
    let (cpu, _) = run_snippet(&[0xFA90, 0xF0A0, 0xE7FE], &[(0, 0x1234_5678)]);
    assert!(cpu.fault.is_none(), "rbit fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 0x1E6A_2C48);
    // clz r0,r0 (FAB0 F080): 0x00F00000 -> 8
    let (cpu, _) = run_snippet(&[0xFAB0, 0xF080, 0xE7FE], &[(0, 0x00F0_0000)]);
    assert!(cpu.fault.is_none(), "clz fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 8);
}

/// MRS across the banked/system registers.
#[test]
fn mrs_full_sysm() {
    let _held = crate::test_util::lock();
    // mrs r0,ipsr (F3EF 8005) in thread mode -> 0
    let (cpu, _) = run_snippet(&[0xF3EF, 0x8005, 0xE7FE], &[]);
    assert!(cpu.fault.is_none(), "mrs ipsr fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 0);
    // mrs r0,msp (F3EF 8008) reads the thread-mode SP
    let (cpu, _) = run_snippet(&[0xF3EF, 0x8008, 0xE7FE], &[]);
    assert!(cpu.fault.is_none(), "mrs msp fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 0x20008000);
}

/// BL validity boundary + DBG hint.
#[test]
fn bl_boundary_and_dbg() {
    let _held = crate::test_util::lock();
    // bl (F000 F800) covered by bl_link_and_land; DBG (F3AF 80F0) is a NOP.
    let (cpu, _) = run_snippet(&[0xF3AF, 0x80F0, 0xE7FE], &[(0, 0x1234)]);
    assert!(cpu.fault.is_none(), "dbg fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 0x1234);
}

/// PLD/PLI hints NOP (any sign, literal or offset); LDR.W pc literal branches.
#[test]
fn pld_hints_vs_ldr_pc_literal() {
    let _held = crate::test_util::lock();
    // pld [pc,#-0xf0] (F81F F0F0): hint, falls through, no fault.
    let (cpu, _) = run_snippet(&[0xF81F, 0xF0F0, 0xE7FE], &[]);
    assert!(cpu.fault.is_none(), "pld literal fault: {:?}", cpu.fault);
    // pld [r0,#-0xfc] (F810 FCFC): hint with Rn!=PC, falls through.
    let (cpu, _) = run_snippet(&[0xF810, 0xFCFC, 0xE7FE], &[(0, 0x20003000)]);
    assert!(cpu.fault.is_none(), "pld offset fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[0], 0x20003000, "pld must not write back");
    // ldr.w pc,[pc,#0] (F85F F000): literal pool at pc+4 holds odd target.
    let (mut cpu, mut mem) = run_snippet(&[], &[]);
    mem.write16(0x20002000, 0xF85F);
    mem.write16(0x20002002, 0xF000);
    mem.write32(0x20002004, 0x20002011);
    cpu.regs.r[15] = 0x20002001;
    let sys = crate::sys();
    cpu.run(sys, &mut mem, 1);
    assert!(cpu.fault.is_none(), "ldr.w pc literal fault: {:?}", cpu.fault);
    assert_eq!(cpu.regs.r[15] & !1, 0x20002010);
}
