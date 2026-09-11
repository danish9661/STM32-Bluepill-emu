//! USB full-speed device (STM32F103 @ 0x4000_5C00, IRQs 19/20, packet memory @
//! 0x4000_6000). Synchronous transaction model: endpoint events complete when
//! the firmware arms them, which is exact for control/bulk/interrupt firmware.
//!
//! Implemented: EP0R-EP7R with real toggle semantics (STAT_TX/RX toggle on
//! 1-write, CTR_TX/RX clear on 0-write, DTOG read-only), CNTR masks, ISTR
//! (event flags write-0-clear; CTR/DIR/EP_ID derived from endpoint flags like
//! hardware), DADDR, BTABLE, 1024 B packet-memory window with byte-exact sub-word access, USB RESET event on
//! FRES release, SETUP/OUT injection (host -> device) with DTOG sequencing,
//! IN completion (device -> host) drained as `VmEvent::UsbIn`, CTR/RESET IRQs
//! on the low-priority vector (IRQ 20; isochronous endpoints are treated as
//! bulk), SOF engine (1ms frames off the instruction counter, FNR + SOF IRQ),
//! suspend/resume with wakeup IRQ 42 (FSUSP force; SOF reception is bus
//! activity so an attached host never idles into suspend; resume on traffic,
//! RESUME pulse, or FSUSP clear), double-buffered bulk endpoints.
//!
//! Buffer-table layout (matches ST's F1 HAL: stride 16 APB bytes/endpoint,
//! `PMA_ACCESS = 2`): each endpoint owns a descriptor pair — DESC0 is
//! ADDR @ +0 / CNT @ +4, DESC1 is ADDR @ +8 / CNT @ +12. Single-buffered
//! endpoints send via DESC0 and receive via DESC1; double-buffered bulk
//! endpoints ping-pong between DESC0 (DTOG = 0) and DESC1 (DTOG = 1).
//! Buffer addresses are PMA words: word W lives at APB bytes 2W with a
//! 4-byte stride per 16-bit word (only the even halfword of each 32-bit
//! APB slot is wired, like `USB_WritePMA`/`USB_ReadPMA`).
//!
//! Deliberately absent: ESOF generation (host never misses in emulation),
//! isochronous CTR, PDWN gating (stored only), remote-wakeup electricals
//! beyond the WKUP flag.

use crate::system::{System, VmEvent};
use super::Peripheral;

pub const USB_BASE: u32 = 0x4000_5C00;
/// End (exclusive) of the USB window: registers + 1024 B packet-memory window.
pub const USB_END: u32 = 0x4000_6400;
/// APB-window bytes for packet memory (0x4000_6000 + 0..1024). The F1 PMA is
/// sparsely mapped (`PMA_ACCESS = 2`): each 16-bit word lives at a 4-byte
/// APB stride, so 256 words span the full 1024-byte window.
const PMA_BYTES: usize = 1024;

// EPnR bits.
const EA_MASK: u32 = 0x000F;
const STAT_TX_MASK: u32 = 0x0030;
const DTOG_TX: u32 = 0x0040;
const CTR_TX: u32 = 0x0080;
const EP_KIND: u32 = 0x0100;
const EP_TYPE_MASK: u32 = 0x0600;
const SETUP_BIT: u32 = 0x0800;
const STAT_RX_MASK: u32 = 0x3000;
const DTOG_RX: u32 = 0x4000;
const CTR_RX: u32 = 0x8000;
const STAT_NAK: u32 = 0x2;
const STAT_VALID: u32 = 0x3;
// ISTR bits.
const ISTR_CTR: u32 = 1 << 15;
const ISTR_DIR: u32 = 1 << 4;
const ISTR_SOF: u32 = 1 << 9;
const ISTR_SUSP: u32 = 1 << 11;
const ISTR_WKUP: u32 = 1 << 12;
// CNTR interrupt-enable bits.
const CNTR_CTRM: u32 = 1 << 15;
const CNTR_RESETM: u32 = 1 << 10;
const CNTR_SOFM: u32 = 1 << 9;
const CNTR_SUSPM: u32 = 1 << 11;
const CNTR_WKUPM: u32 = 1 << 12;
/// Low-priority USB vector (all CTR/RESET events; no isochronous traffic).
pub const USB_LP_IRQ: i32 = 20;
/// High-priority USB vector (shared with CAN1 TX): CTR on isochronous
/// endpoints, which silicon routes to the HP line.
pub const USB_HP_IRQ: i32 = 19;
/// USB wakeup vector (WKUP event only).
pub const USB_WKUP_IRQ: i32 = 42;
/// Instructions per USB frame (1ms @ 72MHz): the SOF engine's tick.
const SOF_PERIOD: u64 = 72_000;

