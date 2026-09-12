use std::cell::Cell;
use crate::system::System;

/// Transaction-level ARM debug-port slice (SWD + JTAG-DP + Cortex debug).
///
/// Scope decision (see sprint notes): NO GPIO tristate / clocked-edge
/// modeling. GPIO here is push-pull only, and pin-level tracing would emit
/// ~1B wire events per run for zero behavioral gain — so the probe speaks to
/// the DP register file directly through the `swd_*` exports (the same level
/// Wokwi virtual peripherals use). The V8 hot path stays untouched: the CPU
/// run loop sees one `DEBUG_HALT` flag load, and the memory paths see one
/// `WATCH_ON` load, both plain-static mirrors synced on config writes.
///
/// Contents:
/// - SWD DPv1 (DPIDR `0x2BA01477` Cortex-M3, CTRL/STAT, SELECT, RDBUFF)
/// - MEM-AP (CSW `0x23000052`, TAR, DRW, BD0-3, CFG, BASE, IDR); DRW data
///   moves through `native.rs` (the only place with both SYS and FlatMemory)
/// - Cortex debug (DHCSR/DCRSR/DCRDR/DEMCR, routed from the SCB window —
///   no new bus window, STIR/ACTRL/IDCODE precedent)
/// - 4 data watchpoints (GDB Z2/Z3/Z4): exact byte-range compare on guest
///   data accesses; the core halts AFTER the matching instruction (like HW)
/// - Minimal JTAG TAP (IDCODE `0x4BA00477`, BYPASS, DPACC/APACC, ABORT)
///   sharing the same DP/AP register file
///
/// No MMU, single core: watch slots are (addr, len, kind) ranges, no
/// DWT comparator masking. DWT stays a cycle counter (own row in the board
/// matrices); ETM/TPIU are absent.
pub struct SwdState {
    // ---- SWD DP ----
    dp_select: Cell<u32>,
    dp_ctrl_stat: Cell<u32>,
    dp_rdbuff: Cell<u32>,
    // ---- MEM-AP ----
    ap_csw: Cell<u32>,
    ap_tar: Cell<u32>,
    ap_bd: [Cell<u32>; 4],
    // ---- Cortex debug (SCB offsets 0xF0-0xFC) ----
    dhcsr: Cell<u32>,
    dcrsr: Cell<u32>,
    dcrdr: Cell<u32>,
    demcr: Cell<u32>,
    /// DCRSR write pending a register transfer (needs the CPU; drained at
    /// the next run/dispatch entry — guest DCRSR writes are near-nonexistent,
    /// the debugger path uses `swd_reg_*` directly).
    xfer_pending: Cell<bool>,
    /// DHCSR C_STEP edge pending a single step (drained the same way).
    step_pending: Cell<bool>,
    // ---- watchpoints: bit63 valid, bits[62:61] kind (1=W,2=R,3=A),
    // bits[59:32] len (bytes), bits[31:0] base address ----
    watch: [Cell<u64>; 4],
    trip_valid: Cell<bool>,
    trip_addr: Cell<u32>,
    /// 1 = write, 2 = read (the access direction that tripped).
    trip_kind: Cell<u32>,
    // ---- JTAG TAP ----
    tap_ir: Cell<u8>,
}

/// ARM Cortex-M3 SWD DP IDCODE (DPIDR).
pub const DPIDR: u32 = 0x2BA0_1477;
/// Cortex-M3 JTAG TAP IDCODE.
pub const TAP_IDCODE: u32 = 0x4BA0_0477;
/// MEM-AP identification (AMBA AHB-AP, class 0x8).
pub const MEMAP_IDR: u32 = 0x2477_0011;
pub const MEMAP_BASE: u32 = 0xE00F_F003;
pub const MEMAP_CFG: u32 = 0x0000_0000;

// 4-bit JTAG IR codes (ARM DAP default).
pub const IR_ABORT: u8 = 0x8;
pub const IR_DPACC: u8 = 0xA;
pub const IR_APACC: u8 = 0xB;
pub const IR_IDCODE: u8 = 0xE;
pub const IR_BYPASS: u8 = 0xF;

