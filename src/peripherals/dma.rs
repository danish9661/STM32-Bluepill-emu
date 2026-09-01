use crate::system::{System, DmaTransfer, DmaDir, set_dma_intr_info};
use super::Peripheral;

pub struct Dma {
    name: String,
    isr: u32,
    ifcr: u32,
    channels: Vec<Channel>,
    num_channels: usize,
    /// DMA channel numbers that have pending requests from peripherals.
    /// Processed in tick(); keeps peripheral dma_request() free of borrow issues.
    pending_requests: Vec<u32>,
}

impl Default for Dma {
    fn default() -> Self {
        Self {
            name: String::new(),
            isr: 0, ifcr: 0,
            channels: Vec::new(),
            num_channels: 7,
            pending_requests: Vec::new(),
        }
    }
}

impl Dma {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "DMA1" {
            Some(Box::new(Self {
                name: name.to_string(),
                channels: vec![Channel::default(); 7],
                num_channels: 7,
                pending_requests: Vec::new(),
                ..Default::default()
            }))
        } else if name == "DMA2" {
            Some(Box::new(Self {
                name: name.to_string(),
                channels: vec![Channel::default(); 5],
                num_channels: 5,
                pending_requests: Vec::new(),
                ..Default::default()
            }))
        } else {
            None
        }
    }

    fn channel_irq(&self, ch: usize) -> i32 {
        // DMA1: IRQ 11-17 (channels 1-7). DMA2: IRQ 56-60 (channels 1-5).
        if self.name == "DMA2" {
            56 + ch as i32
        } else {
            11 + ch as i32
        }
    }
}

#[derive(Default, Clone)]
struct Channel {
    cr: u32,
    ndtr: u32,
    par: u32,
    mar: u32,
}

impl Channel {
    fn dir(&self) -> u8 { ((self.cr >> 4) & 1) as u8 }
    fn data_size(&self) -> usize {
        let psize = match (self.cr >> 8) & 0b11 { 0b00 => 1, 0b01 => 2, _ => 4 };
        let msize = match (self.cr >> 10) & 0b11 { 0b00 => 1, 0b01 => 2, _ => 4 };
        std::cmp::max(msize, psize) * self.ndtr as usize
    }

    fn do_xfer(&self, name: &str, sys: &System, ch: usize) {
        let m2m = self.cr & (1 << 14) != 0;
        let dir = self.dir();
        let (src, dst, direction, peripheral) = if m2m {
            (self.par, self.mar, DmaDir::MemCopy, false)
        } else if dir == 1 {
            (self.mar, self.par, DmaDir::Write, true)
        } else {
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
    fn dma_request(&mut self, _sys: &System, channel: u32) {
        if (channel as usize) < self.num_channels && !self.pending_requests.contains(&channel) {
            self.pending_requests.push(channel);
        }
    }

    fn tick(&mut self, sys: &System) {
        let nc = self.num_channels;
        let pending: Vec<u32> = self.pending_requests.drain(..).collect();
        for &ch in &pending {
            let ch_idx = ch as usize;
            if ch_idx < nc && self.channels[ch_idx].cr & 1 != 0 && self.channels[ch_idx].ndtr > 0 {
                self.channels[ch_idx].do_xfer(&self.name, sys, ch_idx);
            }
        }

        let bits = sys.dma_take_completions();
        if bits != 0 {
            for ch in 0..nc {
                if bits & (1 << ch) != 0 {
                    self.isr |= 1 << (ch * 4 + 1); // TCIF
                    self.channels[ch].cr &= !1;
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
                let nc = self.num_channels;
                if offset >= 0x08 && offset < 0x08 + (nc as u32) * 0x14 {
                    let ch = ((offset - 0x08) / 0x14) as usize;
                    let reg = (offset - 0x08) % 0x14;
                    if ch < nc {
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
        let nc = self.num_channels;
        match offset {
            0x00 => {}
            0x04 => {
                for ch in 0..nc {
                    let mask = value >> (ch * 4);
                    if mask & 0x0F != 0 {
                        self.isr &= !(mask << (ch * 4));
                    }
                }
            }
            _ => {
                if offset >= 0x08 && offset < 0x08 + (nc as u32) * 0x14 {
                    let ch = ((offset - 0x08) / 0x14) as usize;
                    let reg = (offset - 0x08) % 0x14;
                    if ch < nc {
                        match reg {
                            0x00 => {
                                self.channels[ch].cr = value & 0x7FFF;
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
