//! SDIO host (Secure Digital I/O, STM32F103 @ 0x4001_8000, IRQ 49, SDHC only).
//!
//! Pragmatic model in the style of the SPI/I2C devices: commands complete
//! synchronously when CPSMEN is written (real hardware takes ~100s of clocks,
//! but every firmware timeout is generous), and the data path streams through
//! a 32-word FIFO window backed by an `SdCard` ext-device image.
//!
//! Supported commands: CMD0/2/3/6/7/8/9/12/13/16/17/18/24/25/55 + ACMD41
//! (busy for the first polls, like a real power-up). Anything else gets a
//! lenient R1 success, matching the emulator's answer-if-possible philosophy.
//! No card attached → CMDSENT for no-response commands, CTIMEOUT otherwise.
//!
//! DMA: with DCTRL.DMAEN the transfer issues one `dma_request(11)` (DMA2 CH4,
//! the F1 SDIO channel); the normal DMA pump then moves bytes through the
//! FIFO pop/push path, so polled and DMA transfers share one implementation.
//! Transfer completion (rx drained / tx collected to DLEN) sets DATAEND +
//! DBCKEND. Only block-addressed SDHC cards are modeled (CCS=1).

use crate::system::System;
use super::Peripheral;

pub const SDIO_BASE: u32 = 0x4001_8000;
pub const SDIO_IRQ: i32 = 49;
/// Global DMA channel number for DMA2 CH4 (see Peripherals::dma_request).
pub const SDIO_DMA_CHANNEL: u32 = 11;
const FIFO_DEPTH_WORDS: usize = 32;

// STA event flags (latched until cleared via ICR).
const F_CMDREND: u32 = 1 << 6;
const F_CMDSENT: u32 = 1 << 7;
const F_DATAEND: u32 = 1 << 8;
const F_DBCKEND: u32 = 1 << 10;
const F_CTIMEOUT: u32 = 1 << 2;
// R1 status word: READY_FOR_DATA + TRAN state (+APP_CMD for CMD55).
const R1_READY_TRAN: u32 = 0x900;

#[derive(Default)]
struct DataXfer {
    active: bool,
    /// false = read (card->mem), true = write (mem->card).
    write: bool,
    /// Block address latched from ARG when the transfer was armed.
    lba: u32,
    /// Receive payload staging (read path).
    rx: Vec<u8>,
    rx_pos: usize,
    /// Transmit accumulator (write path).
    tx: Vec<u8>,
}

#[derive(Default)]
pub struct Sdio {
    name: String,
    power: u32,
    clkcr: u32,
    arg: u32,
    cmd: u32,
    respcmd: u32,
    resp: [u32; 4],
    dtimer: u32,
    dlen: u32,
    dctrl: u32,
    sta_latched: u32,
    mask: u32,
    app_cmd: bool,
    acmd41_polls: u32,
    blocklen: u32,
    selected: bool,
    xfer: DataXfer,
}