// DHCSR bits.
const DBGKEY: u32 = 0xA05F_0000;
const DH_C_DEBUGEN: u32 = 1 << 0;
const DH_C_HALT: u32 = 1 << 1;
const DH_C_STEP: u32 = 1 << 2;
const DH_S_REGRDY: u32 = 1 << 16;
const DH_S_HALT: u32 = 1 << 17;
const DH_KEY_MASK: u32 = 0xFFFF_0000;

// DP CTRL/STAT bits.
const CS_CSYSPWRUPREQ: u32 = 1 << 30;
const CS_CSYSPWRUPACK: u32 = 1 << 31;
const CS_CDBGPWRUPREQ: u32 = 1 << 28;
const CS_CDBGPWRUPACK: u32 = 1 << 29;
const CS_STICKYORUN: u32 = 1 << 1;
const CS_STICKYERR: u32 = 1 << 5;
const CS_WDATAERR: u32 = 1 << 7;
const CS_STICKY_MASK: u32 = CS_STICKYORUN | CS_STICKYERR | CS_WDATAERR;

impl Default for SwdState {
    fn default() -> Self {
        Self {
            dp_select: Cell::new(0),
            dp_ctrl_stat: Cell::new(0),
            dp_rdbuff: Cell::new(0),
            ap_csw: Cell::new(0x2300_0052),
            ap_tar: Cell::new(0),
            ap_bd: [Cell::new(0), Cell::new(0), Cell::new(0), Cell::new(0)],
            dhcsr: Cell::new(0),
            dcrsr: Cell::new(0),
            dcrdr: Cell::new(0),
            demcr: Cell::new(0),
            xfer_pending: Cell::new(false),
            step_pending: Cell::new(false),
            watch: [Cell::new(0), Cell::new(0), Cell::new(0), Cell::new(0)],
            trip_valid: Cell::new(false),
            trip_addr: Cell::new(0),
            trip_kind: Cell::new(0),
            tap_ir: Cell::new(IR_IDCODE),
        }
    }
}

fn watch_encode(kind: u32, addr: u32, len: u32) -> u64 {
    // NOTE: len is 28 bits ([59:32]) — the kind/valid bits above it must be
    // masked out on decode, or they inflate the range (found by unit test).
    (1u64 << 63) | ((kind as u64 & 3) << 61) | ((len as u64 & 0x0FFF_FFFF) << 32) | addr as u64
}

fn watch_decode(slot: u64) -> Option<(u32, u32, u32)> {
    if slot >> 63 == 0 {
        return None;
    }
    Some((((slot >> 61) & 3) as u32, (slot & 0xFFFF_FFFF) as u32, ((slot >> 32) & 0x0FFF_FFFF) as u32))
}

impl SwdState {
    // ---- SWD DP register file ----
    pub fn dp_read(&self, addr: u32) -> u32 {
        match addr & 0xC {
            0x0 => DPIDR,
            0x4 => {
                // Power-up ACKs follow their REQs (always-powered model).
                let cs = self.dp_ctrl_stat.get();
                let mut v = cs & !(CS_CSYSPWRUPACK | CS_CDBGPWRUPACK);
                if cs & CS_CSYSPWRUPREQ != 0 {
                    v |= CS_CSYSPWRUPACK;
                }
                if cs & CS_CDBGPWRUPREQ != 0 {
                    v |= CS_CDBGPWRUPACK;
                }
                v
            }
            0x8 => self.dp_select.get(),
            0xC => self.dp_rdbuff.get(),
            _ => 0,
        }
    }

    pub fn dp_write(&self, addr: u32, value: u32) {
        match addr & 0xC {
            0x4 => {
                // Sticky bits are W1C; REQ bits are stored.
                let mut cs = self.dp_ctrl_stat.get();
                cs &= !(value & CS_STICKY_MASK);
                cs = (cs & CS_STICKY_MASK) | (value & !CS_STICKY_MASK);
                self.dp_ctrl_stat.set(cs);
            }
            0x8 => self.dp_select.set(value),
            // DPIDR / RDBUFF are read-only.
            _ => {}
        }
    }

    // ---- MEM-AP banked registers (bank from SELECT[7:4]) ----
    pub fn ap_bank(&self) -> u32 {
        (self.dp_select.get() >> 4) & 0xF
    }

