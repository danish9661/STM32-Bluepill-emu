//! USB OTG FS device + host (STM32F105 @ 0x5000_0000, IRQ 67, data FIFOs
//! at 0x5000_1000 + N*0x1000). Synchronous transaction model in the style
//! of the FS-device (`usb.rs`): transfers complete when the firmware arms
//! them, which is exact for control/bulk/interrupt firmware.
//!
//! Implemented, device mode: GOTGCTL/GOTGINT (stored/0), GAHBCFG (GINT
//! global gate), GUSBCFG (stored), GRSTCTL (CSFTRST full reset, RXFFLSH/
//! TXFFLSH+TXFNUM FIFO flushes, AHBIDL always 1), GINTSTS/GINTMSK (W1C
//! event flags, RXFLVL/NPTXFE/CMOD derived, level-sensitive IRQ recalc),
//! GRXSTSR/GRXSTSP (real status queue: SETUP/OUT received+completed),
//! GRXFSIZ/GNPTXFSIZ/HPTXFSIZ/DIEPTXFx (stored, sizes unenforced),
//! GNPTXSTS/DTXFSTSx (generous space), GCCFG (stored; PWRDWN gates like
//! FS PDWN), CID (read-only), DCFG (DAD address filter), DCTL (RWUSIG
//! wake, SDIS disconnect, SGONAK/CGONAK global-NAK strobes), DSTS
//! (SUSPSTS/ENUMSPD/FNSOF), DIEPMSK/DOEPMSK + DAINT + DAINTMSK three-level
//! masking into IEPINT/OEPINT, DIEPCTL/DOEPCTL x4 (EPENA/SNAK/CNAK/STALL/
//! NAKSTS/USBAEP hardware semantics, EPDIS completion), DIEPINT/DOEPINT x4
//! (W1C XFRC/EPDISD/STUP), DIEPTSIZ/DOEPTSIZ x4 (XFRSIZ/PKTCNT/STUPCNT
//! accounting), PCGCCTL (stored), EP0-3 IN/OUT data FIFOs, SOF engine
//! (FNSOF + SOF IRQ), suspend/resume (SDIS/detach/RWUSIG/bus-reset), host
//! bus reset + detach inject APIs, DAD address filtering on injects.
//!
//! Implemented, host mode: HCFG/HFIR (stored), HFNUM (SOF frame counter),
//! HPTXSTS (generous space), HAINT/HAINTMSK into GINTSTS HCINT, HPRT
//! (PCSTS follows virtual-device attach, PCDET + HPRTINT on edges,
//! PENA follows PPWR, PRST stored), 8 host channels (HCCHAR/HCSPLT/
//! HCINT(W1C XFRC/CHHLT/STALL)/HCINTMSK/HCTSIZ; HCDMA reads 0), OUT/SETUP
//! completion drained as HostTx events when pushed FIFO bytes reach
//! XFRSIZ (CHENA first, data after — ST's exact order), IN tokens drained
//! as HostRx request events and completed by otg_host_feed_in (data or
//! STALL), channel halt (CHDIS -> CHHLT), GRXSTSP/RXFIFO shared with the
//! device path (one mode at a time in practice).
//!
//! Synchronous-completion rule (differs from the FS PMA model because ST
//! programs TSIZ + EPENA/CHENA *before* pushing FIFO data): an IN/OUT
//! transfer completes when pushed FIFO bytes reach XFRSIZ (zero-length
//! when XFRSIZ == 0 with packets programmed); OUT/SETUP device packets
//! complete per injected packet.
//! Deliberately absent: host-mode NAK/timeout/retry/timeout-driven
//! aborts (the scripted peer always answers; pending transfers simply
//! wait), ping, split/LS transactions, isochronous SOF-gating (ISO shares
//! bulk mechanics, like the FS model), DMA registers (the FS core has no
//! DMA engine; reads return 0), TXFE/empty interrupts (FIFOs never fill in
//! emulation), OTG negotiation HNP/SRP (always B-device attached /
//! host-driven attach API), ESOF (host never misses).

use crate::system::{System, VmEvent};
use super::Peripheral;

pub const OTG_BASE: u32 = 0x5000_0000;
/// End (exclusive) of the OTG window: registers + DFIFO0-3 (4 KB stride).
pub const OTG_END: u32 = 0x5000_5000;
/// OTG FS global interrupt vector.
pub const OTG_IRQ: i32 = 67;
/// Instructions per USB frame (1 ms @ 72 MHz): the SOF engine's tick.
const SOF_PERIOD: u64 = 72_000;

// Register offsets (relative to OTG_BASE).
const GOTGCTL: u32 = 0x000;
const GOTGINT: u32 = 0x004;
const GAHBCFG: u32 = 0x008;
const GUSBCFG: u32 = 0x00C;
const GRSTCTL: u32 = 0x010;
const GINTSTS: u32 = 0x014;
const GINTMSK: u32 = 0x018;
const GRXSTSR: u32 = 0x01C;
const GRXSTSP: u32 = 0x020;
const GRXFSIZ: u32 = 0x024;
const GNPTXFSIZ: u32 = 0x028;
const GNPTXSTS: u32 = 0x02C;
const GCCFG: u32 = 0x038;
const CID: u32 = 0x03C;
const HPTXFSIZ: u32 = 0x100;
const DIEPTXF1: u32 = 0x104;
// Host block 0x400-0x7FF: inert in device-only emulation.
const DCFG: u32 = 0x800;
const DCTL: u32 = 0x804;
const DSTS: u32 = 0x808;
const DIEPMSK: u32 = 0x810;
const DOEPMSK: u32 = 0x814;
const DAINT: u32 = 0x818;
const DAINTMSK: u32 = 0x81C;
const DVBUSDIS: u32 = 0x828;
const DVBUSPULSE: u32 = 0x82C;
const DIEPEMPMSK: u32 = 0x834;
const PCGCCTL: u32 = 0xE00;
// Data FIFO window: DFIFO[n] at 0x1000 + n*0x1000 (word-accessed).

