//! Native CPU backend (Path B) for the JS drivers.
//!
//! Owns a [`Cpu`] + [`FlatMemory`] pair that runs guest firmware with zero
//! JS crossings per instruction, against the SAME peripheral model/DMA/NVIC
//! as the Unicorn backend. The JS drivers (cli.mjs, emulator.js) pick it
//! with `cpu: 'rust'` (benchmarking; Unicorn stays the default).
//!
//! Driver loop shape per batch (mirrors the Unicorn path):
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
//! included); the Unicorn path credits full batches instead. Both stay
//! within ~1% on real firmware.

use std::sync::atomic::{AtomicBool, Ordering};
use wasm_bindgen::prelude::*;

use crate::cpu::{
    mem::{FlatMemory, Memory},
    Cpu,
};

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

/// Create the CPU + guest RAM. Call after init()/init_svd() and before load.
/// `dsp` is always false here (Cortex-M3 has no DSP extension).
#[wasm_bindgen]
pub fn rustcpu_init(sp: u32, pc: u32, flash_size: u32, ram_size: u32) {
    let mut cpu = Cpu::new(sp, pc | 1);
    cpu.dsp = false;
    cpu.deliver_irqs = false; // lazy batch-boundary dispatch (Unicorn parity)
    let mem = FlatMemory::new(flash_size as usize, ram_size as usize);
    unsafe {
        *std::ptr::addr_of_mut!(NATIVE) = Some(NativeEmu { cpu, mem });
    }
}

/// Load firmware bytes at a guest physical address (bypasses flash
/// protection, like the Unicorn driver's mem_write at load time).
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
    crate::set_intr_masks(emu.cpu.regs.primask, 0);
    let mut done = emu.cpu.run(sys, &mut emu.mem, slice);
    if let Some(f) = emu.cpu.fault.take() {
        if f.op1 & 0xFF00 == 0xDF00 {
            // SVC: step past it and run the SVCall handler synchronously.
            emu.cpu.regs.r[15] = f.pc.wrapping_add(2) | 1;
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
/// is a loud decoder gap, like the Unicorn path's unmapped faults.)
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

/// Raw guest-memory access (RAM + flash; flash writes stay protected, use
/// rustcpu_load for firmware). Backs memRead32 + the hi2c Mode RAM patch.
#[wasm_bindgen]
pub fn rustcpu_mem_read(addr: u32, len: u32) -> Vec<u8> {
    let emu = native_mut();
    let mut out = Vec::with_capacity(len as usize);
    for k in 0..len {
        out.push(emu.mem.read8(addr.wrapping_add(k)));
    }
    out
}

#[wasm_bindgen]
pub fn rustcpu_mem_write(addr: u32, data: &[u8]) {
    let emu = native_mut();
    for (k, &b) in data.iter().enumerate() {
        emu.mem.write8(addr.wrapping_add(k as u32), b);
    }
}

/// Whole DMA pump against Rust RAM: build the op plan and execute it with no
/// JS RAM crossings (the Unicorn driver's mem_read/mem_write per op).
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
/// hi2c->Mode == 0x22 before reading DR). Same condition as the Unicorn
/// memWriteHook; the driver then patches RAM *(0x200002d8)+0x3D.
#[wasm_bindgen]
pub fn rustcpu_i2c_hook_fired() -> bool {
    crate::sys().i2c_dr_hook.take()
}

// ---- Peripheral write tap (onPeriphWrite parity) --------------------------
// The page taps the peripheral bus via Unicorn memWriteHook; Rust→Rust model
// writes never cross JS, so the tap records (addr, size, value) in
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