    /// AP register read WITHOUT data movement (CSW/TAR/BD/CFG/BASE/IDR).
    /// DRW goes through `ap_data_size()` + the native.rs data path.
    pub fn ap_reg_read(&self, bank: u32, reg: u32) -> u32 {
        match (bank, reg & 0xC) {
            (0x0, 0x0) => self.ap_csw.get(),
            (0x0, 0x4) => self.ap_tar.get(),
            (0x1, 0x0) => self.ap_bd[0].get(),
            (0x1, 0x4) => self.ap_bd[1].get(),
            (0x1, 0x8) => self.ap_bd[2].get(),
            (0x1, 0xC) => self.ap_bd[3].get(),
            (0xF, 0x4) => MEMAP_CFG,
            (0xF, 0x8) => MEMAP_BASE,
            (0xF, 0xC) => MEMAP_IDR,
            _ => 0,
        }
    }

    pub fn ap_reg_write(&self, bank: u32, reg: u32, value: u32) {
        match (bank, reg & 0xC) {
            (0x0, 0x0) => self.ap_csw.set(value),
            (0x0, 0x4) => self.ap_tar.set(value),
            (0x1, 0x0) => self.ap_bd[0].set(value),
            (0x1, 0x4) => self.ap_bd[1].set(value),
            (0x1, 0x8) => self.ap_bd[2].set(value),
            (0x1, 0xC) => self.ap_bd[3].set(value),
            _ => {}
        }
    }

    /// CSW SIZE field (bits[2:0]) → access width in bytes (000=8b, 001=16b,
    /// 010=32b). Reset 0x...52 → 010 → word.
    pub fn ap_data_size(&self) -> u8 {
        match self.ap_csw.get() & 7 {
            0 => 1,
            1 => 2,
            _ => 4,
        }
    }

    /// Raw DHCSR C-bits (for halt-mirror re-sync after a single step).
    pub(crate) fn dhcsr_raw(&self) -> u32 {
        self.dhcsr.get()
    }

    /// Raw TAR (for the AP data path in native.rs).
    pub fn tar(&self) -> u32 {
        self.ap_tar.get()
    }

    /// TAR auto-increment (CSW AddrInc==01 single): advance by access size.
    pub fn ap_tar_advance(&self) {
        if (self.ap_csw.get() >> 4) & 3 == 1 {
            self.ap_tar.set(self.ap_tar.get().wrapping_add(self.ap_data_size() as u32));
        }
    }

    /// Latch the last AP data value for RDBUFF reads.
    pub fn ap_latch_rdbuff(&self, value: u32) {
        self.dp_rdbuff.set(value);
    }

    // ---- Cortex debug registers (SCB offsets 0xF0-0xFC) ----
    fn halted(&self) -> bool {
        crate::system::debug_halted()
    }

    pub fn dhcsr_read(&self) -> u32 {
        let mut v = self.dhcsr.get() & (DH_C_DEBUGEN | DH_C_HALT | DH_C_STEP);
        // S_REGRDY: synchronous transfers — ready unless one is pending.
        if !self.xfer_pending.get() {
            v |= DH_S_REGRDY;
        }
        if self.halted() {
            v |= DH_S_HALT;
        }
        v
    }

    pub fn dhcsr_write(&self, value: u32) {
        if value & DH_KEY_MASK != DBGKEY {
            return; // DBGKEY required — silent ignore like silicon.
        }
        let mut cur = self.dhcsr.get();
        cur = (cur & !(DH_C_DEBUGEN | DH_C_HALT | DH_C_STEP)) | (value & (DH_C_DEBUGEN | DH_C_HALT | DH_C_STEP));
        self.dhcsr.set(cur);
        if value & DH_C_STEP != 0 && value & DH_C_DEBUGEN != 0 {
            self.step_pending.set(true);
        }
        crate::system::sync_debug_halt_from_dhcsr(cur);
    }

    pub fn dcrsr_read(&self) -> u32 {
        self.dcrsr.get()
    }

