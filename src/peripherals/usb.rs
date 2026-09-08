//! USB full-speed device (STM32F103 @ 0x4000_5C00, IRQs 19/20, packet memory @
//! 0x4000_6000). Synchronous transaction model: endpoint events complete when
//! the firmware arms them, which is exact for control/bulk/interrupt firmware.
//!
//! Implemented: EP0R-EP7R with real toggle semantics (STAT_TX/RX toggle on
//! 1-write, CTR_TX/RX clear on 0-write, DTOG read-only), CNTR masks, ISTR
//! (event flags write-0-clear; CTR/DIR/EP_ID derived from endpoint flags like
//! hardware), DADDR, BTABLE, 512 B packet memory with byte-exact sub-word access, USB RESET event on
//! FRES release, SETUP/OUT injection (host -> device) with DTOG sequencing,
//! IN completion (device -> host) drained as `VmEvent::UsbIn`, CTR/RESET IRQs
//! on the low-priority vector (IRQ 20; isochronous endpoints are treated as
//! bulk), SOF engine (1ms frames off the instruction counter, FNR + SOF IRQ),
//! suspend/resume with wakeup IRQ 42 (FSUSP force, 3-idle-frame auto-suspend,
//! resume on traffic or RESUME pulse), double-buffered bulk endpoints.
//!
//! Deliberately absent: ESOF generation (host never misses in emulation),
//! isochronous CTR, PDWN gating (stored only), remote-wakeup electricals
//! beyond the WKUP flag.

use crate::system::{System, VmEvent};
use super::Peripheral;

pub const USB_BASE: u32 = 0x4000_5C00;
/// End (exclusive) of the USB window: registers + 512 B packet memory.
pub const USB_END: u32 = 0x4000_6400;
const PMA_BYTES: usize = 512;

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
/// USB wakeup vector (WKUP event only).
pub const USB_WKUP_IRQ: i32 = 42;
/// Instructions per USB frame (1ms @ 72MHz): the SOF engine's tick.
const SOF_PERIOD: u64 = 72_000;
/// Idle frames (no transfers) before the device suspends, like 3ms of J.
const SUSPEND_IDLE_FRAMES: u32 = 3;