// GINTSTS/GINTMSK bits.
const CMOD: u32 = 1 << 0;
const OTGINT: u32 = 1 << 2;
const SOF: u32 = 1 << 3;
const RXFLVL: u32 = 1 << 4;
const NPTXFE: u32 = 1 << 5;
const USBSUSP: u32 = 1 << 11;
const USBRST: u32 = 1 << 12;
const ENUMDNE: u32 = 1 << 13;
const IEPINT: u32 = 1 << 18;
const OEPINT: u32 = 1 << 19;
const WKUPINT: u32 = 1 << 31;
/// Read-only GINTSTS bits (derived status, never W1C-cleared).
const GINT_RO: u32 = CMOD | RXFLVL | NPTXFE | OTGINT;
// GAHBCFG bits.
const GAHB_GINT: u32 = 1 << 0;
// GRSTCTL bits.
const GRST_CSRST: u32 = 1 << 0;
const GRST_RXFFLSH: u32 = 1 << 4;
const GRST_TXFFLSH: u32 = 1 << 5;
const GRST_TXFNUM_SHIFT: u32 = 6;
const GRST_AHBIDL: u32 = 1 << 31;
// DCFG bits.
const DCFG_DAD_SHIFT: u32 = 4;
const DCFG_DAD_MASK: u32 = 0x7F;
// DCTL bits.
const DCTL_RWUSIG: u32 = 1 << 0;
const DCTL_SDIS: u32 = 1 << 1;
const DCTL_SGONAK: u32 = 1 << 9;
const DCTL_CGONAK: u32 = 1 << 10;
// DSTS.
const DSTS_SUSPSTS: u32 = 1 << 0;
const DSTS_ENUMSPD_SHIFT: u32 = 1;
// Endpoint CTL bits (shared IN/OUT layout).
const EP_MPSIZ_MASK: u32 = 0x7FF;
const EP_USBAEP: u32 = 1 << 15;
const EP_NAKSTS: u32 = 1 << 17;
const EP_STALL: u32 = 1 << 21;
const EP_CNAK: u32 = 1 << 26;
const EP_SNAK: u32 = 1 << 27;
const EP_EPDIS: u32 = 1 << 30;
const EP_EPENA: u32 = 1 << 31;
// Endpoint INT bits.
const EPINT_XFRC: u32 = 1 << 0;
const EPINT_EPDISD: u32 = 1 << 1;
const EPINT_STUP: u32 = 1 << 3;
// GRXSTSP PKTSTS codes + field shifts.
const PKTSTS_OUT_RX: u32 = 2;
const PKTSTS_OUT_DONE: u32 = 3;
const PKTSTS_SETUP_DONE: u32 = 4;
const PKTSTS_SETUP_RX: u32 = 6;
// GCCFG bits.
const GCCFG_PWRDWN: u32 = 1 << 16;

// Host-mode GINTSTS bits.
const HPRTINT: u32 = 1 << 24;
const HCINTB: u32 = 1 << 25;
// HPRT bits.
const HPRT_PCSTS: u32 = 1 << 0;
const HPRT_PCDET: u32 = 1 << 1;
const HPRT_PENA: u32 = 1 << 2;
const HPRT_PRST: u32 = 1 << 8;
const HPRT_PPWR: u32 = 1 << 12;
// Host channel CTL bits.
const HC_EPNUM_SHIFT: u32 = 11;
const HC_EPDIR: u32 = 1 << 15;
const HC_CHDIS: u32 = 1 << 30;
const HC_CHENA: u32 = 1 << 31;
// Host channel INT bits.
const HCINT_XFRC: u32 = 1 << 0;
const HCINT_CHHLT: u32 = 1 << 1;
const HCINT_STALL: u32 = 1 << 3;
const HCINT_ACK: u32 = 1 << 5;
// HCTSIZ fields.
const HCTSIZ_XFRSIZ_MASK: u32 = 0x7FFFF;
const HCTSIZ_PKTCNT_SHIFT: u32 = 19;
const HCTSIZ_DPID_SHIFT: u32 = 29;
/// HCTSIZ DPID value for SETUP tokens (ST HC_PID_SETUP).
const DPID_SETUP: u32 = 3;
// GRXSTSP host packet statuses.
const PKTSTS_HCHALTED: u32 = 7;

fn stat_grx(ep: usize, len: usize, pktsts: u32, frame: u16) -> u32 {
    ((ep as u32) & 0xF)
        | (((len as u32) & 0x7FF) << 4)
        | ((pktsts & 0xF) << 17)
        | (((frame as u32) & 0xF) << 21)
}

/// One endpoint's live state (IN and OUT arrays share the shape).
#[derive(Clone, Copy, Default)]
struct OtgEp {
    ctl: u32,
    int: u32,
    tsiz: u32,
    /// FIFO bytes pushed since the last EPENA arm (IN only).
    pushed: u32,
}

/// One host channel's live state.
#[derive(Clone, Copy, Default)]
struct OtgHc {
    char: u32,
    splt: u32,
    int: u32,
    intmsk: u32,
    tsiz: u32,
    /// NPTX-FIFO bytes pushed since the last CHENA arm (OUT/SETUP only).
    pushed: u32,
}

pub struct OtgFs {
    // Global config (stored).
    gotgctl: u32,
    gahbcfg: u32,
    gusbcfg: u32,
    grxfsiz: u32,
    gnptxfsiz: u32,
    gccfg: u32,
    hptxfsiz: u32,
    dieptxf: [u32; 3],
    // Interrupt state.
    gintsts: u32,
    gintmsk: u32,
    // Device config.
    dcfg: u32,
    dctl: u32,
    enumspd: u32,
    diepmsk: u32,
    doepmsk: u32,
    daintmsk: u32,
    dvbusdis: u32,
    dvbuspulse: u32,
    diepempmsk: u32,
    pcgcctl: u32,
    // Endpoints.
    in_ep: [OtgEp; 4],
    out_ep: [OtgEp; 4],
    // Host mode.
    hcfg: u32,
    hfir: u32,
    haintmsk: u32,
    hprt: u32,
    hc: [OtgHc; 8],
    /// Virtual device presence on the host port (set via otg_host_attach).
    host_attached: bool,
    // FIFOs (u32 words) + RX status queue.
    rxfifo: std::collections::VecDeque<u32>,
    txfifo: [std::collections::VecDeque<u32>; 8],
    grxq: std::collections::VecDeque<u32>,
    // SOF engine + link state.
    frame: u16,
    sof_acc: u64,
    last_tick: u64,
    suspended: bool,
    detached: bool,
    sdis: bool,
    gonak: bool,
}