    pub fn dcrsr_write(&self, value: u32) {
        self.dcrsr.set(value);
        // Synchronous model: the transfer itself runs at the next run entry
        // (it needs the CPU), but S_REGRDY semantics stay exact — a poller
        // sees NOT-ready until the drain, then ready with DCRDR valid.
        self.xfer_pending.set(true);
    }

    pub fn dcrdr_read(&self) -> u32 {
        self.dcrdr.get()
    }

    pub fn dcrdr_write(&self, value: u32) {
        self.dcrdr.set(value);
    }

    pub fn demcr_read(&self) -> u32 {
        self.demcr.get()
    }

    pub fn demcr_write(&self, value: u32) {
        self.demcr.set(value);
    }

    pub fn take_xfer_pending(&self) -> bool {
        self.xfer_pending.replace(false)
    }

    pub fn take_step_pending(&self) -> bool {
        self.step_pending.replace(false)
    }

    pub fn trcena(&self) -> bool {
        self.demcr.get() & (1 << 24) != 0
    }

    pub fn vc_harderr(&self) -> bool {
        self.demcr.get() & (1 << 10) != 0
    }

    /// Halt from inside the model (watchpoint trip, VC_HARDERR): sets the
    /// sticky S_HALT readback via the shared halt mirror.
    pub fn halt_from_model(&self) {
        crate::system::set_debug_halt(true);
    }

    /// Debugger resume: clears C_HALT (like the probe writing DHCSR) and
    /// the halt mirror. C_DEBUGEN stays set across halts, like silicon.
    pub fn resume(&self) {
        let cur = self.dhcsr.get() & !DH_C_HALT & !DH_C_STEP;
        self.dhcsr.set(cur);
        self.step_pending.set(false);
        crate::system::set_debug_halt(false);
    }

    /// External halt request (probe/JTAG/GDB Ctrl-C path).
    pub fn halt(&self) {
        let cur = self.dhcsr.get() | DH_C_DEBUGEN | DH_C_HALT;
        self.dhcsr.set(cur);
        crate::system::set_debug_halt(true);
    }

    // ---- watchpoints ----
    /// Install a watchpoint; returns the slot (0-3) or -1 when full.
    /// kind: 1 = write (Z2), 2 = read (Z3), 3 = access (Z4).
    pub fn add_watch(&self, kind: u32, addr: u32, len: u32) -> i32 {
        let kind = kind & 3;
        if kind == 0 {
            return -1;
        }
        let len = len.max(1);
        for (i, slot) in self.watch.iter().enumerate() {
            if slot.get() >> 63 == 0 {
                slot.set(watch_encode(kind, addr, len));
                crate::system::sync_watch_gate(self);
                return i as i32;
            }
        }
        -1
    }

    pub fn remove_watch(&self, slot: u32) {
        if (slot as usize) < self.watch.len() {
            self.watch[slot as usize].set(0);
            crate::system::sync_watch_gate(self);
        }
    }

    pub fn any_watch(&self) -> bool {
        self.watch.iter().any(|s| s.get() >> 63 != 0)
    }

    /// Cold watchpoint check (called only when WATCH_ON is set): exact
    /// byte-range overlap against armed slots with a matching direction.
    /// Records the FIRST trip (sticky until `take_trip`) and halts; the
    /// current instruction still completes — the run loop stops on top of
    /// the NEXT one, matching HW halt-after-access.
    pub fn check_watch(&self, addr: u32, size: u32, is_write: bool) {
        let end = addr.wrapping_add(size.max(1) as u32);
        for slot in &self.watch {
            let s = slot.get();
            let Some((kind, base, len)) = watch_decode(s) else {
                continue;
            };
            let dir_ok = match kind {
                1 => is_write,
                2 => !is_write,
                3 => true,
                _ => false,
            };
            if !dir_ok {
                continue;
            }
            let wend = base.wrapping_add(len.max(1));
            // Overlap with wrap-safe compare (watch regions never wrap in
            // practice; the wrapping_add keeps debug arithmetic total).
            let overlap = addr < wend && base < end;
            if overlap && !self.trip_valid.get() {
                self.trip_valid.set(true);
                self.trip_addr.set(addr);
                self.trip_kind.set(if is_write { 1 } else { 2 });
                self.halt_from_model();
            }
        }
    }