fn stat_tx(r: u32) -> u32 { (r >> 4) & 3 }
fn stat_rx(r: u32) -> u32 { (r >> 12) & 3 }
fn ep_type(r: u32) -> u32 { (r >> 9) & 3 }
/// Isochronous endpoint type (EP_TYPE = 10).
const TYPE_ISO: u32 = 0x2;
/// CNTR power-down bit: the USB macro is dead (no RX/TX, no IRQs, SOF frozen).
const CNTR_PDWN: u16 = 1 << 1;
/// CNTR force-suspend bit (firmware-owned suspend).
const CNTR_FSUSP: u16 = 1 << 3;

pub struct Usb {
    ep: [u16; 8],
    cntr: u16,
    istr: u16,
    daddr: u8,
    btable: u16,
    pma: Vec<u16>,
    /// SOF engine: 11-bit frame number + sub-frame instruction accumulator.
    frame: u16,
    sof_acc: u64,
    last_tick: u64,
    /// Suspend state: set by FSUSP or detach, cleared on resume/reset.
    suspended: bool,
    /// Detached from the host (pull-up off): no tokens arrive, IN never
    /// completes, SOF frozen. Cleared by the next bus reset (reattach).
    detached: bool,
    /// Device-initiated resume pulse (CNTR.RESUME): clears after one frame.
    resume_at: u64,
}

