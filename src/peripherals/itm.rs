use crate::system::System;
use super::Peripheral;

/// ARM ITM stimulus (0xE0000000, Cortex-M3 PPB): the firmware printf
/// channel. Only stimulus port 0 is modeled (what every retarget
/// uses): a word write pushes its low byte as an `ItmByte` event when
/// the port is enabled (TER[0]) and the ITM itself is on (TCR.ITMENA).
/// TER/TPR/TCR are stored; LAR accepts writes (unlock is invisible at
/// this level). STIM reads report FIFO-empty (always ready). ATB/TPIU,
/// timestamps, sync packets and ports 1-31 are out of scope.
pub struct Itm {
    ter: u32,
    tpr: u32,
    tcr: u32,
}

impl Itm {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "ITM" {
            Some(Box::new(Self { ter: 0, tpr: 0, tcr: 0 }))
        } else {
            None
        }
    }

    fn port_enabled(&self, port: usize) -> bool {
        port < 32 && self.tcr & 1 != 0 && self.ter & (1 << port) != 0
    }
}

impl Peripheral for Itm {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x000..=0x07C => 0, // STIM: bit 1 FIFO-full always reads 0 (ready)
            0xE00 => self.ter,
            0xE40 => self.tpr,
            0xE80 => self.tcr,
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x000..=0x07C => {
                let port = (offset / 4) as usize;
                if port == 0 && self.port_enabled(0) {
                    let byte = (value & 0xFF) as u8;
                    sys.push_event(crate::system::VmEvent::ItmByte { port: 0, byte });
                }
            }
            0xE00 => self.ter = value,
            0xE40 => self.tpr = value,
            0xE80 => self.tcr = value & 0x1000F,
            0xFB0 => {} // LAR: unlock invisible here
            _ => {}
        }
    }
}
