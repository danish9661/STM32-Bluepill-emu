use crate::system::System;
use super::Peripheral;

pub struct Afio {
    evcr: u32,
    mapr: u32,
    exticr: [u32; 4],
    mapr2: u32,
}

impl Afio {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "AFIO" {
            Some(Box::new(Self {
                evcr: 0, mapr: 0, exticr: [0; 4], mapr2: 0,
            }))
        } else {
            None
        }
    }
}

impl Peripheral for Afio {
    fn exti_port(&self, line: u32) -> Option<char> {
        if line >= 16 { return None; }
        let idx = (line / 4) as usize;
        let shift = (line % 4) * 4;
        let port_code = (self.exticr[idx] >> shift) & 0x7;
        match port_code {
            0 => Some('A'), 1 => Some('B'), 2 => Some('C'),
            3 => Some('D'), 4 => Some('E'),
            _ => None,
        }
    }

    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.evcr,
            0x04 => self.mapr,
            0x08..=0x14 => {
                let i = ((offset - 0x08) / 4) as usize;
                self.exticr.get(i).copied().unwrap_or(0)
            }
            0x1C => self.mapr2,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.evcr = value & 0xFF,
            0x04 => self.mapr = value,
            0x08..=0x14 => {
                let i = ((offset - 0x08) / 4) as usize;
                if let Some(r) = self.exticr.get_mut(i) { *r = value; }
            }
            0x1C => self.mapr2 = value & 0x07,
            _ => {}
        }
    }
}
