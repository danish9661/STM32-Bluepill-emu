use crate::system::System;
use super::Peripheral;

#[derive(Clone, Copy)]
struct Mailbox {
    tir: u32, tdtr: u32, tdlr: u32, tdhr: u32,
}

/// Split a CAN mailbox (TIR/TDTR/TDLR/TDHR) into (id, len, data[8]).
/// id is the 11-bit STDID or 29-bit EXTID per the IDE bit.
fn msg_fields(tir: u32, tdtr: u32, tdlr: u32, tdhr: u32) -> (u32, u8, [u8; 8]) {
    let ide = (tir >> 2) & 1;
    let id = if ide != 0 { (tir >> 3) & 0x1FFF_FFFF } else { (tir >> 21) & 0x7FF };
    let len = (tdtr & 0xF) as u8;
    let mut data = [0u8; 8];
    data[0] = (tdlr & 0xFF) as u8;
    data[1] = ((tdlr >> 8) & 0xFF) as u8;
    data[2] = ((tdlr >> 16) & 0xFF) as u8;
    data[3] = ((tdlr >> 24) & 0xFF) as u8;
    data[4] = (tdhr & 0xFF) as u8;
    data[5] = ((tdhr >> 8) & 0xFF) as u8;
    data[6] = ((tdhr >> 16) & 0xFF) as u8;
    data[7] = ((tdhr >> 24) & 0xFF) as u8;
    (id, len, data)
}

pub struct Can {
    mcr: u32, msr: u32, tsr: u32, rf0r: u32, rf1r: u32,
    ier: u32, esr: u32, btr: u32,
    tx: [Mailbox; 3],
    rx: [Mailbox; 2],
    fmr: u32, fm1r: u32, fs1r: u32, ffa1r: u32, fa1r: u32,
    filter: [u32; 56],
    irq_base: i32,
}