impl Usb {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "USB" {
            Some(Box::new(Usb {
                ep: [0; 8],
                cntr: 0x0003, // FRES + PDWN out of reset
                istr: 0,
                daddr: 0,
                btable: 0,
                pma: vec![0; PMA_BYTES / 2],
                frame: 0,
                sof_acc: 0,
                last_tick: crate::system::instruction_count(),
                suspended: false,
                detached: false,
                resume_at: 0,
            }))
        } else {
            None
        }
    }

    fn pma_byte(&self, off: usize) -> u8 {
        let h = self.pma.get(off / 2).copied().unwrap_or(0);
        ((h >> ((off % 2) * 8)) & 0xFF) as u8
    }

    fn pma_set_byte(&mut self, off: usize, b: u8) {
        if off >= PMA_BYTES {
            return;
        }
        let i = off / 2;
        let shift = (off % 2) * 8;
        self.pma[i] = (self.pma[i] & !(0xFF << shift)) | ((b as u16) << shift);
    }

    fn pma_half(&self, off: usize) -> u16 {
        self.pma.get(off / 2).copied().unwrap_or(0)
    }

    fn pma_set_half(&mut self, off: usize, v: u16) {
        if off / 2 < self.pma.len() {
            self.pma[off / 2] = v;
        }
    }

    /// Descriptor-pair base (APB bytes) for endpoint n: DESC0 at +0
    /// (ADDR) / +4 (CNT), DESC1 at +8 (ADDR) / +12 (CNT).
    fn desc_pair(&self, n: usize, second: bool) -> usize {
        let base = (self.btable as usize) & 0x1F8;
        base + n * 16 + if second { 8 } else { 0 }
    }

    /// Packet-data byte i of the buffer at PMA word address `word`
    /// (`PMA_ACCESS = 2`: word W at APB bytes 2W, 4-byte stride).
    fn data_byte(&self, word: usize, i: usize) -> u8 {
        self.pma_byte(word * 2 + (i >> 1) * 4 + (i & 1))
    }

    fn data_set_byte(&mut self, word: usize, i: usize, b: u8) {
        let off = word * 2 + (i >> 1) * 4 + (i & 1);
        self.pma_set_byte(off, b);
    }

    /// APB-byte span check for a `len`-byte transfer at word `word`.
    fn data_fits(&self, word: usize, len: usize) -> bool {
        if len == 0 {
            return true;
        }
        match word
            .checked_mul(2)
            .and_then(|b| b.checked_add(((len - 1) >> 1) * 4 + ((len - 1) & 1)))
        {
            Some(end) => end < PMA_BYTES,
            None => false,
        }
    }

    /// Pend the low-priority USB IRQ when its CNTR mask bit is set.
    /// Dead while PDWN holds (the macro is off).
    fn irq(&mut self, sys: &System, mask_bit: u32) {
        if self.cntr & CNTR_PDWN != 0 {
            return;
        }
        if self.cntr as u32 & mask_bit != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(USB_LP_IRQ);
        }
    }

    /// CTR completion IRQ: isochronous endpoints go to the high-priority
    /// vector (shared with CAN1 TX), everything else to low-priority.
    fn ctr_irq(&mut self, sys: &System, n: usize) {
        if self.cntr & CNTR_PDWN != 0 {
            return;
        }
        if self.cntr as u32 & CNTR_CTRM != 0 {
            let irq = if self.is_iso(n) { USB_HP_IRQ } else { USB_LP_IRQ };
            sys.p.nvic.borrow_mut().set_intr_pending(irq);
        }
    }

    /// Pend the wakeup vector (WKUP event only).
    fn irq_wkup(&mut self, sys: &System) {
        if self.cntr as u32 & CNTR_WKUPM != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(USB_WKUP_IRQ);
        }
    }

    /// SOF engine + suspend tracking, once per batch (instruction-delta).
    /// A frame (1ms @ 72MHz = 72000 instr) sets SOF (IRQ via SOFM) unless
    /// suspended. SOF reception IS bus activity, so an attached host (which
    /// sends SOF every frame) never lets the 3ms suspend timer expire —
    /// auto-suspend only happens when the host goes silent, which the
    /// always-attached emulated host never does. Firmware-forced suspend
    /// (FSUSP) still suspends; resume comes from the RESUME pulse or
    /// firmware clearing FSUSP. A device RESUME pulse (CNTR.4) clears after
    /// a frame with the same wake sequence.
    fn tick_usb(&mut self, sys: &System) {
        use crate::system::instruction_count;
        let now = instruction_count();
        let delta = now.wrapping_sub(self.last_tick);
        self.last_tick = now;
        if delta == 0 {
            return;
        }
        if self.suspended {
            // Device-initiated resume pulse still times out while suspended.
            if self.resume_at != 0 && now >= self.resume_at {
                self.resume_at = 0;
                self.cntr &= !(1 << 4);
                self.suspended = false;
                self.istr &= !(ISTR_SUSP as u16);
                self.istr |= ISTR_WKUP as u16;
                self.sof_acc = 0;
                self.irq_wkup(sys);
            }
            return;
        }
        // PDWN freezes the frame engine (keep the tick fresh so power-up
        // doesn't see a false delta burst).
        if self.cntr & CNTR_PDWN != 0 {
            return;
        }
        self.sof_acc += delta;
        while self.sof_acc >= SOF_PERIOD {
            self.sof_acc -= SOF_PERIOD;
            self.frame = (self.frame + 1) & 0x7FF;
            self.istr |= ISTR_SOF as u16;
            self.irq(sys, CNTR_SOFM);
            if self.resume_at != 0 && now >= self.resume_at {
                self.resume_at = 0;
                self.cntr &= !(1 << 4);
                self.istr |= ISTR_WKUP as u16;
                self.irq_wkup(sys);
            }
        }
    }

    /// Bulk double-buffered endpoint? (EP_KIND + EP_TYPE bulk 00; the DTOG
    /// bit then ping-pongs between DESC0 and DESC1. Isochronous stays
    /// treated-as-bulk, single.)
    fn is_double_buffered(&self, n: usize) -> bool {
        let r = self.ep[n] as u32;
        r & EP_KIND != 0 && r & EP_TYPE_MASK == 0
    }

    /// Isochronous endpoint? (EP_TYPE = 10: no STALL, CTR pends HP.)
    fn is_iso(&self, n: usize) -> bool {
        ep_type(self.ep[n] as u32) == TYPE_ISO
    }

    /// Bus traffic while suspended wakes the device (WKUP) unless firmware
    /// holds FSUSP (forced suspend: firmware owns the state, only the
    /// RESUME pulse or clearing FSUSP wakes).
    fn traffic(&mut self, sys: &System) {
        if self.suspended && self.cntr & CNTR_FSUSP != 0 {
            // FSUSP held: stay suspended (firmware owns the state).
            return;
        }
        if self.suspended {
            self.suspended = false;
            self.istr &= !(ISTR_SUSP as u16);
            self.istr |= ISTR_WKUP as u16;
            self.irq_wkup(sys);
        }
    }

    /// ISTR status read: latched event flags (RESET, …) plus the derived
    /// CTR/DIR/EP_ID nibble — like real hardware, CTR is the OR over every
    /// endpoint's CTR_TX/RX with the lowest numbered endpoint winning, so it
    /// retires automatically when firmware clears the endpoint flags.
    fn istr_read(&self) -> u32 {
        let mut v = self.istr as u32;
        for (n, ep) in self.ep.iter().enumerate() {
            if ep & (CTR_TX as u16 | CTR_RX as u16) != 0 {
                v |= ISTR_CTR;
                if ep & CTR_TX as u16 == 0 {
                    v |= ISTR_DIR; // RX/SETUP direction
                }
                v |= n as u32 & 0xF;
                break;
            }
        }
        v
    }

    /// USB reset state (FRES asserted, or FRES 1->0 release which additionally
    /// raises the RESET event, kicking firmware enumeration). A bus reset
    /// implies reattach: detach/suspend clear (reset signaling wakes).
    fn usb_reset(&mut self, sys: &System, with_event: bool) {
        self.ep = [0; 8];
        self.daddr = 0;
        self.istr = 0;
        self.detached = false;
        self.suspended = false;
        if with_event {
            self.istr |= 1 << 10; // RESET
            self.irq(sys, CNTR_RESETM);
        }
    }

    /// Host disconnect (pull-up off): no tokens arrive, IN never completes,
    /// SOF freezes. Cleared by the next bus reset (reattach).
    pub fn detach(&mut self, sys: &System) {
        self.detached = true;
        self.suspended = true;
        self.istr |= ISTR_SUSP as u16;
        self.irq(sys, CNTR_SUSPM);
    }

    /// Device->host IN completion for endpoint n (called on a 0/1/2->VALID
    /// STAT_TX transition with CTR_TX clear): move COUNT_TX bytes from the
    /// PMA TX buffer into a UsbIn event, then apply the hardware
    /// post-conditions (CTR_TX set, STAT_TX back to NAK, DTOG_TX toggled).
    /// Single-buffered endpoints send via DESC0; double-buffered bulk
    /// endpoints ping-pong between DESC0 (DTOG_TX = 0) and DESC1 (DTOG_TX = 1).
    fn complete_in(&mut self, sys: &System, n: usize) {
        let second = self.is_double_buffered(n) && self.ep[n] & DTOG_TX as u16 != 0;
        let blk = self.desc_pair(n, second);
        let tx_word = self.pma_half(blk) as usize;
        let count = (self.pma_half(blk + 4) & 0x3FF) as usize;
        let mut data = Vec::new();
        for i in 0..count {
            let off = tx_word * 2 + (i >> 1) * 4 + (i & 1);
            if off >= PMA_BYTES {
                break;
            }
            data.push(self.data_byte(tx_word, i));
        }
        sys.push_event(VmEvent::UsbIn { ep: n as u8, data });
        self.ep[n] |= CTR_TX as u16;
        self.ep[n] = (self.ep[n] & !(STAT_TX_MASK as u16)) | ((STAT_NAK << 4) as u16);
        self.ep[n] ^= DTOG_TX as u16;
        // ISTR CTR/DIR/EP_ID derive from the endpoint flags on read.
        self.ctr_irq(sys, n);
        self.traffic(sys);
    }

    /// Host->device OUT/SETUP delivery (called by usb_inject_*): stage bytes
    /// into the PMA RX buffer when the endpoint is armed (STAT_RX VALID),
    /// else NAK (return false). Applies DTOG_RX toggle, CTR_RX, ISTR.
    /// Single-buffered endpoints receive via DESC1; double-buffered bulk
    /// endpoints ping-pong between DESC0 (DTOG_RX = 0) and DESC1 (DTOG_RX = 1)
    /// and stay VALID across the first fill (firmware drains at its own pace).
    fn deliver_rx(
        &mut self,
        sys: &System,
        ep: usize,
        data: &[u8],
        is_setup: bool,
        addr: Option<u8>,
    ) -> bool {
        if ep >= 8 || (is_setup && ep != 0) {
            return false;
        }
        // Powered-down macro or detached bus: the PHY sees nothing.
        if self.cntr & CNTR_PDWN != 0 || self.detached {
            return false;
        }
        // Hardware address filter (host must address the device; the
        // scripted-host default of None accepts, like a correctly
        // addressed bus).
        if let Some(a) = addr {
            let ef = self.daddr & 0x80 != 0;
            let dev = self.daddr & 0x7F;
            if (!ef && a != 0) || (ef && a != dev) {
                return false;
            }
        }
        // Like silicon, a SETUP transaction is ACKed even while STAT_RX is
        // NAK (the stack only re-arms RX for OUT data/status stages, never
        // after a status-IN transfer); plain OUT packets still need VALID.
        if !is_setup && stat_rx(self.ep[ep] as u32) != STAT_VALID {
            return false; // not armed: NAK.
        }
        let db = self.is_double_buffered(ep);
        let second = if db {
            self.ep[ep] & DTOG_RX as u16 != 0
        } else {
            true
        };
        let blk = self.desc_pair(ep, second);
        let rx_word = self.pma_half(blk) as usize;
        if !self.data_fits(rx_word, data.len()) {
            return false;
        }
        for (i, b) in data.iter().enumerate() {
            self.data_set_byte(rx_word, i, *b);
        }
        // COUNT_RX: preserve the firmware's block-size config (bits 15:10),
        // report the received count in bits 9:0.
        let cnt_off = blk + 4;
        let cfg = self.pma_half(cnt_off) & 0xFC00;
        self.pma_set_half(cnt_off, cfg | ((data.len() & 0x3FF) as u16));
        let mut r = self.ep[ep] as u32;
        if is_setup {
            r |= SETUP_BIT;
        }
        if !db {
            r = (r & !STAT_RX_MASK) | (STAT_NAK << 12); // HW NAKs after reception
        }
        r |= CTR_RX;
        r ^= DTOG_RX; // DATA0/DATA1 sequencing
        self.ep[ep] = r as u16;
        // ISTR CTR/DIR/EP_ID derive from the endpoint flags on read.
        self.ctr_irq(sys, ep);
        self.traffic(sys);
        true
    }

    /// Host-side injection entry point (SETUP only on EP0). `addr` selects
    /// hardware address filtering (None = correctly-addressed host).
    pub fn inject(
        &mut self,
        sys: &System,
        ep: usize,
        data: &[u8],
        is_setup: bool,
        addr: Option<u8>,
    ) -> bool {
        self.deliver_rx(sys, ep, data, is_setup, addr)
    }

    fn write_ep(&mut self, sys: &System, n: usize, v: u16) {
        let cur = self.ep[n] as u32;
        let v = v as u32;
        let mut r = cur;
        // CTR flags: writing 0 clears (writing 1: no effect). Clearing
        // CTR_RX also retires the SETUP marker for that transaction.
        if v & CTR_RX == 0 {
            r &= !CTR_RX;
            if r & CTR_RX == 0 {
                r &= !SETUP_BIT;
            }
        }
        if v & CTR_TX == 0 {
            r &= !CTR_TX;
        }
        // STAT fields: writing 1 toggles each bit. Isochronous endpoints
        // cannot stall: drop toggle bits that would land in STALL.
        let mut tm = v & (STAT_TX_MASK | STAT_RX_MASK);
        if self.is_iso(n) {
            let after = cur ^ tm;
            if stat_tx(after) == 1 {
                tm &= !STAT_TX_MASK;
            }
            if stat_rx(after) == 1 {
                tm &= !STAT_RX_MASK;
            }
        }
        r ^= tm;
        // Direct fields: endpoint address, kind, type.
        r = (r & !(EA_MASK | EP_KIND | EP_TYPE_MASK)) | (v & (EA_MASK | EP_KIND | EP_TYPE_MASK));
        // DTOG bits are read-only (toggled by hardware paths above).
        self.ep[n] = r as u16;
        // IN completion on a ->VALID STAT_TX transition with CTR_TX clear
        // (and a host on the bus to ACK it).
        if stat_tx(r) == STAT_VALID && stat_tx(cur) != STAT_VALID && (r & CTR_TX) == 0 {
            if self.cntr & CNTR_PDWN == 0 && !self.detached {
                self.complete_in(sys, n);
            }
        }
    }

    fn read_reg(&mut self, offset: u32) -> u32 {
        match offset {
            0x00..=0x1C if offset % 4 == 0 => self.ep[(offset / 4) as usize] as u32,
            0x40 => self.cntr as u32,
            0x44 => self.istr_read(),
            // FNR: frame number + RXDP (D+ line: 1 while attached).
            // LSOF/LCK read 0: the host never misses in emulation (no ESOF
            // generation while attached; detached freezes SOF outright).
            0x48 => {
                (self.frame as u32)
                    | (if self.detached { 0 } else { 1 << 15 })
            }
            0x4C => self.daddr as u32,
            0x50 => self.btable as u32,
            _ => 0,
        }
    }

    fn write_reg(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00..=0x1C if offset % 4 == 0 => {
                let v = (value & 0xFFFF) as u16;
                self.write_ep(sys, (offset / 4) as usize, v);
            }
            0x40 => {
                let fresh = value as u16 & 0xFF1F;
                let was_fsusp = self.cntr & CNTR_FSUSP != 0;
                self.cntr = fresh;
                // A latched RESET with newly-enabled RESETM pends like
                // silicon's level-sensitive line: covers a bus reset that
                // landed before the firmware armed its masks (the event
                // would otherwise be lost forever).
                if self.istr & (1 << 10) != 0 {
                    self.irq(sys, CNTR_RESETM);
                }
                if fresh & 1 != 0 {
                    // FRES asserted: hold the USB logic in reset (no event).
                    self.ep = [0; 8];
                    self.istr = 0;
                    self.daddr = 0;
                }
                // NOTE: FRES release is NOT a bus reset (ISTR RESET means SE0
                // on the wire, sent only by the host via usb_reset): the
                // endpoints stay closed until a real reset arrives. A latched
                // RESET re-pends below once the firmware arms its masks.
                // FSUSP set: force suspend now (SUSP + IRQ via SUSPM).
                // Re-asserted on every rising edge, even if a cleared
                // SUSP flag hides an already-suspended state.
                if fresh & CNTR_FSUSP != 0 && !was_fsusp {
                    self.suspended = true;
                    self.istr |= ISTR_SUSP as u16;
                    self.irq(sys, CNTR_SUSPM);
                }
                // FSUSP cleared while suspended: wake (WKUP + IRQ42).
                if fresh & CNTR_FSUSP == 0 && was_fsusp && self.suspended {
                    self.suspended = false;
                    self.istr &= !(ISTR_SUSP as u16);
                    self.istr |= ISTR_WKUP as u16;
                    self.sof_acc = 0;
                    self.irq_wkup(sys);
                }
                // RESUME pulse (device-initiated remote wakeup): completes
                // after one frame with the wake sequence (also when not
                // suspended; the pulse still self-clears).
                if fresh & (1 << 4) != 0 {
                    self.resume_at = crate::system::instruction_count() + SOF_PERIOD;
                }
            }
            0x44 => {
                // Latched event flags clear on 0-write; CTR/DIR/EP_ID are
                // derived status (see istr_read) and ignore writes.
                self.istr &= value as u16;
            }
            0x4C => self.daddr = (value & 0xFF) as u8,
            0x50 => self.btable = (value & 0xFFF8) as u16,
            _ => {}
        }
    }
}