    /// Take the pending watch trip: [addr, dir] (dir 1=write, 2=read), or
    /// empty. Clears the latch; the halt stays until resume.
    pub fn take_trip(&self) -> Option<(u32, u32)> {
        if self.trip_valid.replace(false) {
            Some((self.trip_addr.get(), self.trip_kind.get()))
        } else {
            None
        }
    }

    // ---- JTAG TAP (shares the DP/AP file above) ----
    pub fn jtag_reset(&self) {
        self.tap_ir.set(IR_IDCODE);
    }

    pub fn jtag_ir(&self) -> u8 {
        self.tap_ir.get()
    }

    pub fn jtag_ir_write(&self, ir: u8) {
        self.tap_ir.set(ir & 0xF);
    }

    pub fn jtag_idcode(&self) -> u32 {
        TAP_IDCODE
    }

    /// DPACC shift: (addr[3:2], RnW, wdata) → rdata. Reads latch RDBUFF
    /// for APACC-read-then-RDBUFF flows; writes are direct.
    pub fn jtag_dp(&self, addr: u32, rnw: bool, wdata: u32) -> u32 {
        if rnw {
            self.dp_read(addr)
        } else {
            // ABORT register lives at DP 0x0 write: clears sticky bits.
            if addr & 0xC == 0 {
                let mut cs = self.dp_ctrl_stat.get();
                cs &= !(wdata & CS_STICKY_MASK);
                self.dp_ctrl_stat.set(cs);
                0
            } else {
                self.dp_write(addr, wdata);
                0
            }
        }
    }

    /// APACC shift with explicit bank (SELECT[7:4] is the SWD way; JTAG
    /// probes that bypass SELECT use this). DRW data movement is handled
    /// by the native.rs caller via `ap_data_size`/`ap_tar` — this covers
    /// the register file only.
    pub fn jtag_ap_reg(&self, bank: u32, reg: u32, rnw: bool, wdata: u32) -> u32 {
        if rnw {
            let v = self.ap_reg_read(bank, reg);
            self.ap_latch_rdbuff(v);
            v
        } else {
            self.ap_reg_write(bank, reg, wdata);
            0
        }
    }

    /// Debug-register transfer for DCRSR (runs with CPU access from
    /// native.rs at run/dispatch entry). REGSEL mapping: 0-12 → R0-R12,
    /// 13 → current SP, 14 → LR, 15 → PC, 16 → xPSR, 17 → MSP, 18 → PSP.
    /// REGWnR (bit 16) selects write (DCRDR → reg) vs read (reg → DCRDR).
    pub fn transfer_regsel(&self) -> u32 {
        self.dcrsr.get() & 0x7F
    }

    pub fn transfer_is_write(&self) -> bool {
        self.dcrsr.get() & (1 << 16) != 0
    }
}

/// Route a Cortex-debug SCB-window access (offsets 0xF0-0xFC) to the SWD
/// state. Returns None for non-debug offsets (normal SCB handling).
pub fn scb_debug_read(sys: &System, offset: u32) -> Option<u32> {
    match offset {
        0xF0 => Some(sys.swd.dhcsr_read()),
        0xF4 => Some(sys.swd.dcrsr_read()),
        0xF8 => Some(sys.swd.dcrdr_read()),
        0xFC => Some(sys.swd.demcr_read()),
        _ => None,
    }
}