impl Can {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        let irq_base = match name {
            "CAN1" => Some(19),
            "CAN2" => Some(63),
            _ => None,
        }?;
        Some(Box::new(Can {
            mcr: 0x0001_0002, msr: 0x0000_0C02, tsr: 0x1C00_0000,
            irq_base,
            ..Self::default()
        }))
    }

    fn can_num(&self) -> u8 { if self.irq_base == 19 { 1 } else { 2 } }

    /// CAN2 start bank from FMR CAN2SB[13:8], clamped to 1..27 (RM0008
    /// reserves CAN2SB=0; bare FMR=0 writes — the common HAL RMW exit —
    /// keep bank 0 with CAN1 instead of orphaning it, lenient + documented).
    /// Reset value 14 splits 0..13 to CAN1, 14..27 to CAN2.
    fn split(&self) -> u32 { ((self.fmr >> 8) & 0x3F).clamp(1, 27) }

    /// Time-triggered stamp: low 16 bits of the instruction counter stand
    /// in for the CAN bit-time counter (monotonic, deterministic; full
    /// TTCM sync/calibration frames are out of scope).
    fn time_stamp() -> u32 {
        (crate::system::instruction_count() & 0xFFFF) as u32
    }

    /// Inject a received message into the CAN peripheral, matching filters.
    /// Returns true if the message was accepted into a FIFO.
    pub fn inject_message(&mut self, sys: &System, tir: u32, tdtr: u32, tdlr: u32, tdhr: u32) -> bool {
        let fifo = self.match_filter(sys, tir);
        if let Some(fifo_idx) = fifo {
            let (rfxr, fifo) = if fifo_idx == 0 {
                (&mut self.rf0r, &mut self.rx[0])
            } else {
                (&mut self.rf1r, &mut self.rx[1])
            };
            let fmp = *rfxr & 0x3;
            if fmp < 2 {
                fifo.tir = tir;
                // TTCM: stamp arrival time into RDTxR TIME[31:16].
                fifo.tdtr = if self.mcr & (1 << 7) != 0 {
                    (tdtr & 0xFFFF) | (Self::time_stamp() << 16)
                } else {
                    tdtr
                };
                fifo.tdlr = tdlr;
                fifo.tdhr = tdhr;
                *rfxr = (*rfxr & !0x3) | (fmp + 1);
                let (id, len, data) = msg_fields(tir, tdtr, tdlr, tdhr);
                sys.push_event(crate::system::VmEvent::CanRx { can: self.can_num(), id, len, data });
                self.fire_interrupts(sys);
                true
            } else {
                *rfxr |= 1 << 4; // FOVR
                true
            }
        } else {
            false
        }
    }

    fn match_filter(&self, sys: &System, tir: u32) -> Option<usize> {
        if self.can_num() == 2 {
            // Silicon layout: filter banks live ONLY in CAN1 (28 banks);
            // CAN2 borrows banks [CAN2SB..28] and has no filter registers
            // of its own. Delegate the match to CAN1's bank.
            return sys.p.can_match_can2(tir);
        }
        self.match_for(tir, false)
    }

    /// Match `tir` against this bank over `[lo..hi)`. Pure on registers
    /// (shared by CAN1's own match and CAN2's delegated match).
    fn match_for(&self, tir: u32, for_can2: bool) -> Option<usize> {
        let split = self.split();
        let (lo, hi) = if for_can2 { (split, 28) } else { (0, split) };
        if self.fmr & 1 != 0 { return None; } // FINIT=1: init mode, no matching
        let _ide = (tir >> 2) & 1; // 0=standard, 1=extended
        let mut best = None;
        for bank in lo..hi {
            let b = bank as usize;
            let enabled = (self.fa1r >> bank) & 1;
            if enabled == 0 { continue; }
            let scale = (self.fm1r >> bank) & 1; // 0=16-bit x 2, 1=32-bit
            let mode = (self.fs1r >> bank) & 1; // 0=ID mask, 1=ID list
            let identifier = tir >> 21; // STDID[10:0] for standard, EXTID[28:0] for extended
            let f0 = self.filter[b * 2];
            let f1 = self.filter[b * 2 + 1];
            let matched = if scale == 0 && mode == 0 {
                    let id1 = f0 >> 16; let mask1 = f0 & 0xFFFF;
                    let id2 = f1 >> 16; let mask2 = f1 & 0xFFFF;
                    let id = identifier & 0x7FF;
                    ((id & mask1) == (id1 & mask1)) || ((id & mask2) == (id2 & mask2))
                } else if scale == 0 && mode == 1 {
                    let id1 = f0 >> 16; let id2 = f0 & 0xFFFF;
                    let id3 = f1 >> 16; let id4 = f1 & 0xFFFF;
                    let id = identifier & 0x7FF;
                    id == id1 || id == id2 || id == id3 || id == id4
                } else if scale == 1 && mode == 0 {
                    let mask = f1; let id = f0;
                    (identifier & mask) == (id & mask)
                } else {
                    identifier == f0
                };
            if matched {
                best = Some((self.ffa1r >> bank) as usize & 1);
            }
        }
        best
    }

    fn fire_interrupts(&mut self, sys: &System) {
        let base = self.irq_base;
        // NOTE: no TX arm here on purpose. The old code pended TX on the
        // LEVEL of TSR bits 24-26 — but those are CODE[1:0]/TME0, not RQCP
        // (TME0 is set at reset!), so any TMEIE-enabled run stormed IRQ37
        // on every later CAN event write. Real HW pends on the RQCP 0->1
        // edge (and on TMEIE rising with RQCP already set); both edges pend
        // explicitly at their sites below, exactly once each.
        // RX0 (FMPIE0 bit 1, FFIE0 bit 2, FOVIE0 bit 3)
        if self.ier & 0x0E != 0 && self.rf0r & 0x03 != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(base + 1);
        }
        // RX1 (FMPIE1 bit 4, FFIE1 bit 5, FOVIE1 bit 6)
        if self.ier & 0x70 != 0 && self.rf1r & 0x03 != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(base + 2);
        }
        // SCE (EWGIE bit 7, EPVIE bit 8, BOFIE bit 9, LECIE bit 10, ERRIE bit 11)
        if self.ier & 0xF80 != 0 && self.esr != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(base + 3);
        }
    }
}

impl Default for Can {
    fn default() -> Self {
        Self {
            mcr: 0, msr: 0, tsr: 0, rf0r: 0, rf1r: 0, ier: 0, esr: 0, btr: 0,
            tx: [Mailbox { tir: 0, tdtr: 0, tdlr: 0, tdhr: 0 }; 3],
            rx: [Mailbox { tir: 0, tdtr: 0, tdlr: 0, tdhr: 0 }; 2],
            fmr: 0x2A1C_0E01, fm1r: 0, fs1r: 0xFFFF_FFFF, ffa1r: 0, fa1r: 0,
            filter: [0; 56],
            irq_base: 0,
        }
    }
}