impl Peripheral for Usb {
    fn tick(&mut self, sys: &System) {
        self.tick_usb(sys);
    }
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        self.read_reg(offset)
    }
    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        self.write_reg(sys, offset, value)
    }
    fn read_sized(&mut self, _sys: &System, offset: u32, size: u8) -> u32 {
        if (0x400..0x800).contains(&offset) {
            // Packet memory: byte-exact regardless of access width (the bus
            // lane logic is bypassed for this window — see is_register).
            // Offset is slot-relative; PMA starts at relative 0x400.
            let base = offset as usize - 0x400;
            let mut v = 0u32;
            for i in 0..size.min(4) as usize {
                v |= (self.pma_byte(base + i) as u32) << (i * 8);
            }
            return v;
        }
        let w = self.read_reg(offset & !3);
        // 16-bit endpoint/control registers: take the addressed half.
        if size <= 2 {
            (w >> ((offset % 4) * 8)) & 0xFFFF
        } else {
            w
        }
    }
    fn write_sized(&mut self, sys: &System, offset: u32, size: u8, value: u32) {
        if (0x400..0x800).contains(&offset) {
            let base = offset as usize - 0x400;
            for i in 0..size.min(4) as usize {
                self.pma_set_byte(base + i, ((value >> (i * 8)) & 0xFF) as u8);
            }
            return;
        }
        // Registers are 16-bit: the bus pre-merged odd-lane stores, so the
        // low halfword always carries the access.
        self.write_reg(sys, offset & !3, value & 0xFFFF);
    }

    fn usb_inject(
        &mut self,
        sys: &System,
        ep: usize,
        data: &[u8],
        is_setup: bool,
        addr: Option<u8>,
    ) -> bool {
        self.inject(sys, ep, data, is_setup, addr)
    }

    fn usb_bus_reset(&mut self, sys: &System) -> bool {
        if self.cntr & CNTR_PDWN != 0 {
            return false; // powered-down macro sees no bus.
        }
        self.usb_reset(sys, true);
        true
    }

    fn usb_detach(&mut self, sys: &System) -> bool {
        self.detach(sys);
        true
    }
}
