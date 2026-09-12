//! Native CPU backend (Path B) for the JS drivers.
//!
//! Owns a [`Cpu`] + [`FlatMemory`] pair that runs guest firmware with zero
//! JS crossings per instruction, against the shared peripheral model/DMA/NVIC.
//!
//! Driver loop shape per batch (cli.mjs / emulator.js):
//!   1. pump stdin via `uart_rx_byte`, skip while DMA busy
//!   2. `rustcpu_dma_pump()` (plan build + exec against Rust RAM, no JS RAM)
//!   3. `n = rustcpu_run(batch)` (SVC dispatched inline, exact count back)
//!   4. `step_batch(n)` / `process_batch(n)` (peripheral tick, IRQ probe)
//!   5. `rustcpu_dma_pump()` again
//!   6. `rustcpu_dispatch()` (lazy batch-boundary IRQ dispatch, like
//!      processInterrupts; handlers run single-stepped so a return never
//!      overshoots into thread code)
//!   7. hi2c Mode hook via `rustcpu_i2c_hook_fired()` + RAM patch
//!   8. `is_watchdog_reset_requested()` check
//!
//! Instruction accounting is EXACT here (executed count returned, handlers
//! included).

use std::sync::atomic::{AtomicBool, Ordering};
use wasm_bindgen::prelude::*;

use crate::cpu::{
    mem::{is_periph, FlatMemory, Memory},
    Cpu,
};
use crate::system::WasmSystem;

struct NativeEmu {
    cpu: Cpu,
    mem: FlatMemory,
}

// Process-global like SYS (WASM is single-threaded). Cleared by init()/init_svd()
// via reset() so a re-init never leaves a stale CPU/RAM pair behind.
static mut NATIVE: Option<NativeEmu> = None;

fn native_mut() -> &'static mut NativeEmu {
    unsafe {
        (*std::ptr::addr_of_mut!(NATIVE))
            .as_mut()
            .expect("native backend not initialized — call rustcpu_init() first")
    }
}

pub(crate) fn reset() {
    unsafe {
        *std::ptr::addr_of_mut!(NATIVE) = None;
    }
    WRITE_TAP.store(false, Ordering::Relaxed);
    take_writes();
}

/// Loaded image windows ((flash_base, len), (ram_base, len)) for agents
/// that validate guest addresses (bootloader ROM flows). None when the
/// native backend is not initialized.
pub(crate) fn mem_ranges() -> Option<((u32, u32), (u32, u32))> {
    unsafe {
        (*std::ptr::addr_of!(NATIVE)).as_ref().map(|e| {
            (
                (e.mem.flash_base, e.mem.flash.len() as u32),
                (e.mem.ram_base, e.mem.ram.len() as u32),
            )
        })
    }
}

/// Create the CPU + guest RAM. Call after init()/init_svd() and before load.
/// `dsp` is always false here (Cortex-M3 has no DSP extension).
#[wasm_bindgen]
pub fn rustcpu_init(sp: u32, pc: u32, flash_size: u32, ram_size: u32) {
    let mut cpu = Cpu::new(sp, pc | 1);
    cpu.dsp = false;
    cpu.deliver_irqs = false; // lazy batch-boundary dispatch
    let mem = FlatMemory::new(flash_size as usize, ram_size as usize);
    unsafe {
        *std::ptr::addr_of_mut!(NATIVE) = Some(NativeEmu { cpu, mem });
    }
}

/// Load firmware bytes at a guest physical address (bypasses flash
/// protection, like a debugger memory write at load time).
#[wasm_bindgen]
pub fn rustcpu_load(data: &[u8], base: u32) {
    native_mut().mem.load(data, base);
}