fn stat_tx(r: u32) -> u32 { (r >> 4) & 3 }
fn stat_rx(r: u32) -> u32 { (r >> 12) & 3 }

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
    /// Suspend state: set by FSUSP or 3 idle frames, cleared on resume.
    suspended: bool,
    idle_frames: u32,
    traffic_since_sof: bool,
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
                idle_frames: 0,
                traffic_since_sof: false,
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

    /// Pend the low-priority USB IRQ when its CNTR mask bit is set.
    fn irq(&mut self, sys: &System, mask_bit: u32) {
        if self.cntr as u32 & mask_bit != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(USB_LP_IRQ);
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
    /// suspended (host silent: SOF freezes too). Three frames without any
    /// transfer auto-suspend (SUSP + IRQ via SUSPM); any transfer resumes
    /// (WKUP + IRQ42). A device RESUME pulse (CNTR.4) clears after a frame
    /// with the same wake sequence.
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
                self.idle_frames = 0;
                self.irq_wkup(sys);
            }
            return;
        }
        self.sof_acc += delta;
        while self.sof_acc >= SOF_PERIOD {
            self.sof_acc -= SOF_PERIOD;
            self.frame = (self.frame + 1) & 0x7FF;
            self.istr |= ISTR_SOF as u16;
            self.irq(sys, CNTR_SOFM);
            if self.traffic_since_sof {
                self.traffic_since_sof = false;
                self.idle_frames = 0;
            } else {
                self.idle_frames += 1;
                if self.idle_frames >= SUSPEND_IDLE_FRAMES {
                    self.enter_suspend(sys);
                }
            }
            if self.resume_at != 0 && now >= self.resume_at {
                self.resume_at = 0;
                self.cntr &= !(1 << 4);
                self.istr |= ISTR_WKUP as u16;
                self.irq_wkup(sys);
            }
        }
    }

    fn enter_suspend(&mut self, sys: &System) {
        self.suspended = true;
        self.istr |= ISTR_SUSP as u16;
        self.irq(sys, CNTR_SUSPM);
    }

    /// Bulk double-buffered endpoint? (EP_KIND + EP_TYPE bulk 00; the DTOG
    /// bit then selects between the TX-descriptor and RX-descriptor blocks
    /// as the two buffers. Isochronous stays treated-as-bulk, single.)
    fn is_double_buffered(&self, n: usize) -> bool {
        let r = self.ep[n] as u32;
        r & EP_KIND != 0 && r & EP_TYPE_MASK == 0
    }

    /// Any bus transfer marks traffic (resets the suspend-idle count).
    /// Resume-from-suspend also raises WKUP here (host traffic resumes).
    fn traffic(&mut self, sys: &System) {
        self.traffic_since_sof = true;
        self.idle_frames = 0;
        if self.suspended && self.cntr & (1 << 1) != 0 {
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
    /// raises the RESET event, kicking firmware enumeration).
    fn usb_reset(&mut self, sys: &System, with_event: bool) {
        self.ep = [0; 8];
        self.daddr = 0;
        self.istr = 0;
        if with_event {
            self.istr |= 1 << 10; // RESET
            self.irq(sys, CNTR_RESETM);
        }
    }

    /// Device->host IN completion for endpoint n (called on a 0/1/2->VALID
    /// STAT_TX transition with CTR_TX clear): move COUNT_TX bytes from the
    /// PMA TX buffer into a UsbIn event, then apply the hardware
    /// post-conditions (CTR_TX set, STAT_TX back to NAK, DTOG_TX toggled).
    /// Double-buffered bulk endpoints use the DTOG_TX-selected descriptor
    /// block (TX desc when DTOG=0, RX desc repurposed when DTOG=1).
    fn complete_in(&mut self, sys: &System, n: usize) {
        let base = (self.btable as usize) & 0x1F8;
        let blk = if self.is_double_buffered(n) && self.ep[n] & DTOG_TX as u16 != 0 {
            base + n * 8 + 4
        } else {
            base + n * 8
        };
        let tx_addr = self.pma_half(blk) as usize;
        let count = (self.pma_half(blk + 2) & 0x3FF) as usize;
        let mut data = vec![0u8; count.min(PMA_BYTES)];
        for (i, b) in data.iter_mut().enumerate() {
            *b = self.pma_byte(tx_addr + i);
        }
        sys.push_event(VmEvent::UsbIn { ep: n as u8, data });
        self.ep[n] |= CTR_TX as u16;
        self.ep[n] = (self.ep[n] & !(STAT_TX_MASK as u16)) | ((STAT_NAK << 4) as u16);
        self.ep[n] ^= DTOG_TX as u16;
        // ISTR CTR/DIR/EP_ID derive from the endpoint flags on read.
        self.irq(sys, CNTR_CTRM);
        self.traffic(sys);
    }

    /// Host->device OUT/SETUP delivery (called by usb_inject_*): stage bytes
    /// into the PMA RX buffer when the endpoint is armed (STAT_RX VALID),
    /// else NAK (return false). Applies DTOG_RX toggle, CTR_RX, ISTR.
    /// Double-buffered bulk endpoints use the DTOG_RX-selected block (RX
    /// desc when DTOG=0, TX desc repurposed when DTOG=1) and stay VALID
    /// across the first fill (firmware drains at its own pace).
    fn deliver_rx(&mut self, sys: &System, ep: usize, data: &[u8], is_setup: bool) -> bool {
        if ep >= 8 || (is_setup && ep != 0) {
            return false;
        }
        if stat_rx(self.ep[ep] as u32) != STAT_VALID {
            return false; // not armed: NAK.
        }
        let base = (self.btable as usize) & 0x1F8;
        let db = self.is_double_buffered(ep);
        let blk = if db && self.ep[ep] & DTOG_RX as u16 != 0 {
            base + ep * 8
        } else {
            base + ep * 8 + 4
        };
        let rx_addr = self.pma_half(blk) as usize;
        if rx_addr + data.len() > PMA_BYTES {
            return false;
        }
        for (i, b) in data.iter().enumerate() {
            self.pma_set_byte(rx_addr + i, *b);
        }
        // COUNT_RX: preserve the firmware's block-size config (bits 15:10),
        // report the received count in bits 9:0.
        let cnt_off = blk + 2;
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
        self.irq(sys, CNTR_CTRM);
        self.traffic(sys);
        true
    }

    /// Host-side injection entry point (SETUP only on EP0).
    pub fn inject(&mut self, sys: &System, ep: usize, data: &[u8], is_setup: bool) -> bool {
        self.deliver_rx(sys, ep, data, is_setup)
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
        // STAT fields: writing 1 toggles each bit.
        r ^= v & (STAT_TX_MASK | STAT_RX_MASK);
        // Direct fields: endpoint address, kind, type.
        r = (r & !(EA_MASK | EP_KIND | EP_TYPE_MASK)) | (v & (EA_MASK | EP_KIND | EP_TYPE_MASK));
        // DTOG bits are read-only (toggled by hardware paths above).
        self.ep[n] = r as u16;
        // IN completion on a ->VALID STAT_TX transition with CTR_TX clear.
        if stat_tx(r) == STAT_VALID && stat_tx(cur) != STAT_VALID && (r & CTR_TX) == 0 {
            self.complete_in(sys, n);
        }
    }

    fn read_reg(&mut self, offset: u32) -> u32 {
        match offset {
            0x00..=0x1C if offset % 4 == 0 => self.ep[(offset / 4) as usize] as u32,
            0x40 => self.cntr as u32,
            0x44 => self.istr_read(),
            // FNR: frame number + RXDP (attached). LSOF/LCK read 0: the
            // host never misses in emulation (no ESOF generation).
            0x48 => (self.frame as u32) | (1 << 15),
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
                let was_fres = self.cntr & 1 != 0;
                let was_fsusp = self.cntr & (1 << 1) != 0;
                self.cntr = fresh;
                if fresh & 1 != 0 {
                    // FRES asserted: hold the USB logic in reset (no event).
                    self.ep = [0; 8];
                    self.istr = 0;
                    self.daddr = 0;
                } else if was_fres {
                    // FRES release: attach event; firmware enumeration starts.
                    self.usb_reset(sys, true);
                }
                // FSUSP set: force suspend now (SUSP + IRQ via SUSPM).
                // Re-asserted on every rising edge, even if a cleared
                // SUSP flag hides an already-suspended state.
                if fresh & (1 << 1) != 0 && !was_fsusp {
                    self.suspended = true;
                    self.istr |= ISTR_SUSP as u16;
                    self.irq(sys, CNTR_SUSPM);
                }
                // FSUSP cleared while suspended: wake (WKUP + IRQ42).
                if fresh & (1 << 1) == 0 && was_fsusp && self.suspended {
                    self.suspended = false;
                    self.istr &= !(ISTR_SUSP as u16);
                    self.istr |= ISTR_WKUP as u16;
                    self.sof_acc = 0;
                    self.idle_frames = 0;
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
        if (0x400..0x600).contains(&offset) {
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
        if (0x400..0x600).contains(&offset) {
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

    fn usb_inject(&mut self, sys: &System, ep: usize, data: &[u8], is_setup: bool) -> bool {
        self.inject(sys, ep, data, is_setup)
    }
}
