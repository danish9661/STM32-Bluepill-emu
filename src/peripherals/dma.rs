use crate::system::{System, DmaTransfer, DmaDir, set_dma_intr_info};
use super::Peripheral;

#[derive(Default)]
pub struct Dma {
    name: String,
    isr: u32,
    ifcr: u32,
    channels: [Channel; 7],
}

impl Dma {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name.starts_with("DMA") {
            Some(Box::new(Self { name: name.to_string(), ..Self::default() }))
        } else {
            None
        }
    }

    fn channel_irq(&self, ch: usize) -> i32 {
        11 + ch as i32
    }
}

#[derive(Default)]
struct Channel {
    cr: u32,
    ndtr: u32,
    par: u32,
    mar: u32,
}

impl Channel {
    fn dir(&self) -> u8 { ((self.cr >> 4) & 1) as u8 }
    fn data_size(&self) -> usize {
        let psize = match (self.cr >> 6) & 0b11 { 0b00 => 1, 0b01 => 2, _ => 4 };
        let msize = match (self.cr >> 8) & 0b11 { 0b00 => 1, 0b01 => 2, _ => 4 };
        std::cmp::max(msize, psize) * self.ndtr as usize
    }

    fn do_xfer(&self, name: &str, sys: &System, ch: usize) {
        let m2m = self.cr & (1 << 14) != 0;
        let dir = self.dir();
        let (src, dst, direction, peripheral) = if m2m {
            // RM0008: M2M always transfers CPAR -> CMAR (DIR ignored).
            (self.par, self.mar, DmaDir::MemCopy, false)
        } else if dir == 1 {
            // DIR=1: read from memory (CMAR), write to peripheral (CPAR).
            (self.mar, self.par, DmaDir::Write, true)
        } else {
            // DIR=0: read from peripheral (CPAR), write to memory (CMAR).
            (self.par, self.mar, DmaDir::Read, true)
        };
        let size = self.data_size();
        sys.queue_dma_transfer(DmaTransfer {
            direction,
            stream_idx: ch,
            dma_name: name.to_string(),
            src, dst, size,
            peri_addr: self.par,
            peripheral,
        });
    }
}

impl Peripheral for Dma {
    /// Peripheral request (ADC end-of-conversion etc.): if the channel is enabled
    /// and configured, move one transfer (real HW: the request line pulses).
    fn dma_request(&mut self, sys: &System, channel: u32) {
        let ch = channel as usize;
        if ch < 7 && self.channels[ch].cr & 1 != 0 {
            self.channels[ch].do_xfer(&self.name, sys, ch);
        }
    }

    fn tick(&mut self, sys: &System) {
        let bits = sys.dma_take_completions();
        if bits != 0 {
            for ch in 0..7 {
                if bits & (1 << ch) != 0 {
                    // Real HW layout: channel N flags sit at bits (N-1)*4..(N-1)*4+3,
                    // GIF=(N-1)*4, TCIF=(N-1)*4+1. For 0-based ch: TCIF = ch*4+1.
                    // (Was: bit (ch+1)*4 — off by one channel, silently wrong; the
                    // periph39 DMA tests only passed because both transfers completed
                    // into one shared ISR read.)
                    self.isr |= 1 << (ch * 4 + 1); // TCIF
                    self.channels[ch].cr &= !1;    // clear EN
                    self.channels[ch].ndtr = 0;
                }
            }
        }
    }

    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.isr,
            0x04 => self.ifcr,
            _ => {
                if offset >= 0x08 && offset < 0x98 {
                    let ch = ((offset - 0x08) / 0x14) as usize;
                    let reg = (offset - 0x08) % 0x14;
                    if ch < 7 {
                        return match reg {
                            0x00 => self.channels[ch].cr,
                            0x04 => self.channels[ch].ndtr,
                            0x08 => self.channels[ch].par,
                            0x0C => self.channels[ch].mar,
                            _ => 0,
                        };
                    }
                }
                0
            }
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {}
            0x04 => {
                for ch in 0..7 {
                    let mask = value >> (ch * 4);
                    if mask & 0x0F != 0 {
                        self.isr &= !(mask << (ch * 4));
                    }
                }
            }
            _ => {
                if offset >= 0x08 && offset < 0x98 {
                    let ch = ((offset - 0x08) / 0x14) as usize;
                    let reg = (offset - 0x08) % 0x14;
                    if ch < 7 {
                        match reg {
                            0x00 => {
                                self.channels[ch].cr = value & 0x7F7F;
                                if value & 1 != 0 {
                                    self.channels[ch].do_xfer(&self.name, sys, ch);
                                    let irq = self.channel_irq(ch);
                                    let cr = self.channels[ch].cr;
                                    let tcie = ((cr >> 4) & 1) as u8;
                                    let htie = ((cr >> 3) & 1) as u8;
                                    let teie = ((cr >> 2) & 1) as u8;
                                    let flags = tcie | (htie << 1) | (teie << 2);
                                    set_dma_intr_info(ch, irq, flags);
                                }
                            }
                            0x04 => self.channels[ch].ndtr = value & 0xFFFF,
                            0x08 => self.channels[ch].par = value,
                            0x0C => self.channels[ch].mar = value,
                            _ => {}
                        }
                    }
                }
            }
        }
    }
}