/// Run the CPU for up to `slice` instructions. SVC is dispatched inline onto
/// the real stack (no mirror needed); any other fault stops the run and is
/// reported via rustcpu_fault(). Returns instructions actually executed
/// (thread + handler), for exact accounting.
#[wasm_bindgen]
pub fn rustcpu_run(slice: u32) -> u32 {
    let emu = native_mut();
    let sys = crate::sys();
    // Debug drains (cold unless a probe attached something): a guest DCRSR
    // write pends a register transfer, a DHCSR C_STEP edge pends one step.
    if sys.swd.take_xfer_pending() {
        debug_do_transfer(sys, &mut emu.cpu);
    }
    if sys.swd.take_step_pending() {
        return debug_single_step(sys, &mut emu.cpu, &mut emu.mem);
    }
    // Halted core (DHCSR C_HALT / watchpoint / VC_HARDERR): the debugger
    // polls `swd_halted()` / steps via `swd_step()`.
    if crate::system::debug_halted() {
        return 0;
    }
    crate::set_intr_masks(emu.cpu.regs.primask, 0);
    let mut done = emu.cpu.run(sys, &mut emu.mem, slice);
    if let Some(f) = emu.cpu.fault.take() {
        if f.op1 & 0xFF00 == 0xDF00 {
            // SVC: step past it and run the SVCall handler synchronously.
            // Push the active entry the pop-based dispatch would have pushed
            // (balanced by exception_return, including nested takes).
            emu.cpu.regs.r[15] = f.pc.wrapping_add(2) | 1;
            sys.p.nvic.borrow_mut().push_active(-5);
            emu.cpu.take_exception(sys, &mut emu.mem, -5);
            done += run_handler_to_return(&mut emu.cpu, &mut emu.mem);
        } else {
            emu.cpu.fault = Some(f);
        }
    }
    done
}

/// Run a pending handler to exception return. Single-stepped on purpose: a
/// chunked run would keep executing past `bx lr` into thread code for the
/// rest of the budget (seen: overshoot from an SPI2 print reached testSVC
/// and faulted on `svc #2`).
fn run_handler_to_return(cpu: &mut Cpu, mem: &mut FlatMemory) -> u32 {
    let sys = crate::sys();
    let mut hdone = 0u32;
    while cpu.ipsr != 0 && hdone < 20000 && cpu.fault.is_none() {
        crate::set_intr_masks(cpu.regs.primask, 0);
        hdone += cpu.run(sys, mem, 1);
    }
    hdone
}

/// Dispatch all pending interrupts within the shared per-batch budget.
/// Returns the number of IRQs dispatched.
#[wasm_bindgen]
pub fn rustcpu_dispatch() -> u32 {
    let emu = native_mut();
    let sys = crate::sys();
    // A halted core takes no exceptions (the probe owns it now).
    if crate::system::debug_halted() {
        return 0;
    }
    if sys.swd.take_xfer_pending() {
        debug_do_transfer(sys, &mut emu.cpu);
    }
    // Honor live PRIMASK (not the stale INTR_MASK snapshot): intr_next()
    // consults the statics, and a batch ending inside a noInterrupts()
    // critical section must not dispatch into it (real HW blocks on live
    // PRIMASK). Stale snapshots broke the EXTI SWIER-vs-PR race once the
    // SysTick phase fix moved the alignment onto the window.
    crate::system::INTR_MASK_PRIMASK.store(emu.cpu.regs.primask, std::sync::atomic::Ordering::Relaxed);
    let mut n = 0u32;
    loop {
        let irq = crate::interrupts::intr_next();
        if irq <= -100 {
            return n;
        }
        emu.cpu.take_exception(sys, &mut emu.mem, irq);
        run_handler_to_return(&mut emu.cpu, &mut emu.mem);
        if emu.cpu.fault.is_some() {
            return n;
        }
        n += 1;
    }
}

/// Pending CPU fault, if the last run/dispatch stopped on one: empty when
/// clean, else [pc, op1, op2, len]. (Periph39 runs fault-free; anything here
/// is a loud decoder gap.)
#[wasm_bindgen]
pub fn rustcpu_fault() -> Vec<u32> {
    match &native_mut().cpu.fault {
        None => Vec::new(),
        Some(f) => vec![f.pc, f.op1 as u32, f.op2 as u32, f.len as u32],
    }
}

#[wasm_bindgen]
pub fn rustcpu_fault_clear() {
    native_mut().cpu.fault = None;
}

/// Registers for getRegisters/getPc/getSp parity + debugging:
/// [r0..r12, sp, lr, pc, xpsr, primask, control, ipsr] (20 words).
#[wasm_bindgen]
pub fn rustcpu_regs() -> Vec<u32> {
    let r = &native_mut().cpu.regs;
    let mut out = Vec::with_capacity(20);
    out.extend_from_slice(&r.r[0..13]);
    out.push(r.r[13]);
    out.push(r.r[14]);
    out.push(r.r[15]);
    out.push(r.xpsr);
    out.push(r.primask);
    out.push(r.control);
    out.push(native_mut().cpu.ipsr);
    out
}

#[wasm_bindgen]
pub fn rustcpu_set_pc(pc: u32) {
    native_mut().cpu.regs.r[15] = pc | 1;
}

