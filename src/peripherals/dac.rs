use crate::system::System;
use super::Peripheral;

pub struct Dac {
    cr: u32,
    swtrigr: u32,
    dhr12r1: u32,
    dhr12l1: u32,
    dhr8r1: u32,
    dhr12r2: u32,
    dhr12l2: u32,
    dhr8r2: u32,
    dhr12rd: u32,
    dhr12ld: u32,
    dhr8rd: u32,
    dor1: u32,
    dor2: u32,
    sr: u32,
}

impl Dac {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "DAC" {
            Some(Box::new(Self {
                cr: 0, swtrigr: 0,
                dhr12r1: 0, dhr12l1: 0, dhr8r1: 0,
                dhr12r2: 0, dhr12l2: 0, dhr8r2: 0,
                dhr12rd: 0, dhr12ld: 0, dhr8rd: 0,
                dor1: 0, dor2: 0, sr: 0,
            }))
        } else {
            None
        }
    }

    /// 12-bit voltage the DAC drives on the pin, if the channel is enabled.
    /// F103: DAC1 -> PA4, DAC2 -> PA5 (no remap options on this part).
    pub fn output_voltage(&self, port: u8, pin: u8) -> Option<u32> {
        if port == 0 && pin == 4 && self.cr & 1 != 0 {
            Some(self.dor1 & 0xFFF)
        } else if port == 0 && pin == 5 && self.cr & (1 << 4) != 0 {
            Some(self.dor2 & 0xFFF)
        } else {
            None
        }
    }

    /// Called after a DHR write — if DMA is enabled, fire a DMA request so the
    /// DMA controller can transfer the next sample from memory to DHR.
    fn fire_dma(&self, sys: &System, channel: u32) {
        sys.p.dma_request(sys, channel);
    }
}

impl Peripheral for Dac {
    fn dac_output(&self, port: u8, pin: u8) -> Option<u32> {
        self.output_voltage(port, pin)
    }

    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.swtrigr,
            0x08 => self.dhr12r1,
            0x0C => self.dhr12l1,
            0x10 => self.dhr8r1,
            0x14 => self.dhr12r2,
            0x18 => self.dhr12l2,
            0x1C => self.dhr8r2,
            0x20 => self.dhr12rd,
            0x24 => self.dhr12ld,
            0x28 => self.dhr8rd,
            0x2C => self.dor1,
            0x30 => self.dor2,
            0x34 => self.sr,
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.cr = value & 0x3F3F_003F,
            0x04 => self.swtrigr = value & 0x03,
            0x08 => {
                self.dhr12r1 = value & 0xFFF;
                self.dor1 = self.dhr12r1;
                sys.push_event(crate::system::VmEvent::DacWrite { chan: 1, value: self.dor1 });
                if self.cr & 1 != 0 { self.fire_dma(sys, 3); } // DMAEN1 → DMA1 ch3
            }
            0x0C => {
                self.dhr12l1 = value & 0xFFF0;
                self.dor1 = self.dhr12l1 >> 4;
                sys.push_event(crate::system::VmEvent::DacWrite { chan: 1, value: self.dor1 });
                if self.cr & 1 != 0 { self.fire_dma(sys, 3); }
            }
            0x10 => {
                self.dhr8r1 = value & 0xFF;
                self.dor1 = (self.dhr8r1 << 4) as u32;
                sys.push_event(crate::system::VmEvent::DacWrite { chan: 1, value: self.dor1 });
                if self.cr & 1 != 0 { self.fire_dma(sys, 3); }
            }
            0x14 => {
                self.dhr12r2 = value & 0xFFF;
                self.dor2 = self.dhr12r2;
                sys.push_event(crate::system::VmEvent::DacWrite { chan: 2, value: self.dor2 });
                if self.cr & (1 << 4) != 0 { self.fire_dma(sys, 4); } // DMAEN2 → DMA1 ch4
            }
            0x18 => {
                self.dhr12l2 = value & 0xFFF0;
                self.dor2 = self.dhr12l2 >> 4;
                sys.push_event(crate::system::VmEvent::DacWrite { chan: 2, value: self.dor2 });
                if self.cr & (1 << 4) != 0 { self.fire_dma(sys, 4); }
            }
            0x1C => {
                self.dhr8r2 = value & 0xFF;
                self.dor2 = (self.dhr8r2 << 4) as u32;
                sys.push_event(crate::system::VmEvent::DacWrite { chan: 2, value: self.dor2 });
                if self.cr & (1 << 4) != 0 { self.fire_dma(sys, 4); }
            }
            0x20 => {
                self.dhr12rd = value;
                self.dor1 = value & 0xFFF;
                self.dor2 = (value >> 16) & 0xFFF;
                sys.push_event(crate::system::VmEvent::DacWrite { chan: 1, value: self.dor1 });
                sys.push_event(crate::system::VmEvent::DacWrite { chan: 2, value: self.dor2 });
                if self.cr & 1 != 0 { self.fire_dma(sys, 3); }
                if self.cr & (1 << 4) != 0 { self.fire_dma(sys, 4); }
            }
            0x24 => {
                self.dhr12ld = value;
                self.dor1 = (value & 0xFFF0) >> 4;
                self.dor2 = ((value >> 16) & 0xFFF0) >> 4;
                sys.push_event(crate::system::VmEvent::DacWrite { chan: 1, value: self.dor1 });
                sys.push_event(crate::system::VmEvent::DacWrite { chan: 2, value: self.dor2 });
                if self.cr & 1 != 0 { self.fire_dma(sys, 3); }
                if self.cr & (1 << 4) != 0 { self.fire_dma(sys, 4); }
            }
            0x28 => {
                self.dhr8rd = value;
                self.dor1 = ((value & 0xFF) << 4) as u32;
                self.dor2 = (((value >> 8) & 0xFF) << 4) as u32;
                sys.push_event(crate::system::VmEvent::DacWrite { chan: 1, value: self.dor1 });
                sys.push_event(crate::system::VmEvent::DacWrite { chan: 2, value: self.dor2 });
                if self.cr & 1 != 0 { self.fire_dma(sys, 3); }
                if self.cr & (1 << 4) != 0 { self.fire_dma(sys, 4); }
            }
            0x34 => self.sr = value & 0x03,
            _ => {}
        }
    }
}