/// Returns true when the offset was a debug register (handled).
pub fn scb_debug_write(sys: &System, offset: u32, value: u32) -> bool {
    match offset {
        0xF0 => {
            sys.swd.dhcsr_write(value);
            true
        }
        0xF4 => {
            sys.swd.dcrsr_write(value);
            true
        }
        0xF8 => {
            sys.swd.dcrdr_write(value);
            true
        }
        0xFC => {
            sys.swd.demcr_write(value);
            true
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::with_sys;

    #[test]
    fn dp_registers_default_and_select() {
        with_sys(|sys| {
            assert_eq!(sys.swd.dp_read(0x0), DPIDR, "DPIDR reads Cortex-M3 ID");
            assert_eq!(sys.swd.dp_read(0xC), 0, "RDBUFF reset 0");
            sys.swd.dp_write(0x8, 0x0000_00F0);
            assert_eq!(sys.swd.dp_read(0x8), 0x0000_00F0, "SELECT stored");
            assert_eq!(sys.swd.ap_bank(), 0xF, "bank from SELECT[7:4]");
            // DPIDR/RDBUFF writes are ignored (read-only).
            sys.swd.dp_write(0x0, 0xFFFF_FFFF);
            assert_eq!(sys.swd.dp_read(0x0), DPIDR, "DPIDR write ignored");
        });
    }

    #[test]
    fn ctrl_stat_power_ack_and_sticky_w1c() {
        with_sys(|sys| {
            // REQs drive ACKs (always-powered model).
            sys.swd.dp_write(0x4, CS_CSYSPWRUPREQ | CS_CDBGPWRUPREQ);
            let v = sys.swd.dp_read(0x4);
            assert_ne!(v & CS_CSYSPWRUPACK, 0, "SYS ACK follows REQ");
            assert_ne!(v & CS_CDBGPWRUPACK, 0, "DBG ACK follows REQ");
            // Sticky bits set internally clear on W1C (REQ bits ride along —
            // CTRL/STAT is RW, so a write without REQs would drop them).
            let cs = sys.swd.dp_ctrl_stat.get() | CS_STICKYORUN | CS_WDATAERR;
            sys.swd.dp_ctrl_stat.set(cs);
            sys.swd.dp_write(0x4, CS_CSYSPWRUPREQ | CS_CDBGPWRUPREQ | CS_STICKYORUN | CS_WDATAERR);
            let v = sys.swd.dp_read(0x4);
            assert_eq!(v & (CS_STICKYORUN | CS_WDATAERR), 0, "stickies W1C");
            assert_ne!(v & CS_CSYSPWRUPACK, 0, "REQ/ACK survive W1C");
        });
    }

    #[test]
    fn mem_ap_banked_regs_and_csw_size() {
        with_sys(|sys| {
            assert_eq!(sys.swd.ap_reg_read(0xF, 0xC), MEMAP_IDR, "MEM-AP IDR");
            assert_eq!(sys.swd.ap_reg_read(0xF, 0x8), MEMAP_BASE, "MEM-AP BASE");
            assert_eq!(sys.swd.ap_reg_read(0x0, 0x0), 0x2300_0052, "CSW reset");
            assert_eq!(sys.swd.ap_data_size(), 4, "default 32-bit");
            sys.swd.ap_reg_write(0x0, 0x0, 0x2300_0040); // SIZE=000 byte
            assert_eq!(sys.swd.ap_data_size(), 1, "CSW SIZE=000 byte");
            sys.swd.ap_reg_write(0x0, 0x4, 0x2000_0100); // TAR
            sys.swd.ap_reg_write(0x0, 0x0, 0x2300_0052 | (1 << 4)); // AddrInc single
            sys.swd.ap_tar_advance();
            assert_eq!(sys.swd.ap_reg_read(0x0, 0x4), 0x2000_0104, "TAR auto-inc by size");
            sys.swd.ap_reg_write(0x1, 0x8, 0xDEAD_BEEF); // BD2
            assert_eq!(sys.swd.ap_reg_read(0x1, 0x8), 0xDEAD_BEEF, "BD2 stored");
        });
    }

    #[test]
    fn dhcsr_key_and_halt_mirror() {
        with_sys(|sys| {
            // No key → ignored.
            sys.swd.dhcsr_write(DH_C_DEBUGEN | DH_C_HALT);
            assert!(!crate::system::debug_halted(), "keyless DHCSR write ignored");
            // Enable + halt with key → halted mirror set.
            sys.swd.dhcsr_write(DBGKEY | DH_C_DEBUGEN | DH_C_HALT);
            assert!(crate::system::debug_halted(), "C_HALT halts");
            assert_ne!(sys.swd.dhcsr_read() & DH_S_HALT, 0, "S_HALT readback");
            // Resume clears C_HALT + mirror, keeps C_DEBUGEN.
            sys.swd.resume();
            assert!(!crate::system::debug_halted(), "resume clears halt");
            assert_ne!(sys.swd.dhcsr_read() & DH_C_DEBUGEN, 0, "C_DEBUGEN sticky");
            assert_eq!(sys.swd.dhcsr_read() & DH_S_HALT, 0, "S_HALT cleared");
        });
    }

    #[test]
    fn dhcsr_step_sets_pending() {
        with_sys(|sys| {
            sys.swd.dhcsr_write(DBGKEY | DH_C_DEBUGEN | DH_C_STEP);
            assert!(sys.swd.take_step_pending(), "C_STEP pends a step");
            assert!(!sys.swd.take_step_pending(), "pending is one-shot");
        });
    }

    #[test]
    fn dcrsr_transfer_moves_regs() {
        with_sys(|sys| {
            let mut cpu = crate::cpu::Cpu::new(0x2000_8000, 0x0800_0101);
            cpu.regs.r[3] = 0x1234_5678;
            // Read R3 → DCRDR (REGSEL=3, RnW=read).
            sys.swd.dcrdr_write(0);
            sys.swd.dcrsr_write(3);
            assert!(sys.swd.take_xfer_pending(), "xfer pends");
            crate::native::debug_do_transfer(sys, &mut cpu);
            assert_eq!(sys.swd.dcrdr_read(), 0x1234_5678, "R3 lands in DCRDR");
            assert_ne!(sys.swd.dhcsr_read() & DH_S_REGRDY, 0, "S_REGRDY after drain");
            // Write DCRDR → R7 (REGSEL=7, REGWnR=1).
            sys.swd.dcrdr_write(0xA5A5_5A5A);
            sys.swd.dcrsr_write((1 << 16) | 7);
            crate::native::debug_do_transfer(sys, &mut cpu);
            assert_eq!(cpu.regs.r[7], 0xA5A5_5A5A, "DCRDR lands in R7");
            // xPSR round-trips through REGSEL=16.
            cpu.regs.xpsr = 0x8100_0000;
            sys.swd.dcrsr_write(16);
            crate::native::debug_do_transfer(sys, &mut cpu);
            assert_eq!(sys.swd.dcrdr_read(), 0x8100_0000, "xPSR readable");
        });
    }

    #[test]
    fn watchpoint_trip_and_first_wins() {
        with_sys(|sys| {
            assert_eq!(sys.swd.add_watch(1, 0x2000_0100, 4), 0, "slot 0");
            assert_eq!(sys.swd.add_watch(3, 0x2000_0200, 1), 1, "slot 1");
            assert!(crate::system::watch_on(), "WATCH_ON armed");
            // Write watch ignores reads.
            sys.swd.check_watch(0x2000_0102, 1, false);
            assert!(sys.swd.take_trip().is_none(), "read does not trip write watch");
            assert!(!crate::system::debug_halted(), "no halt without trip");
            // Overlapping write trips (byte 2 of a 4-byte watch).
            sys.swd.check_watch(0x2000_0102, 1, true);
            assert!(crate::system::debug_halted(), "trip halts");
            let t = sys.swd.take_trip().expect("trip latched");
            assert_eq!(t, (0x2000_0102, 1), "trip addr+dir");
            assert!(sys.swd.take_trip().is_none(), "trip one-shot");
            // Access watch trips on reads too.
            sys.swd.resume();
            sys.swd.check_watch(0x2000_0200, 1, false);
            let t = sys.swd.take_trip().expect("access watch trips on read");
            assert_eq!(t.1, 2, "read direction recorded");
            // Remove all → gate disarms.
            sys.swd.remove_watch(0);
            sys.swd.remove_watch(1);
            sys.swd.resume();
            assert!(!crate::system::watch_on(), "WATCH_ON disarmed");
            assert_eq!(sys.swd.add_watch(0, 0x2000_0100, 4), -1, "kind 0 rejected");
        });
    }

    #[test]
    fn jtag_shares_dp_and_abort_clears() {
        with_sys(|sys| {
            sys.swd.jtag_reset();
            assert_eq!(sys.swd.jtag_ir(), IR_IDCODE, "reset selects IDCODE");
            assert_eq!(sys.swd.jtag_idcode(), TAP_IDCODE, "TAP IDCODE");
            sys.swd.jtag_ir_write(IR_DPACC);
            // DPACC write to SELECT, then read back through the SWD path.
            sys.swd.jtag_dp(0x8, false, 0x0000_0010);
            assert_eq!(sys.swd.dp_read(0x8), 0x0000_0010, "JTAG DPACC shares DP");
            let v = sys.swd.jtag_dp(0x8, true, 0);
            assert_eq!(v, 0x0000_0010, "JTAG DPACC readback");
            // ABORT clears stickies.
            let cs = sys.swd.dp_ctrl_stat.get() | CS_STICKYERR;
            sys.swd.dp_ctrl_stat.set(cs);
            sys.swd.jtag_dp(0x0, false, CS_STICKYERR);
            assert_eq!(sys.swd.dp_read(0x4) & CS_STICKYERR, 0, "ABORT clears STICKYERR");
            // APACC reg path latches RDBUFF.
            sys.swd.jtag_ir_write(IR_APACC);
            sys.swd.ap_reg_write(0x1, 0x0, 0xCAFE_F00D);
            let v = sys.swd.jtag_ap_reg(0x1, 0x0, true, 0);
            assert_eq!(v, 0xCAFE_F00D, "APACC BD0 read");
            assert_eq!(sys.swd.dp_read(0xC), 0xCAFE_F00D, "RDBUFF latched");
        });
    }

    #[test]
    fn guest_store_trips_watch_and_stops_run() {
        use crate::cpu::mem::Memory;
        let _held = crate::test_util::lock();
        crate::init();
        let sys = crate::sys();
        // Guest: str r0,[r1] (0x6008) with r1 on watch, then b.n loop.
        let mut mem = crate::cpu::mem::FlatMemory::new(0x1000, 0x10000);
        mem.write16(0x2000_2000, 0x6008);
        mem.write16(0x2000_2002, 0xE7FE);
        let watch_at = 0x2000_0100u32;
        assert_eq!(sys.swd.add_watch(1, watch_at, 4), 0, "watch armed");
        let mut cpu = crate::cpu::Cpu::new(0x2000_8000, 0x2000_2001);
        cpu.dsp = false;
        cpu.regs.r[0] = 0xDEAD_BEEF;
        cpu.regs.r[1] = watch_at;
        let done = cpu.run(sys, &mut mem, 10);
        assert_eq!(done, 1, "run stops right after the tripping store");
        assert!(crate::system::debug_halted(), "core halted");
        assert_eq!(mem.read32(watch_at), 0xDEAD_BEEF, "store still landed (halt-after)");
        let t = sys.swd.take_trip().expect("trip latched");
        assert_eq!(t, (watch_at, 1), "trip addr+dir");
        // Resume → the loop runs out the full budget.
        sys.swd.resume();
        let done = cpu.run(sys, &mut mem, 10);
        assert_eq!(done, 10, "resumed core runs free");
        sys.swd.remove_watch(0);
        sys.swd.resume();
    }

    #[test]
    fn dhcsr_guest_write_halts_run() {
        let _held = crate::test_util::lock();
        crate::init();
        let sys = crate::sys();
        // Guest writes DHCSR (SCB+0xF0) through the peripheral bus, both
        // maps route it (STIR/ACTRL/IDCODE precedent — no bus window).
        sys.p.write(sys, 0xE000_EDF0, 4, 0xA05F_0000 | 3); // KEY + EN + HALT
        assert!(crate::system::debug_halted(), "guest C_HALT halts");
        let mut mem = crate::cpu::mem::FlatMemory::new(0x1000, 0x10000);
        let mut cpu = crate::cpu::Cpu::new(0x2000_8000, 0x2000_2001);
        assert_eq!(cpu.run(sys, &mut mem, 100), 0, "halted run executes nothing");
        sys.swd.resume();
        assert!(!crate::system::debug_halted(), "resume releases");
    }

    #[test]
    fn demcr_store_and_vc_bit() {        with_sys(|sys| {
            sys.swd.demcr_write((1 << 24) | (1 << 10));
            assert!(sys.swd.trcena(), "TRCENA decoded");
            assert!(sys.swd.vc_harderr(), "VC_HARDERR decoded");
            assert_eq!(sys.swd.demcr_read(), (1 << 24) | (1 << 10), "DEMCR readback");
        });
    }
}