/// Debugger register write (GDB `P` packet): r0-r12, SP (bank-synced like
/// the run loop), LR, PC (forced Thumb). xPSR is read-only here.
#[wasm_bindgen]
pub fn rustcpu_set_reg(i: u32, v: u32) {
    let emu = native_mut();
    match i {
        0..=12 => emu.cpu.regs.r[i as usize] = v,
        13 => {
            emu.cpu.regs.r[13] = v;
            if emu.cpu.ipsr == 0 {
                if emu.cpu.regs.control & 2 != 0 {
                    emu.cpu.regs.psp = v;
                } else {
                    emu.cpu.regs.msp = v;
                }
            }
        }
        14 => emu.cpu.regs.r[14] = v,
        15 => emu.cpu.regs.r[15] = v | 1,
        _ => {}
    }
}

/// Raw guest-memory write for debugger clients (GDB `M` packets, BKPT
/// patching): bypasses flash protection and MPU checks like a probe would.
/// Firmware install should still use rustcpu_load.
#[wasm_bindgen]
pub fn rustcpu_mem_write_raw(addr: u32, data: &[u8]) {
    native_mut().mem.load(data, addr);
}

/// Raw guest-memory access (RAM + flash; flash writes stay protected, use
/// rustcpu_load for firmware). Bypasses MPU checks like a debugger would.
/// Backs memRead32 + the hi2c Mode RAM patch.
#[wasm_bindgen]
pub fn rustcpu_mem_read(addr: u32, len: u32) -> Vec<u8> {
    let emu = native_mut();
    let mut out = Vec::with_capacity(len as usize);
    for k in 0..len {
        out.push(emu.mem.read8_raw(addr.wrapping_add(k)));
    }
    out
}

#[wasm_bindgen]
pub fn rustcpu_mem_write(addr: u32, data: &[u8]) {
    let emu = native_mut();
    for (k, &b) in data.iter().enumerate() {
        emu.mem.write8_raw(addr.wrapping_add(k as u32), b);
    }
}

/// Whole DMA pump against Rust RAM with no JS crossings: build the op plan
/// and execute it in one call.
#[wasm_bindgen]
pub fn rustcpu_dma_pump() {
    let emu = native_mut();
    let sys = crate::sys();
    if sys.pending_dma_count() == 0 {
        return;
    }
    let plan = sys.dma_build_plan();
    sys.dma_exec_plan(&mut emu.mem, &plan);
}

/// Fires when I2C1 DR was written with the R-bit set (HAL I2C1 ISR needs
/// hi2c->Mode == 0x22 before reading DR). The driver drains the model flag
/// per batch, then patches RAM *(0x200002d8)+0x3D.
#[wasm_bindgen]
pub fn rustcpu_i2c_hook_fired() -> bool {
    crate::sys().i2c_dr_hook.take()
}

// ---- Peripheral write tap (onPeriphWrite parity) --------------------------
// The page taps the peripheral bus via onPeriphWrite; model writes never
// cross JS, so the tap records (addr, size, value) in
// Peripherals::write for the driver to feed to write watchers per batch.
// Gated by a flag (default off): zero overhead unless a watcher subscribes.

static WRITE_TAP: AtomicBool = AtomicBool::new(false);
static WRITE_LOG: std::sync::Mutex<Vec<u32>> = std::sync::Mutex::new(Vec::new());

pub(crate) fn write_tap_enabled() -> bool {
    WRITE_TAP.load(Ordering::Relaxed)
}

pub(crate) fn record_write(addr: u32, size: u8, value: u32) {
    if let Ok(mut log) = WRITE_LOG.lock() {
        log.extend_from_slice(&[addr, size as u32, value]);
    }
}

/// Enable/disable recording of peripheral writes (driver enables when a
/// write watcher subscribes, disables when the last one leaves).
#[wasm_bindgen]
pub fn rustcpu_write_tap(on: bool) {
    WRITE_TAP.store(on, Ordering::Relaxed);
    if !on {
        take_writes();
    }
}

/// Drain recorded writes as flat [addr, size, value, ...]. Clears the log.
#[wasm_bindgen]
pub fn rustcpu_take_writes() -> Vec<u32> {
    take_writes()
}

fn take_writes() -> Vec<u32> {
    std::mem::take(&mut *WRITE_LOG.lock().unwrap())
}

// ---- ARM debug-port slice (SWD + JTAG-DP + watchpoints) --------------------
// Transaction-level probe API (see peripherals/swd.rs): the DP/AP register
// file lives on WasmSystem.swd, DRW data moves here (only place with both
// SYS and FlatMemory), register transfers touch the live CPU.