impl Sdio {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "SDIO" {
            Some(Box::new(Sdio {
                name: name.to_string(),
                blocklen: 512,
                ..Default::default()
            }))
        } else {
            None
        }
    }

    fn find_card(&self) -> Option<std::rc::Rc<std::cell::RefCell<crate::ext_devices::SdCard>>> {
        let ext = crate::system::get_ext_devices().lock().unwrap();
        ext.sd_cards.iter()
            .find(|c| c.borrow().name() == self.name)
            .cloned()
    }

    /// Set an event flag; pend IRQ49 when the corresponding MASK bit is set.
    fn raise(&mut self, sys: &System, flag: u32) {
        self.sta_latched |= flag;
        if self.mask & flag != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(SDIO_IRQ);
        }
    }

    fn rx_remaining(&self) -> usize {
        if !self.xfer.active || self.xfer.write { 0 } else { self.xfer.rx.len() - self.xfer.rx_pos }
    }

    fn tx_pending(&self) -> usize {
        if !self.xfer.active || !self.xfer.write { 0 } else { self.xfer.tx.len() }
    }

    /// Pop up to `n` bytes (LE) from the receive path. Short reads at the tail
    /// return what is left; reads past the end return zeros.
    fn pop_bytes(&mut self, sys: &System, n: usize) -> u32 {
        let mut out = 0u32;
        for i in 0..n.min(4) {
            let b = if self.xfer.rx_pos < self.xfer.rx.len() {
                let b = self.xfer.rx[self.xfer.rx_pos];
                self.xfer.rx_pos += 1;
                b
            } else { 0 };
            out |= (b as u32) << (i * 8);
        }
        // Stage words only model occupancy; the byte stream is authoritative.
        if self.rx_remaining() == 0 && self.xfer.active && !self.xfer.write {
            self.finish_rx(sys);
        }
        out
    }

    /// Push `n` bytes (LE) into the transmit path; finalizes at DLEN.
    /// Bytes pushed with no transfer armed are ignored (lenient).
    fn push_bytes(&mut self, sys: &System, value: u32, n: usize) {
        if !self.xfer.active || !self.xfer.write {
            return;
        }
        for i in 0..n.min(4) {
            self.xfer.tx.push(((value >> (i * 8)) & 0xFF) as u8);
        }
        if self.xfer.tx.len() >= self.dlen as usize {
            self.finish_tx(sys);
        }
    }

    fn finish_rx(&mut self, sys: &System) {
        self.xfer.active = false;
        self.raise(sys, F_DATAEND | F_DBCKEND);
    }

    fn finish_tx(&mut self, sys: &System) {
        let data = std::mem::take(&mut self.xfer.tx);
        let lba = self.xfer.lba;
        if let Some(card) = self.find_card() {
            card.borrow_mut().write_block(lba, &data);
        }
        self.xfer.active = false;
        self.raise(sys, F_DATAEND | F_DBCKEND);
    }

    /// Start a read transfer: stage DLEN bytes from the card image.
    fn start_read(&mut self, sys: &System, lba: u32) {
        let mut payload = vec![0u8; self.dlen as usize];
        if let Some(card) = self.find_card() {
            card.borrow().read_block(lba, &mut payload);
        }
        self.xfer = DataXfer { active: true, write: false, lba, rx: payload, rx_pos: 0, tx: Vec::new() };
        if self.dctrl & (1 << 3) != 0 {
            sys.p.dma_request(sys, SDIO_DMA_CHANNEL);
        }
        // Empty transfer completes immediately (DATAEND with no data phase).
        if self.dlen == 0 {
            self.finish_rx(sys);
        }
    }

    /// Arm a write transfer; completion lands in push_bytes at DLEN.
    fn start_write(&mut self, sys: &System) {
        let dlen = self.dlen as usize;
        let lba = self.arg;
        self.xfer = DataXfer { active: true, write: true, lba, rx: Vec::new(), rx_pos: 0, tx: Vec::with_capacity(dlen) };
        if self.dctrl & (1 << 3) != 0 {
            sys.p.dma_request(sys, SDIO_DMA_CHANNEL);
        }
        if dlen == 0 {
            // Nothing to move: still report completion (lenient).
            self.finish_tx(sys);
        }
    }

    /// Execute a command written with CPSMEN set. All responses are produced
    /// synchronously; STA event flags + IRQ follow the MASK.
    fn exec_cmd(&mut self, sys: &System, idx: u32) {
        self.respcmd = idx;
        // CPSMEN clears once the command is taken (response path started).
        self.cmd &= !(1 << 10);
        let card = self.find_card();
        let has_card = card.is_some();

        // No-response commands complete with CMDSENT even without a card.
        let waitresp = (self.cmd >> 6) & 3;
        if !has_card {
            if waitresp == 0 {
                self.raise(sys, F_CMDSENT);
            } else {
                self.raise(sys, F_CTIMEOUT);
            }
            self.app_cmd = idx == 55;
            return;
        }
        let card = card.unwrap();

        match idx {
            0 => { // GO_IDLE_STATE: no response.
                self.acmd41_polls = 0;
                self.selected = false;
                self.raise(sys, F_CMDSENT);
            }
            2 => { // ALL_SEND_CID: R2.
                let cid = card.borrow().cid();
                self.resp = cid;
                self.raise(sys, F_CMDREND);
            }
            3 => { // SEND_RELATIVE_ADDR: R6.
                self.resp[0] = card.borrow().r6();
                self.raise(sys, F_CMDREND);
            }
            6 | 7 | 12 | 13 => { // SWITCH/SELECT/STOP/STATUS: R1.
                if idx == 7 {
                    self.selected = (self.arg >> 16) == card.borrow().rca;
                }
                if idx == 12 {
                    self.xfer.active = false;
                }
                self.resp[0] = R1_READY_TRAN;
                self.raise(sys, F_CMDREND);
            }
            8 => { // SEND_IF_COND: R7 echoes the argument.
                self.resp[0] = self.arg;
                self.raise(sys, F_CMDREND);
            }
            9 => { // SEND_CSD: R2.
                let csd = card.borrow().csd();
                self.resp = csd;
                self.raise(sys, F_CMDREND);
            }
            16 => { // SET_BLOCKLEN.
                if self.arg == 512 {
                    self.blocklen = 512;
                }
                self.resp[0] = R1_READY_TRAN;
                self.raise(sys, F_CMDREND);
            }
            17 | 18 => { // READ_SINGLE/MULTIPLE_BLOCK.
                self.resp[0] = R1_READY_TRAN;
                self.raise(sys, F_CMDREND);
                if self.dctrl & 1 != 0 {
                    self.start_read(sys, self.arg);
                }
            }
            24 | 25 => { // WRITE_BLOCK/MULTIPLE_BLOCK.
                self.resp[0] = R1_READY_TRAN;
                self.raise(sys, F_CMDREND);
                if self.dctrl & 1 != 0 {
                    self.start_write(sys);
                }
            }
            41 if self.app_cmd => { // ACMD41: R3 OCR, busy for the first polls.
                self.acmd41_polls += 1;
                let ready = self.acmd41_polls >= 3;
                self.resp[0] = card.borrow().ocr(ready);
                self.raise(sys, F_CMDREND);
            }
            55 => { // APP_CMD: R1 with APP_CMD bit.
                self.resp[0] = R1_READY_TRAN | 0x20;
                self.raise(sys, F_CMDREND);
            }
            _ => { // Lenient R1 for anything else.
                self.resp[0] = R1_READY_TRAN;
                self.raise(sys, if waitresp == 0 { F_CMDSENT } else { F_CMDREND });
            }
        }
        self.app_cmd = idx == 55;
    }

    fn sta(&self) -> u32 {
        let mut v = self.sta_latched;
        // Dynamic FIFO/activity state.
        let rx_rem = self.rx_remaining();
        if self.xfer.active && !self.xfer.write { v |= 1 << 13; }       // RXACT
        if self.xfer.active && self.xfer.write { v |= 1 << 12; }        // TXACT
        if rx_rem >= FIFO_DEPTH_WORDS * 4 { v |= 1 << 17; }              // RXFIFOF
        if rx_rem == 0 { v |= 1 << 19; } else { v |= 1 << 21; }          // RXFIFOE/RXDAVL
        if rx_rem > FIFO_DEPTH_WORDS * 2 { v |= 1 << 15; }               // RXFIFOHF
        v |= 1 << 14;                                                   // TXFIFOHE (instant drain: always room)
        if self.tx_pending() == 0 { v |= 1 << 18; }                     // TXFIFOE
        v
    }

    fn read_reg(&mut self, sys: &System, offset: u32, size: u8) -> u32 {
        match offset {
            0x00 => self.power,
            0x04 => self.clkcr,
            0x08 => self.arg,
            0x0C => self.cmd,
            0x10 => self.respcmd,
            0x14 => self.resp[0],
            0x18 => self.resp[1],
            0x1C => self.resp[2],
            0x20 => self.resp[3],
            0x24 => self.dtimer,
            0x28 => self.dlen,
            0x2C => self.dctrl,
            0x30 => {
                // DCOUNT: bytes still to move (remaining on reads, DLEN-sent on writes).
                if self.xfer.active && self.xfer.write {
                    (self.dlen as usize).saturating_sub(self.xfer.tx.len()) as u32
                } else {
                    self.rx_remaining().min(self.dlen as usize) as u32
                }
            }
            0x34 => self.sta(),
            0x38 => 0, // ICR is write-only.
            0x3C => self.mask,
            0x48 => (self.rx_remaining().div_ceil(4)) as u32,
            0x80 => self.pop_bytes(sys, size as usize),
            _ => 0,
        }
    }

    fn write_reg(&mut self, sys: &System, offset: u32, size: u8, value: u32) {
        match offset {
            0x00 => self.power = value & 3,
            0x04 => self.clkcr = value,
            0x08 => self.arg = value,
            0x0C => {
                self.cmd = value;
                if value & (1 << 10) != 0 {
                    self.exec_cmd(sys, value & 0x3F);
                }
            }
            0x24 => self.dtimer = value,
            0x28 => self.dlen = value & 0x01FF_FFFF,
            0x2C => self.dctrl = value & 0xFFF,
            0x38 => self.sta_latched &= !value, // W1C.
            0x3C => self.mask = value & 0xFF_FFFF,
            0x80 => self.push_bytes(sys, value, size as usize),
            _ => {}
        }
    }
}

impl Peripheral for Sdio {
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        self.read_reg(sys, offset, 4)
    }
    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        self.write_reg(sys, offset, 4, value)
    }
    fn read_sized(&mut self, sys: &System, offset: u32, size: u8) -> u32 {
        self.read_reg(sys, offset, size)
    }
    fn write_sized(&mut self, sys: &System, offset: u32, size: u8, value: u32) {
        self.write_reg(sys, offset, size, value)
    }
}
