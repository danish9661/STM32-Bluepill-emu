use crate::system::System;
use super::Peripheral;

pub struct Exti {
    imr: u32,
    emr: u32,
    rtsr: u32,
    ftsr: u32,
    swier: u32,
    pr: u32,
}

impl Exti {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "EXTI" {
            Some(Box::new(Self { imr: 0, emr: 0, rtsr: 0, ftsr: 0, swier: 0, pr: 0 }))
        } else {
            None
        }
    }

    pub fn set_pending(&mut self, sys: &System, line: u32) {
        let mask = 1 << line;
        if line > 31 { return; }
        if self.imr & mask == 0 { return; }
        self.pr |= mask;
        sys.p.nvic.borrow_mut().set_intr_pending(exti_irq(line));
    }
}

fn exti_irq(line: u32) -> i32 {
    match line {
        0 => 6, 1 => 7, 2 => 8, 3 => 9, 4 => 10,
        5..=9 => 23,
        10..=15 => 40,
        16 => 1,
        17 => 3,
        18 => 42,
        _ => -1,
    }
}

impl Peripheral for Exti {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.imr,
            0x04 => self.emr,
            0x08 => self.rtsr,
            0x0C => self.ftsr,
            0x10 => self.swier,
            0x14 => self.pr,
            _ => 0,
        }
    }

    fn gpio_pin_changed(&mut self, sys: &System, port: u8, pin: u8, rising: bool) -> bool {
        let line = pin as u32;
        let mask = 1 << line;
        if line > 31 { return false; }
        if self.imr & mask == 0 { return false; }
        let edge_ok = if rising { self.rtsr & mask != 0 } else { self.ftsr & mask != 0 };
        if !edge_ok { return false; }
        if line < 16 {
            if let Some(expected) = sys.p.exti_port_for_line(line) {
                if (b'A' + port) as char != expected { return false; }
            }
        }
        self.pr |= mask;
        sys.p.nvic.borrow_mut().set_intr_pending(exti_irq(line));
        true
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.imr = value,
            0x04 => self.emr = value,
            0x08 => self.rtsr = value,
            0x0C => self.ftsr = value,
            0x10 => {
                let new_sw = value & !self.swier;
                self.swier |= value;
                for line in 0..32 {
                    if new_sw & (1 << line) != 0 {
                        self.set_pending(sys, line);
                    }
                }
            }
            0x14 => {
                self.pr &= !value;
            }
            _ => {}
        }
    }
}