/// DCRSR transfer engine (shared by the run-entry drain and `swd_reg_*`).
/// Synchronous semantics: the value moves immediately and S_REGRDY reads
/// ready afterwards (the pending flag is only what a bare guest DCRSR write
/// leaves for the next run entry).
pub(crate) fn debug_do_transfer(sys: &WasmSystem, cpu: &mut Cpu) {
    let sel = sys.swd.transfer_regsel();
    if sys.swd.transfer_is_write() {
        let w = sys.swd.dcrdr_read();
        match sel {
            0..=12 => cpu.regs.r[sel as usize] = w,
            13 => cpu.regs.r[13] = w,
            14 => cpu.regs.r[14] = w,
            15 => cpu.regs.r[15] = w | 1,
            16 => cpu.regs.xpsr = (w & 0xF800_0000) | 0x0100_0000,
            17 => cpu.write_msp(w),
            18 => cpu.write_psp(w),
            _ => {}
        }
    } else {
        let v = match sel {
            0..=12 => cpu.regs.r[sel as usize],
            13 => cpu.regs.r[13],
            14 => cpu.regs.r[14],
            15 => cpu.regs.r[15],
            16 => cpu.regs.xpsr,
            17 => cpu.read_msp(),
            18 => cpu.read_psp(),
            _ => 0,
        };
        sys.swd.dcrdr_write(v);
    }
}

/// Single-step past a halt: clear the mirror, run exactly one instruction,
/// re-halt iff DHCSR still asks (C_DEBUGEN && C_HALT). Returns the executed
/// count (0 or 1; faults stay live for `rustcpu_fault()`).
fn debug_single_step(sys: &WasmSystem, cpu: &mut Cpu, mem: &mut FlatMemory) -> u32 {
    crate::system::set_debug_halt(false);
    crate::set_intr_masks(cpu.regs.primask, 0);
    let done = cpu.run(sys, mem, 1);
    crate::system::sync_debug_halt_from_dhcsr(sys.swd.dhcsr_raw());
    // A step that trips a watchpoint re-halts via the trip itself; the
    // re-sync above already covers the DHCSR-asked case.
    done
}

/// Probe memory read for MEM-AP DRW (debugger path: bypasses MPU and flash
/// protection like a real probe; peripherals route to the model).
fn ap_mem_read(sys: &WasmSystem, mem: &FlatMemory, addr: u32, size: u8) -> u32 {
    if is_periph(addr) {
        sys.p.read(sys, addr, size)
    } else {
        let mut v = 0u32;
        for k in 0..size {
            v |= (mem.read8_raw(addr.wrapping_add(k as u32)) as u32) << (8 * k as u32);
        }
        v
    }
}

/// Probe memory write for MEM-AP DRW (same bypasses; flash stores drop like
/// guest stores — the probe programs flash through the FLASH peripheral).
fn ap_mem_write(sys: &WasmSystem, mem: &mut FlatMemory, addr: u32, size: u8, value: u32) {
    if is_periph(addr) {
        sys.p.write(sys, addr, size, value);
    } else {
        for k in 0..size {
            mem.write8_raw(addr.wrapping_add(k as u32), ((value >> (8 * k as u32)) & 0xFF) as u8);
        }
    }
}

#[wasm_bindgen]
pub fn swd_dp_read(addr: u32) -> u32 {
    crate::sys().swd.dp_read(addr)
}

#[wasm_bindgen]
pub fn swd_dp_write(addr: u32, value: u32) {
    crate::sys().swd.dp_write(addr, value);
}

/// MEM-AP register read. Bank-0 DRW (reg 0xC) performs the data movement:
/// reads TAR-width bytes, latches RDBUFF, auto-increments TAR.
#[wasm_bindgen]
pub fn swd_ap_read(bank: u32, reg: u32) -> u32 {
    let emu = native_mut();
    let sys = crate::sys();
    if bank == 0 && reg & 0xC == 0xC {
        let size = sys.swd.ap_data_size();
        let v = ap_mem_read(sys, &emu.mem, sys.swd.tar(), size);
        sys.swd.ap_latch_rdbuff(v);
        sys.swd.ap_tar_advance();
        return v;
    }
    sys.swd.ap_reg_read(bank, reg)
}