impl OtgFs {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "USB_OTG" || name.starts_with("USB_OTG_") {
            Some(Box::new(Self::fresh()))
        } else {
            None
        }
    }

    fn fresh() -> Self {
        OtgFs {
            gotgctl: 0x800,
            gahbcfg: 0,
            gusbcfg: 0xA00,
            grxfsiz: 0x200,
            gnptxfsiz: 0x200,
            gccfg: 0,
            hptxfsiz: 0x0200_0600,
            dieptxf: [0x0200_0400, 0, 0],
            gintsts: 0,
            gintmsk: 0,
            dcfg: 0x0220_0000,
            dctl: 0,
            enumspd: 1,
            diepmsk: 0,
            doepmsk: 0,
            daintmsk: 0,
            dvbusdis: 0,
            dvbuspulse: 0,
            diepempmsk: 0,
            pcgcctl: 0,
            in_ep: [OtgEp::default(); 4],
            out_ep: [OtgEp::default(); 4],
            hcfg: 0,
            hfir: 0,
            haintmsk: 0,
            hprt: 0,
            hc: [OtgHc::default(); 8],
            host_attached: false,
            rxfifo: std::collections::VecDeque::new(),
            txfifo: [const { std::collections::VecDeque::new() }; 8],
            grxq: std::collections::VecDeque::new(),
            frame: 0,
            sof_acc: 0,
            last_tick: crate::system::instruction_count(),
            suspended: false,
            detached: false,
            sdis: false,
            gonak: false,
        }
    }

    fn powered(&self) -> bool {
        self.gccfg & GCCFG_PWRDWN == 0
    }

    fn link_down(&self) -> bool {
        self.detached || self.sdis
    }

    /// Recompute derived GINTSTS bits + the level-sensitive IRQ line.
    fn recalc(&mut self, sys: &System) {
        if self.grxq.is_empty() {
            self.gintsts &= !RXFLVL;
        } else {
            self.gintsts |= RXFLVL;
        }
        if self.gahbcfg & GAHB_GINT != 0
            && self.powered()
            && self.gintsts & self.gintmsk != 0
        {
            sys.p.nvic.borrow_mut().set_intr_pending(OTG_IRQ);
        }
    }

    fn set_gint(&mut self, sys: &System, bits: u32) {
        self.gintsts |= bits;
        self.recalc(sys);
    }

    /// DAINT low half: IN endpoints with unmasked pending IN flags.
    fn daint_in(&self) -> u32 {
        let mut v = 0u32;
        for (n, ep) in self.in_ep.iter().enumerate() {
            if ep.int & self.diepmsk != 0 {
                v |= 1 << n;
            }
        }
        v
    }

    /// DAINT high half: OUT endpoints with unmasked pending OUT flags.
    fn daint_out(&self) -> u32 {
        let mut v = 0u32;
        for (n, ep) in self.out_ep.iter().enumerate() {
            if ep.int & self.doepmsk != 0 {
                v |= 1 << n;
            }
        }
        v
    }

    fn daint(&self) -> u32 {
        self.daint_in() | (self.daint_out() << 16)
    }

    /// Fold DAINT through DAINTMSK into GINTSTS IEPINT/OEPINT.
    fn fold_daint(&mut self, sys: &System) {
        let in_pend = self.daint_in() & (self.daintmsk & 0xFFFF);
        let out_pend = self.daint_out() & ((self.daintmsk >> 16) & 0xFFFF);
        if in_pend != 0 {
            self.gintsts |= IEPINT;
        } else {
            self.gintsts &= !IEPINT;
        }
        if out_pend != 0 {
            self.gintsts |= OEPINT;
        } else {
            self.gintsts &= !OEPINT;
        }
        self.recalc(sys);
    }

    fn set_in_int(&mut self, sys: &System, n: usize, bits: u32) {
        self.in_ep[n].int |= bits;
        self.fold_daint(sys);
    }

    fn set_out_int(&mut self, sys: &System, n: usize, bits: u32) {
        self.out_ep[n].int |= bits;
        self.fold_daint(sys);
    }

    fn xfrsiz_in(&self, n: usize) -> u32 {
        if n == 0 {
            self.in_ep[0].tsiz & 0x7F
        } else {
            self.in_ep[n].tsiz & 0x3FFFF
        }
    }

    fn xfrsiz_out(&self, n: usize) -> u32 {
        if n == 0 {
            self.out_ep[0].tsiz & 0x7F
        } else {
            self.out_ep[n].tsiz & 0x3FFFF
        }
    }

    fn sub_xfrsiz_in(&mut self, n: usize, v: u32) {
        if n == 0 {
            let cur = self.in_ep[n].tsiz & 0x7F;
            let left = cur.saturating_sub(v);
            self.in_ep[n].tsiz = (self.in_ep[n].tsiz & !0x7F) | left;
            // PKTCNT[20:19] drains with the transfer.
            if left == 0 {
                self.in_ep[n].tsiz &= !(0x3 << 19);
            }
        } else {
            let cur = self.in_ep[n].tsiz & 0x3FFFF;
            let left = cur.saturating_sub(v);
            self.in_ep[n].tsiz = (self.in_ep[n].tsiz & !0x3FFFF) | left;
            if left == 0 {
                self.in_ep[n].tsiz &= !(0x3FF << 19);
            }
        }
    }

    fn sub_xfrsiz_out(&mut self, n: usize, v: u32) {
        if n == 0 {
            let cur = self.out_ep[n].tsiz & 0x7F;
            let left = cur.saturating_sub(v);
            self.out_ep[n].tsiz = (self.out_ep[n].tsiz & !0x7F) | left;
            if left == 0 {
                self.out_ep[n].tsiz &= !(1 << 19);
            }
        } else {
            let cur = self.out_ep[n].tsiz & 0x3FFFF;
            let left = cur.saturating_sub(v);
            self.out_ep[n].tsiz = (self.out_ep[n].tsiz & !0x3FFFF) | left;
            if left == 0 {
                self.out_ep[n].tsiz &= !(0x3FF << 19);
            }
        }
    }

    fn pktcnt_in(&self, n: usize) -> u32 {
        if n == 0 {
            (self.in_ep[0].tsiz >> 19) & 0x3
        } else {
            (self.in_ep[n].tsiz >> 19) & 0x3FF
        }
    }

    /// SOF engine + suspend tracking, once per batch (instruction-delta).
    /// A frame sets SOF (IRQ via SOFM); SOF reception is bus activity so an
    /// attached host never idles into suspend (same lesson as the FS
    /// device). Frozen while powered down or detached.
    fn tick_otg(&mut self, sys: &System) {
        use crate::system::instruction_count;
        let now = instruction_count();
        let delta = now.wrapping_sub(self.last_tick);
        self.last_tick = now;
        if delta == 0 {
            return;
        }
        if !self.powered() || self.detached {
            return;
        }
        if self.suspended {
            return;
        }
        self.sof_acc += delta;
        while self.sof_acc >= SOF_PERIOD {
            self.sof_acc -= SOF_PERIOD;
            self.frame = (self.frame + 1) & 0x3FFF;
            self.set_gint(sys, SOF);
        }
    }

    /// Full core reset (GRSTCTL CSFTRST): everything back to defaults
    /// (SVD reset values), keeping only the SOF clock base stable. A
    /// physically attached device stays attached across the soft reset
    /// (silicon keeps the PHY/pull-ups; HPRT PCSTS reflects the port),
    /// so host_attached survives — otherwise a pre-boot attach (the page
    /// attaches at load, firmware CSFTRSTs at boot) would boot into an
    /// E0 "no device" spin with no recovery.
    fn core_reset(&mut self) {
        let last_tick = self.last_tick;
        let host_attached = self.host_attached;
        *self = Self::fresh();
        self.last_tick = last_tick;
        self.host_attached = host_attached;
    }

    /// Host bus reset (SE0): endpoints + FIFOs + address reset, USBRST +
    /// ENUMDNE events, reattach (detach/suspend clear). Global config
    /// (GUSBCFG, FIFO carve, masks) survives like silicon.
    fn bus_reset(&mut self, sys: &System) -> bool {
        if !self.powered() {
            return false;
        }
        self.in_ep = [OtgEp::default(); 4];
        self.out_ep = [OtgEp::default(); 4];
        self.rxfifo.clear();
        for f in self.txfifo.iter_mut() {
            f.clear();
        }
        self.grxq.clear();
        self.dcfg &= !((DCFG_DAD_MASK) << DCFG_DAD_SHIFT);
        self.enumspd = 1; // full speed
        self.detached = false;
        self.suspended = false;
        self.set_gint(sys, USBRST | ENUMDNE);
        true
    }

    /// Host disconnect (pull-up off): tokens stop, IN never completes, SOF
    /// freezes. Cleared by the next bus reset (reattach).
    pub fn detach(&mut self, sys: &System) {
        self.detached = true;
        if !self.suspended {
            self.suspended = true;
            self.set_gint(sys, USBSUSP);
        } else {
            self.recalc(sys);
        }
    }

    // ----------------------------------------------------------
    // Host mode (device driver is firmware; the peer device is JS).
    // ----------------------------------------------------------

    fn hc_ep(&self, ch: usize) -> usize {
        ((self.hc[ch].char >> HC_EPNUM_SHIFT) & 0xF) as usize
    }

    fn hc_dir_in(&self, ch: usize) -> bool {
        self.hc[ch].char & HC_EPDIR != 0
    }

    fn hc_xfrsiz(&self, ch: usize) -> u32 {
        self.hc[ch].tsiz & HCTSIZ_XFRSIZ_MASK
    }

    fn hc_dpid(&self, ch: usize) -> u32 {
        (self.hc[ch].tsiz >> HCTSIZ_DPID_SHIFT) & 0x3
    }

    /// HAINT bit for a channel (masked per-channel flags, like DAINT).
    fn haint(&self) -> u32 {
        let mut v = 0u32;
        for (ch, hc) in self.hc.iter().enumerate() {
            if hc.int & hc.intmsk != 0 {
                v |= 1 << ch;
            }
        }
        v
    }

    /// Fold HAINT through HAINTMSK into GINTSTS HCINT.
    fn fold_haint(&mut self, sys: &System) {
        if self.haint() & (self.haintmsk & 0xFF) != 0 {
            self.gintsts |= HCINTB;
        } else {
            self.gintsts &= !HCINTB;
        }
        self.recalc(sys);
    }

    fn set_hc_int(&mut self, sys: &System, ch: usize, bits: u32) {
        self.hc[ch].int |= bits;
        self.fold_haint(sys);
    }

    /// Virtual-device attach/detach on the host port: PCSTS follows
    /// presence, edges raise PCDET + HPRTINT like silicon.
    pub fn host_attach(&mut self, sys: &System, present: bool) {
        if present != self.host_attached {
            self.host_attached = present;
            self.hprt |= HPRT_PCDET;
            self.set_gint(sys, HPRTINT);
        } else {
            self.recalc(sys);
        }
    }

    /// Device->host... host IN feed: the scripted peer answers a pending
    /// IN token on `ep` with `data` (or a STALL handshake). Matches the
    /// first armed IN channel addressed at `ep`; returns false when none
    /// is waiting.
    pub fn host_feed_in(&mut self, sys: &System, ep: usize, data: &[u8], stall: bool) -> bool {
        let mut target = None;
        for (ch, hc) in self.hc.iter().enumerate() {
            if hc.char & HC_CHENA != 0 && hc.char & HC_EPDIR != 0 && self.hc_ep(ch) == (ep & 0xF) {
                target = Some(ch);
                break;
            }
        }
        let ch = match target {
            Some(ch) => ch,
            None => return false,
        };
        self.hc[ch].char &= !HC_CHENA;
        if stall {
            self.set_hc_int(sys, ch, HCINT_STALL);
            return true;
        }
        // Stage answer bytes into the RX FIFO (LE words, last padded).
        for w in data.chunks(4).map(|c| {
            let mut w = 0u32;
            for (i, b) in c.iter().enumerate() {
                w |= (*b as u32) << (i * 8);
            }
            w
        }) {
            self.rxfifo.push_back(w);
        }
        self.grxq
            .push_back(stat_grx(ep, data.len(), PKTSTS_OUT_RX, self.frame));
        self.grxq
            .push_back(stat_grx(ep, data.len(), PKTSTS_OUT_DONE, self.frame));
        let mut tsiz = self.hc[ch].tsiz;
        tsiz = (tsiz & !HCTSIZ_XFRSIZ_MASK) | ((tsiz & HCTSIZ_XFRSIZ_MASK).saturating_sub(data.len() as u32));
        tsiz &= !(0x1FF << HCTSIZ_PKTCNT_SHIFT);
        self.hc[ch].tsiz = tsiz;
        self.set_hc_int(sys, ch, HCINT_XFRC | HCINT_ACK);
        true
    }

    /// Try to complete an armed host OUT/SETUP transfer (called on CHENA
    /// arm and on every NPTX-FIFO push). Zero-length completes at once;
    /// otherwise completion needs pushed >= XFRSIZ, drained as HostTx.
    fn try_complete_hc(&mut self, sys: &System, ch: usize) {
        let char = self.hc[ch].char;
        if char & HC_CHENA == 0 || self.hc_dir_in(ch) {
            return;
        }
        if self.link_down() || !self.powered() || !self.host_attached {
            return;
        }
        let want = self.hc_xfrsiz(ch) as usize;
        if want == 0 {
            self.finish_hc(sys, ch, 0);
            return;
        }
        if self.hc[ch].pushed as usize >= want {
            self.finish_hc(sys, ch, want);
        }
    }

    fn finish_hc(&mut self, sys: &System, ch: usize, len: usize) {
        // Drain transfer bytes from this channel's TX FIFO window.
        let mut data = Vec::with_capacity(len);
        let mut left = len;
        while left >= 4 {
            match self.txfifo[ch].pop_front() {
                Some(w) => {
                    data.push((w & 0xFF) as u8);
                    data.push(((w >> 8) & 0xFF) as u8);
                    data.push(((w >> 16) & 0xFF) as u8);
                    data.push(((w >> 24) & 0xFF) as u8);
                }
                None => break,
            }
            left -= 4;
        }
        if left > 0 {
            if let Some(w) = self.txfifo[ch].pop_front() {
                for i in 0..left {
                    data.push(((w >> (i * 8)) & 0xFF) as u8);
                }
            }
        }
        data.truncate(len);
        let ep = self.hc_ep(ch);
        let setup = self.hc_dpid(ch) == DPID_SETUP;
        sys.push_event(VmEvent::HostTx {
            ch: ch as u8,
            ep: ep as u8,
            setup,
            data,
        });
        let mut tsiz = self.hc[ch].tsiz;
        tsiz = (tsiz & !HCTSIZ_XFRSIZ_MASK) | ((tsiz & HCTSIZ_XFRSIZ_MASK).saturating_sub(len as u32));
        tsiz &= !(0x1FF << HCTSIZ_PKTCNT_SHIFT);
        self.hc[ch].tsiz = tsiz;
        self.hc[ch].char &= !HC_CHENA;
        self.hc[ch].pushed = 0;
        self.set_hc_int(sys, ch, HCINT_XFRC | HCINT_ACK);
    }

    fn enter_suspend(&mut self, sys: &System) {
        if !self.suspended {
            self.suspended = true;
            self.set_gint(sys, USBSUSP);
        } else {
            self.recalc(sys);
        }
    }

    fn wake(&mut self, sys: &System, wkup: bool) {
        self.suspended = false;
        if wkup {
            self.set_gint(sys, WKUPINT);
        } else {
            self.recalc(sys);
        }
    }

    /// Device->host IN completion for endpoint n: drain XFRSIZ bytes from
    /// its TX FIFO into a UsbIn event, then the hardware post-conditions
    /// (XFRC, EPENA clear, NAKSTS set).
    fn finish_in(&mut self, sys: &System, n: usize, len: usize) {
        let mut data = Vec::with_capacity(len);
        let mut left = len;
        while left >= 4 {
            match self.txfifo[n].pop_front() {
                Some(w) => {
                    data.push((w & 0xFF) as u8);
                    data.push(((w >> 8) & 0xFF) as u8);
                    data.push(((w >> 16) & 0xFF) as u8);
                    data.push(((w >> 24) & 0xFF) as u8);
                }
                None => break,
            }
            left -= 4;
        }
        if left > 0 {
            if let Some(w) = self.txfifo[n].pop_front() {
                for i in 0..left {
                    data.push(((w >> (i * 8)) & 0xFF) as u8);
                }
            }
        }
        data.truncate(len);
        sys.push_event(VmEvent::UsbIn {
            ep: n as u8,
            data,
        });
        self.sub_xfrsiz_in(n, len as u32);
        self.in_ep[n].ctl &= !EP_EPENA;
        self.in_ep[n].ctl |= EP_NAKSTS;
        self.in_ep[n].pushed = 0;
        self.set_in_int(sys, n, EPINT_XFRC);
    }

    /// Try to complete an armed IN transfer (called on EPENA arm and on
    /// every DFIFO push). Zero-length (XFRSIZ 0, PKTCNT set) completes
    /// immediately; otherwise completion needs pushed >= XFRSIZ.
    fn try_complete_in(&mut self, sys: &System, n: usize) {
        let ctl = self.in_ep[n].ctl;
        if ctl & EP_EPENA == 0 || ctl & EP_STALL != 0 {
            return;
        }
        if self.link_down() || !self.powered() {
            return;
        }
        let want = self.xfrsiz_in(n) as usize;
        if want == 0 {
            if self.pktcnt_in(n) > 0 {
                self.finish_in(sys, n, 0);
            }
            return;
        }
        if self.in_ep[n].pushed as usize >= want {
            self.finish_in(sys, n, want);
        }
    }

    /// Host-side delivery entry point (SETUP only on EP0). `addr` selects
    /// hardware address filtering against DCFG.DAD (None =
    /// correctly-addressed host). Returns false when dropped.
    pub fn inject(
        &mut self,
        sys: &System,
        ep: usize,
        data: &[u8],
        is_setup: bool,
        addr: Option<u8>,
    ) -> bool {
        if ep >= 4 || (is_setup && ep != 0) {
            return false;
        }
        // Powered-down macro, detached bus, or globally NAKed OUT: the PHY
        // sees nothing (SETUP shares the OUT NAK gate on silicon).
        if !self.powered() || self.link_down() || self.gonak {
            return false;
        }
        if let Some(a) = addr {
            // DCFG.DAD holds the assigned address (no enable bit on OTG:
            // address 0 answers only address 0).
            let dev = ((self.dcfg >> DCFG_DAD_SHIFT) & DCFG_DAD_MASK) as u8;
            if a != dev {
                return false;
            }
        }
        let ctl = self.out_ep[ep].ctl;
        // Endpoint must be enabled; STALL answers STALL (drop here).
        if ctl & EP_EPENA == 0 || ctl & EP_STALL != 0 {
            return false;
        }
        let armed = self.xfrsiz_out(ep) as usize;
        // An unarmed endpoint (XFRSIZ 0) drops OUT data; SETUP is always
        // 8 bytes with its own STUPCNT path, so it only needs EPENA.
        if !is_setup && armed == 0 && !data.is_empty() {
            return false; // nothing armed: drop like an unready endpoint.
        }
        // Stage packet bytes into the RX FIFO (LE words, last padded).
        for w in data.chunks(4).map(|c| {
            let mut w = 0u32;
            for (i, b) in c.iter().enumerate() {
                w |= (*b as u32) << (i * 8);
            }
            w
        }) {
            self.rxfifo.push_back(w);
        }
        // GRX status queue: received + completed (SETUP: received + done),
        // like silicon back-to-back.
        let (rx, done) = if is_setup {
            (PKTSTS_SETUP_RX, PKTSTS_SETUP_DONE)
        } else {
            (PKTSTS_OUT_RX, PKTSTS_OUT_DONE)
        };
        self.grxq.push_back(stat_grx(ep, data.len(), rx, self.frame));
        self.grxq.push_back(stat_grx(ep, data.len(), done, self.frame));
        self.sub_xfrsiz_out(ep, data.len() as u32);
        if ep == 0 && is_setup {
            // STUPCNT drains one setup per delivery.
            let stup = (self.out_ep[0].tsiz >> 29) & 0x3;
            if stup > 0 {
                self.out_ep[0].tsiz -= 1 << 29;
            }
            self.set_out_int(sys, ep, EPINT_XFRC | EPINT_STUP);
        } else {
            self.set_out_int(sys, ep, EPINT_XFRC);
        }
        // Core NAKs the endpoint after a completed transfer; EPENA drops
        // for non-isochronous endpoints.
        self.out_ep[ep].ctl |= EP_NAKSTS;
        self.out_ep[ep].ctl &= !EP_EPENA;
        true
    }

    /// HCCHAR write: full control word stored; CHDIS halts with CHHLT;
    /// CHENA rising edge arms a transfer (fresh push counter, plus an
    /// IN-request event for IN channels so the scripted peer can answer).
    fn write_hcchar(&mut self, sys: &System, ch: usize, v: u32) {
        const DIRECT: u32 = 0x7FF | (0xF << 11) | (1 << 15) | (1 << 17) | (0x3 << 18) | (0x3 << 20) | (0x7F << 22) | (1 << 29);
        let cur = self.hc[ch].char;
        let mut r = (cur & !DIRECT) | (v & DIRECT);
        if v & HC_CHDIS != 0 {
            r &= !(HC_CHENA | HC_CHDIS);
            self.hc[ch].char = r;
            self.hc[ch].pushed = 0;
            self.grxq
                .push_back(stat_grx(self.hc_ep(ch), 0, PKTSTS_HCHALTED, self.frame));
            self.set_hc_int(sys, ch, HCINT_CHHLT);
            return;
        }
        if v & HC_CHENA != 0 && cur & HC_CHENA == 0 {
            self.hc[ch].pushed = 0;
        }
        if v & HC_CHENA != 0 {
            r |= HC_CHENA;
        }
        self.hc[ch].char = r;
        if v & HC_CHENA != 0 && cur & HC_CHENA == 0 {
            if self.hc_dir_in(ch) {
                let ep = self.hc_ep(ch);
                let len = self.hc_xfrsiz(ch);
                sys.push_event(VmEvent::HostRx {
                    ch: ch as u8,
                    ep: ep as u8,
                    len,
                });
            }
            self.try_complete_hc(sys, ch);
        }
        self.recalc(sys);
    }

    fn write_ep_in_ctl(&mut self, sys: &System, n: usize, v: u32) {
        let cur = self.in_ep[n].ctl;
        let mut r = cur;
        // SETUP-free zone: direct fields follow the write.
        r = (r & !(EP_MPSIZ_MASK | (0x3 << 18) | EP_USBAEP))
            | (v & (EP_MPSIZ_MASK | (0x3 << 18) | EP_USBAEP));
        // TXFNUM (non-EP0 IN): direct.
        if n != 0 {
            r = (r & !(0xF << 22)) | (v & (0xF << 22));
        }
        // STALL follows the write directly (ST's SetStall/ClearStall use
        // RMW, so every real flow carries the intended bit as written).
        if v & EP_STALL != 0 {
            r |= EP_STALL;
        } else {
            r &= !EP_STALL;
        }
        // SNAK sets NAKSTS; CNAK is a write-1 strobe clearing it.
        if v & EP_SNAK != 0 {
            r |= EP_NAKSTS;
        }
        if v & EP_CNAK != 0 {
            r &= !EP_NAKSTS;
        }
        // EPDIS: tear the transfer down with an EPDISD event.
        if v & EP_EPDIS != 0 {
            r &= !(EP_EPENA | EP_EPDIS);
            self.in_ep[n].ctl = r;
            self.in_ep[n].pushed = 0;
            self.set_in_int(sys, n, EPINT_EPDISD);
            return;
        }
        // EPENA rising edge arms a transfer (fresh push counter).
        if v & EP_EPENA != 0 && cur & EP_EPENA == 0 {
            self.in_ep[n].pushed = 0;
        }
        if v & EP_EPENA != 0 {
            r |= EP_EPENA;
        }
        self.in_ep[n].ctl = r;
        self.try_complete_in(sys, n);
    }

    fn write_ep_out_ctl(&mut self, sys: &System, n: usize, v: u32) {
        let cur = self.out_ep[n].ctl;
        let mut r = cur;
        r = (r & !(EP_MPSIZ_MASK | (0x3 << 18) | EP_USBAEP))
            | (v & (EP_MPSIZ_MASK | (0x3 << 18) | EP_USBAEP));
        // STALL follows the write directly (ST's SetStall/ClearStall use
        // RMW, so every real flow carries the intended bit as written).
        if v & EP_STALL != 0 {
            r |= EP_STALL;
        } else {
            r &= !EP_STALL;
        }
        if v & EP_SNAK != 0 {
            r |= EP_NAKSTS;
        }
        if v & EP_CNAK != 0 {
            r &= !EP_NAKSTS;
        }
        if v & EP_EPDIS != 0 {
            r &= !(EP_EPENA | EP_EPDIS);
            self.out_ep[n].ctl = r;
            self.set_out_int(sys, n, EPINT_EPDISD);
            return;
        }
        if v & EP_EPENA != 0 {
            r |= EP_EPENA;
        }
        self.out_ep[n].ctl = r;
        self.recalc(sys);
    }

    fn read_reg(&mut self, sys: &System, offset: u32) -> u32 {
        // Data FIFOs: DFIFO0 reads pop the RX FIFO; other FIFOs read 0
        // (firmware only ever reads RX data through FIFO 0).
        if (0x1000..0x9000).contains(&offset) {
            let n = ((offset - 0x1000) / 0x1000) as usize;
            if n == 0 && offset % 4 == 0 {
                let w = self.rxfifo.pop_front().unwrap_or(0);
                self.recalc(sys);
                return w;
            }
            return 0;
        }
        match offset {
            GOTGCTL => self.gotgctl,
            GOTGINT => 0, // no OTG negotiation in device-only emulation.
            GAHBCFG => self.gahbcfg,
            GUSBCFG => self.gusbcfg,
            GRSTCTL => GRST_AHBIDL,
            GINTSTS => {
                (self.gintsts & !GINT_RO)
                    | CMOD
                    | NPTXFE
                    | if self.grxq.is_empty() { 0 } else { RXFLVL }
                    | self.gintsts & OTGINT
            }
            GINTMSK => self.gintmsk,
            GRXSTSR => self.grxq.front().copied().unwrap_or(0),
            GRXSTSP => {
                let w = self.grxq.pop_front().unwrap_or(0);
                self.recalc(sys);
                w
            }
            GRXFSIZ => self.grxfsiz,
            GNPTXFSIZ => self.gnptxfsiz,
            GNPTXSTS => 0x200, // generous nonperiodic space.
            GCCFG => self.gccfg,
            CID => 0x1000,
            HPTXFSIZ => self.hptxfsiz,
            DIEPTXF1 => self.dieptxf[0],
            0x108 => self.dieptxf[1],
            0x10C => self.dieptxf[2],
            DCFG => self.dcfg,
            DCTL => self.dctl & !(DCTL_SGONAK | (1 << 10)),
            DSTS => {
                (if self.suspended { DSTS_SUSPSTS } else { 0 })
                    | ((self.enumspd & 0x3) << DSTS_ENUMSPD_SHIFT)
                    | (((self.frame as u32) & 0x3FFF) << 8)
            }
            DIEPMSK => self.diepmsk,
            DOEPMSK => self.doepmsk,
            DAINT => self.daint(),
            DAINTMSK => self.daintmsk,
            DVBUSDIS => self.dvbusdis,
            DVBUSPULSE => self.dvbuspulse,
            DIEPEMPMSK => self.diepempmsk,
            PCGCCTL => self.pcgcctl,
            0x400 => self.hcfg,
            0x404 => self.hfir,
            0x408 => (self.frame as u32) & 0xFFFF,
            0x410 => 0x00080200, // generous periodic TX space.
            0x414 => self.haint(),
            0x418 => self.haintmsk,
            0x440 => {
                (if self.host_attached { HPRT_PCSTS } else { 0 })
                    | (self.hprt & (HPRT_PCDET | HPRT_PENA | HPRT_PRST | HPRT_PPWR))
            }
            o if (0x900..0x9C0).contains(&o) && (o - 0x900) % 0x20 < 0x18 => {
                let n = ((o - 0x900) / 0x20) as usize;
                match (o - 0x900) % 0x20 {
                    0x00 => self.in_ep[n].ctl & !EP_CNAK,
                    0x08 => self.in_ep[n].int,
                    0x10 => self.in_ep[n].tsiz,
                    _ => 0, // DIEPDMA: no DMA engine on the FS core.
                }
            }
            o if (0xB00..0xBC0).contains(&o) && (o - 0xB00) % 0x20 < 0x18 => {
                let n = ((o - 0xB00) / 0x20) as usize;
                match (o - 0xB00) % 0x20 {
                    0x00 => self.out_ep[n].ctl & !EP_CNAK,
                    0x08 => self.out_ep[n].int,
                    0x10 => self.out_ep[n].tsiz,
                    _ => 0, // DOEPDMA: no DMA engine on the FS core.
                }
            }
            o if (0x918..0x980).contains(&o) && (o - 0x918) % 0x20 == 0 => 0x200,
            o if (0x500..0x600).contains(&o) && (o - 0x500) % 0x20 < 0x18 => {
                let ch = ((o - 0x500) / 0x20) as usize;
                match (o - 0x500) % 0x20 {
                    0x00 => self.hc[ch].char,
                    0x04 => self.hc[ch].splt,
                    0x08 => self.hc[ch].int,
                    0x0C => self.hc[ch].intmsk,
                    0x10 => self.hc[ch].tsiz,
                    _ => 0, // HCDMA: no DMA engine on the FS core.
                }
            }
            _ => 0,
        }
    }

    fn write_reg(&mut self, sys: &System, offset: u32, value: u32) {
        // Data FIFOs: word pushes to the endpoint's TX FIFO (firmware
        // stages IN data after programming DIEPTSIZ + EPENA).
        if (0x1000..0x9000).contains(&offset) {
            let n = ((offset - 0x1000) / 0x1000) as usize;
            if n < 8 && offset % 4 == 0 {
                self.txfifo[n].push_back(value);
                // Device EP n and host channel n share window n (ST writes
                // DFIFO(ch) for host OUT, DFIFO(ep) for device IN); each
                // side completes only when it is itself armed.
                if n < 4 {
                    let pushed = self.in_ep[n].pushed + 4;
                    self.in_ep[n].pushed = pushed;
                    self.try_complete_in(sys, n);
                }
                let hpushed = self.hc[n].pushed + 4;
                self.hc[n].pushed = hpushed;
                self.try_complete_hc(sys, n);
            }
            return;
        }
        // Host block: HCFG/HFIR/HFNUM/HPTXSTS/HAINT/HAINTMSK/HPRT plus
        // per-channel HCCHAR/HCSPLT/HCINT/HCINTMSK/HCTSIZ (HCDMA stored).
        if offset == 0x400 {
            self.hcfg = value;
            return;
        }
        if offset == 0x404 {
            self.hfir = value;
            return;
        }
        if offset == 0x410 || offset == 0x414 || offset == 0x418 {
            return; // HPTXSTS/HAINT read-only; HAINTMSK below.
        }
        if offset == 0x41C {
            self.haintmsk = value;
            self.fold_haint(sys);
            return;
        }
        if offset == 0x440 {
            // HPRT: PCSTS/PCDET/PENA managed; PRST/PPWR stored.
            if value & HPRT_PCDET != 0 {
                self.hprt &= !HPRT_PCDET;
            }
            if value & HPRT_PPWR != 0 {
                self.hprt |= HPRT_PPWR | HPRT_PENA;
            } else {
                self.hprt &= !(HPRT_PPWR | HPRT_PENA);
            }
            if value & HPRT_PRST != 0 {
                self.hprt |= HPRT_PRST;
            } else {
                self.hprt &= !HPRT_PRST;
            }
            self.recalc(sys);
            return;
        }
        if (0x500..0x600).contains(&offset) {
            let ch = ((offset - 0x500) / 0x20) as usize;
            if ch < 8 {
                match (offset - 0x500) % 0x20 {
                    0x00 => self.write_hcchar(sys, ch, value),
                    0x04 => self.hc[ch].splt = value,
                    0x08 => {
                        self.hc[ch].int &= !value;
                        self.fold_haint(sys);
                    }
                    0x0C => {
                        self.hc[ch].intmsk = value;
                        self.fold_haint(sys);
                    }
                    0x10 => self.hc[ch].tsiz = value,
                    _ => {}
                }
            }
            return;
        }
        match offset {
            GOTGCTL => self.gotgctl = value,
            GOTGINT => {}
            GAHBCFG => {
                self.gahbcfg = value;
                self.recalc(sys);
            }
            GUSBCFG => self.gusbcfg = value,
            GRSTCTL => {
                if value & GRST_CSRST != 0 {
                    self.core_reset();
                    return;
                }
                if value & GRST_RXFFLSH != 0 {
                    self.rxfifo.clear();
                    self.grxq.clear();
                    self.recalc(sys);
                }
                if value & GRST_TXFFLSH != 0 {
                    let n = ((value >> GRST_TXFNUM_SHIFT) & 0x1F) as usize;
                    if n < 8 {
                        self.txfifo[n].clear();
                    } else {
                        for f in self.txfifo.iter_mut() {
                            f.clear();
                        }
                    }
                    self.recalc(sys);
                }
            }
            GINTSTS => {
                // W1C event flags (read-only status bits are preserved).
                self.gintsts &= !(value & !GINT_RO);
                self.recalc(sys);
            }
            GINTMSK => {
                self.gintmsk = value;
                self.recalc(sys);
            }
            GRXSTSR | GRXSTSP | GRXFSIZ => {}
            GNPTXFSIZ => self.gnptxfsiz = value,
            GNPTXSTS => {}
            GCCFG => self.gccfg = value,
            CID => {}
            HPTXFSIZ => self.hptxfsiz = value,
            DIEPTXF1 => self.dieptxf[0] = value,
            0x108 => self.dieptxf[1] = value,
            0x10C => self.dieptxf[2] = value,
            DCFG => self.dcfg = value,
            DCTL => {
                // RWUSIG/SDIS are state; SGONAK/CGONAK are strobes.
                if value & DCTL_RWUSIG != 0 {
                    self.dctl |= DCTL_RWUSIG;
                    if self.suspended {
                        self.dctl &= !DCTL_RWUSIG;
                        self.wake(sys, true);
                    }
                }
                let sdis = value & DCTL_SDIS != 0;
                if sdis && !self.sdis {
                    self.sdis = true;
                    self.dctl |= DCTL_SDIS;
                    self.enter_suspend(sys);
                } else if !sdis && self.sdis {
                    self.sdis = false;
                    self.dctl &= !DCTL_SDIS;
                    if !self.detached {
                        self.wake(sys, false);
                    }
                }
                if value & DCTL_SGONAK != 0 {
                    self.gonak = true;
                }
                if value & DCTL_CGONAK != 0 {
                    self.gonak = false;
                }
                self.recalc(sys);
            }
            DSTS | DIEPMSK | DOEPMSK => {}
            _ => {}
        }
        // Mask + misc stores handled above by early match arms; DIEPMSK etc
        // fall through here intentionally (see below).
        match offset {
            DIEPMSK => {
                self.diepmsk = value;
                self.fold_daint(sys);
            }
            DOEPMSK => {
                self.doepmsk = value;
                self.fold_daint(sys);
            }
            DAINT => {}
            DAINTMSK => {
                self.daintmsk = value;
                self.fold_daint(sys);
            }
            DVBUSDIS => self.dvbusdis = value,
            DVBUSPULSE => self.dvbuspulse = value,
            DIEPEMPMSK => self.diepempmsk = value,
            PCGCCTL => self.pcgcctl = value,
            _ => {}
        }
        // Endpoint register blocks.
        if (0x900..0x9C0).contains(&offset) && (offset - 0x900) % 0x20 < 0x18 {
            let n = ((offset - 0x900) / 0x20) as usize;
            match (offset - 0x900) % 0x20 {
                0x00 => self.write_ep_in_ctl(sys, n, value),
                0x08 => {
                    self.in_ep[n].int &= !value;
                    self.fold_daint(sys);
                }
                0x10 => self.in_ep[n].tsiz = value,
                _ => {}
            }
            return;
        }
        if (0xB00..0xBC0).contains(&offset) && (offset - 0xB00) % 0x20 < 0x18 {
            let n = ((offset - 0xB00) / 0x20) as usize;
            match (offset - 0xB00) % 0x20 {
                0x00 => self.write_ep_out_ctl(sys, n, value),
                0x08 => {
                    self.out_ep[n].int &= !value;
                    self.fold_daint(sys);
                }
                0x10 => self.out_ep[n].tsiz = value,
                _ => {}
            }
        }
    }
}