impl Peripheral for Can {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x000 => self.mcr,
            0x004 => self.msr,
            0x008 => self.tsr,
            0x00C => self.rf0r,
            0x010 => self.rf1r,
            0x014 => self.ier,
            0x018 => self.esr,
            0x01C => self.btr,
            0x180..=0x1AC => {
                let i = ((offset - 0x180) / 0x10) as usize;
                if i >= 3 { return 0; }
                match (offset - 0x180) % 0x10 {
                    0x00 => self.tx[i].tir,
                    0x04 => self.tx[i].tdtr,
                    0x08 => self.tx[i].tdlr,
                    0x0C => self.tx[i].tdhr,
                    _ => 0,
                }
            }
            0x1B0..=0x1CC => {
                let i = ((offset - 0x1B0) / 0x10) as usize;
                if i >= 2 { return 0; }
                let val = match (offset - 0x1B0) % 0x10 {
                    0x00 => self.rx[i].tir,
                    0x04 => self.rx[i].tdtr,
                    0x08 => self.rx[i].tdlr,
                    0x0C => self.rx[i].tdhr,
                    _ => 0,
                };
                // Decrement FMP on read of first RX mailbox register
                if (offset - 0x1B0) % 0x10 == 0 && i == 0 && self.rf0r & 0x3 != 0 {
                    self.rf0r = (self.rf0r & !0x3) | ((self.rf0r & 0x3) - 1);
                    self.rf0r |= 1 << 3; // RFOM flag
                }
                val
            }
            0x200 => {
                // CAN2 owns no filter registers on silicon (all 28 banks
                // live in CAN1, configured through CAN1's window).
                if self.can_num() == 2 { return 0; }
                self.fmr
            }
            0x204 => {
                if self.can_num() == 2 { return 0; }
                self.fm1r
            }
            0x20C => {
                if self.can_num() == 2 { return 0; }
                self.fs1r
            }
            0x214 => {
                if self.can_num() == 2 { return 0; }
                self.ffa1r
            }
            0x21C => {
                if self.can_num() == 2 { return 0; }
                self.fa1r
            }
            0x240..=0x31C => {
                // CAN2 owns no filter registers on silicon (all 28 banks
                // live in CAN1, configured through CAN1's window) — reads
                // return 0.
                if self.can_num() == 2 { return 0; }
                let i = ((offset - 0x240) / 4) as usize;
                self.filter.get(i).copied().unwrap_or(0)
            }
            _ => 0,
        }
    }

    fn can_inject_message(&mut self, sys: &System, tir: u32, tdtr: u32, tdlr: u32, tdhr: u32) -> bool {
        self.inject_message(sys, tir, tdtr, tdlr, tdhr)
    }
    /// CAN2's delegated filter match against CAN1's shared bank
    /// (banks [CAN2SB..28]). Default: no match (CAN1 overrides).
    fn can_match_for(&self, tir: u32, for_can2: bool) -> Option<usize> {
        self.match_for(tir, for_can2)
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x000 => {
                // F1 CAN_MCR writeable bits: INRQ..TTCM (0-7), RESET (15), DBF (16)
                let mask = 0x0001_80FF;
                self.mcr = (self.mcr & !mask) | (value & mask);
                let inrq = value & 1;
                let sleep = (value >> 1) & 1;
                if inrq != 0 {
                    self.msr |= 1; self.msr &= !2;
                } else {
                    self.msr &= !1; self.msr |= 2;
                }
                if sleep != 0 { self.msr |= 2; }
                else if inrq == 0 { self.msr &= !2; }
            }
            0x004 => self.msr = (self.msr & !0x0C0B) | (value & 0x0C0B),
            0x008 => self.tsr &= !(value & 0x0007_0707),
            0x00C => {
                self.rf0r = (self.rf0r & 0xFFFF_0000) | (value & 0x3F);
                self.fire_interrupts(sys);
            }
            0x010 => {
                self.rf1r = (self.rf1r & 0xFFFF_0000) | (value & 0x3F);
                self.fire_interrupts(sys);
            }
            0x014 => {
                // TMEIE 0->1 with a completion already latched (TSR TXOK-ish
                // bits — the firmware-visible completion flags) pends once,
                // like silicon; later event writes must NOT re-pend (level
                // storm). Completion clears via the TSR W1C arm below.
                let tme_rising = (value & !self.ier) & 0x01 != 0;
                self.ier = value & 0x7FF;
                if tme_rising && self.tsr & 0x0007_0707 != 0 {
                    sys.p.nvic.borrow_mut().set_intr_pending(self.irq_base);
                }
                self.fire_interrupts(sys);
            }
            0x01C => self.btr = value, // incl. SILM/LBKM (reserved bits stored, as before)
            0x180..=0x1AC => {
                let i = ((offset - 0x180) / 0x10) as usize;
                if i >= 3 { return; }
                match (offset - 0x180) % 0x10 {
                    0x00 => self.tx[i].tir = value,
                    0x04 => self.tx[i].tdtr = value,
                    0x08 => self.tx[i].tdlr = value,
                    0x0C => self.tx[i].tdhr = value,
                    _ => {}
                }
                if (offset - 0x180) % 0x10 == 0 && value & 1 != 0 {
                    self.tsr |= 1 << i;
                    self.tsr |= 0x2 << (8 + i * 4);
                    let rqcp = (self.tsr >> 26) & 7;
                    self.tsr = (self.tsr & !(7 << 26)) | ((rqcp & !(1 << i)) << 26);
                    self.tsr |= 1 << (16 + i);
                    // TTCM: capture transmit time into TDTxR TIME[31:16].
                    if self.mcr & (1 << 7) != 0 {
                        let t = self.tx[i].tdtr;
                        self.tx[i].tdtr = (t & 0xFFFF) | (Self::time_stamp() << 16);
                    }
                    let mb = self.tx[i];
                    let (id, len, data) = msg_fields(mb.tir, mb.tdtr, mb.tdlr, mb.tdhr);
                    sys.push_event(crate::system::VmEvent::CanTx { can: self.can_num(), id, len, data });
                    // Loopback mode (LBKM, BTR.30, without SILM.31): the
                    // transmitted frame is received into our own FIFOs
                    // (through the filters, like silicon).
                    if self.btr & (1 << 30) != 0 && self.btr & (1u32 << 31) == 0 {
                        self.inject_message(sys, mb.tir, mb.tdtr, mb.tdlr, mb.tdhr);
                    }
                    // TX completion edge: pend the TX IRQ once if TMEIE is
                    // armed (RQCP 0->1 equivalent; see fire_interrupts note).
                    if self.ier & 0x01 != 0 {
                        sys.p.nvic.borrow_mut().set_intr_pending(self.irq_base);
                    }
                    self.fire_interrupts(sys);
                }
            }
            0x1B0..=0x1CC => {
                let i = ((offset - 0x1B0) / 0x10) as usize;
                if i >= 2 { return; }
                match (offset - 0x1B0) % 0x10 {
                    0x00 => self.rx[i].tir = value,
                    0x04 => self.rx[i].tdtr = value,
                    0x08 => self.rx[i].tdlr = value,
                    0x0C => self.rx[i].tdhr = value,
                    _ => {}
                }
            }
            0x200 => {
                // FMR CAN2SB[13:8] assigns banks [CAN2SB..28] to CAN2.
                if self.can_num() == 2 { return; }
                if value & 1 != 0 {
                    self.fm1r = 0; self.fs1r = 0xFFFF_FFFF; self.ffa1r = 0; self.fa1r = 0;
                }
                self.fmr = value & 0x3F3F;
            }
            0x204 => {
                if self.can_num() == 2 { return; }
                self.fm1r = value
            }
            0x20C => {
                if self.can_num() == 2 { return; }
                self.fs1r = value
            }
            0x214 => {
                if self.can_num() == 2 { return; }
                self.ffa1r = value
            }
            0x21C => {
                if self.can_num() == 2 { return; }
                self.fa1r = value
            }
            0x240..=0x31C => {
                if self.can_num() == 2 { return; }
                let i = ((offset - 0x240) / 4) as usize;
                if let Some(f) = self.filter.get_mut(i) { *f = value; }
            }
            _ => {}
        }
    }
}