/// MEM-AP register write. Bank-0 DRW (reg 0xC) stores TAR-width bytes and
/// auto-increments TAR.
#[wasm_bindgen]
pub fn swd_ap_write(bank: u32, reg: u32, value: u32) {
    if bank == 0 && (reg & 0xC) == 0xC {
        let emu = native_mut();
        let sys = crate::sys();
        let size = sys.swd.ap_data_size();
        let tar = sys.swd.tar();
        ap_mem_write(sys, &mut emu.mem, tar, size, value);
        sys.swd.ap_tar_advance();
        return;
    }
    crate::sys().swd.ap_reg_write(bank, reg, value);
}

/// Install a data watchpoint: kind 1 = write (GDB Z2), 2 = read (Z3),
/// 3 = access (Z4). Returns the slot (0-3) or -1 when full.
#[wasm_bindgen]
pub fn swd_add_watchpoint(kind: u32, addr: u32, len: u32) -> i32 {
    crate::sys().swd.add_watch(kind, addr, len)
}

#[wasm_bindgen]
pub fn swd_remove_watchpoint(slot: u32) {
    crate::sys().swd.remove_watch(slot);
}

/// Take the pending watch trip: [] when clean, else [addr, dir] with dir
/// 1 = write, 2 = read. One-shot latch; the halt stays until resume.
#[wasm_bindgen]
pub fn swd_take_trip() -> Vec<u32> {
    match crate::sys().swd.take_trip() {
        Some((a, d)) => vec![a, d],
        None => Vec::new(),
    }
}

#[wasm_bindgen]
pub fn swd_halted() -> bool {
    crate::system::debug_halted()
}

/// External halt request (probe/GDB Ctrl-C path): sets C_DEBUGEN+C_HALT.
#[wasm_bindgen]
pub fn swd_halt() {
    crate::sys().swd.halt();
}

/// Debugger resume: clears C_HALT (C_DEBUGEN stays, like silicon).
#[wasm_bindgen]
pub fn swd_resume() {
    crate::sys().swd.resume();
}

/// Single-step the halted core once (0/1 executed; faults stay live).
#[wasm_bindgen]
pub fn swd_step() -> u32 {
    let emu = native_mut();
    let sys = crate::sys();
    sys.swd.take_step_pending();
    debug_single_step(sys, &mut emu.cpu, &mut emu.mem)
}

/// DCRSR-style core register read (0-12, 13 SP, 14 LR, 15 PC, 16 xPSR,
/// 17 MSP, 18 PSP). Synchronous: DCRDR holds the value on return.
#[wasm_bindgen]
pub fn swd_reg_read(idx: u32) -> u32 {
    let emu = native_mut();
    let sys = crate::sys();
    sys.swd.dcrsr_write(idx & 0x7F);
    sys.swd.take_xfer_pending();
    debug_do_transfer(sys, &mut emu.cpu);
    sys.swd.dcrdr_read()
}

/// DCRSR-style core register write (same numbering). Synchronous.
#[wasm_bindgen]
pub fn swd_reg_write(idx: u32, value: u32) {
    let emu = native_mut();
    let sys = crate::sys();
    sys.swd.dcrdr_write(value);
    sys.swd.dcrsr_write((idx & 0x7F) | (1 << 16));
    sys.swd.take_xfer_pending();
    debug_do_transfer(sys, &mut emu.cpu);
}

// ---- JTAG TAP (shares the DP/AP file; no new tests — probe helper) ----

#[wasm_bindgen]
pub fn swd_jtag_reset() {
    crate::sys().swd.jtag_reset();
}

#[wasm_bindgen]
pub fn swd_jtag_ir(ir: u32) {
    crate::sys().swd.jtag_ir_write(ir as u8);
}

#[wasm_bindgen]
pub fn swd_jtag_idcode() -> u32 {
    crate::sys().swd.jtag_idcode()
}

#[wasm_bindgen]
pub fn swd_jtag_dp(addr: u32, rnw: bool, wdata: u32) -> u32 {
    crate::sys().swd.jtag_dp(addr, rnw, wdata)
}

/// JTAG APACC shift with explicit bank. DRW register moves data like the
/// SWD AP path (rnw=true reads, false writes); other registers are direct.
#[wasm_bindgen]
pub fn swd_jtag_ap(bank: u32, reg: u32, rnw: bool, wdata: u32) -> u32 {
    if bank == 0 && reg & 0xC == 0xC {
        if rnw {
            return swd_ap_read(bank, reg);
        }
        swd_ap_write(bank, reg, wdata);
        return 0;
    }
    crate::sys().swd.jtag_ap_reg(bank, reg, rnw, wdata)
}