impl Peripheral for OtgFs {
    fn tick(&mut self, sys: &System) {
        self.tick_otg(sys);
    }
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        self.read_reg(sys, offset)
    }
    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        self.write_reg(sys, offset, value)
    }
    fn read_sized(&mut self, sys: &System, offset: u32, size: u8) -> u32 {
        if (0x1000..0x9000).contains(&offset) {
            // Data FIFOs are word-accessed like silicon; sub-word reads
            // return zero (UNPREDICTABLE on hardware).
            if size == 4 && offset % 4 == 0 {
                let n = ((offset - 0x1000) / 0x1000) as usize;
                if n == 0 {
                    let r = self.read(sys, offset);
                    return r;
                }
            }
            return 0;
        }
        let w = self.read(sys, offset & !3);
        match size {
            1 => (w >> ((offset % 4) * 8)) & 0xFF,
            2 => (w >> ((offset % 4) * 8)) & 0xFFFF,
            _ => w,
        }
    }
    fn write_sized(&mut self, sys: &System, offset: u32, size: u8, value: u32) {
        if (0x1000..0x9000).contains(&offset) {
            // Data FIFOs are word-accessed like silicon; sub-word writes
            // are ignored (UNPREDICTABLE on hardware).
            if size == 4 && offset % 4 == 0 {
                self.write(sys, offset, value);
            }
            return;
        }
        // 32-bit registers: merge sub-word stores.
        let base = offset & !3;
        let cur = self.read(sys, base);
        let shift = (offset % 4) * 8;
        let mask = if size >= 4 {
            0xFFFF_FFFF
        } else {
            ((1u32 << (size * 8)) - 1) << shift
        };
        self.write(sys, base, (cur & !mask) | ((value << shift) & mask));
    }

    fn otg_inject(
        &mut self,
        sys: &System,
        ep: usize,
        data: &[u8],
        is_setup: bool,
        addr: Option<u8>,
    ) -> bool {
        self.inject(sys, ep, data, is_setup, addr)
    }

    fn otg_bus_reset(&mut self, sys: &System) -> bool {
        self.bus_reset(sys)
    }

    fn otg_detach(&mut self, sys: &System) -> bool {
        self.detach(sys);
        true
    }

    fn otg_host_feed_in(&mut self, sys: &System, ep: usize, data: &[u8], stall: bool) -> bool {
        self.host_feed_in(sys, ep, data, stall)
    }

    fn otg_host_attach(&mut self, sys: &System, present: bool) -> bool {
        self.host_attach(sys, present);
        true
    }
}
