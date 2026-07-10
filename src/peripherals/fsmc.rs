use std::{rc::Rc, cell::RefCell};
use crate::ext_devices::{ExtDevices, ExtDevice};
use crate::system::System;
use super::Peripheral;

pub struct Bank {
    pub name: String,
    ext_device: Option<Rc<RefCell<dyn ExtDevice<u32, u32>>>>,
    bcr: u32,
    btr: u32,
}

impl Bank {
    pub fn new(bank: usize, ext_devices: &ExtDevices) -> Self {
        let name = format!("FSMC.BANK{}", bank + 1);
        let ext_device = ext_devices.find_mem_device(&name);
        let name = ext_device.as_ref()
            .map(|d| d.borrow_mut().connect_peripheral(&name))
            .unwrap_or(name);
        Self { name, ext_device, bcr: 0, btr: 0 }
    }

    fn read_data(&mut self, sys: &System, offset: u32) -> u32 {
        self.ext_device.as_ref().map(|d| d.borrow_mut().read(sys, offset)).unwrap_or(0)
    }

    fn write_data(&mut self, sys: &System, offset: u32, value: u32) {
        if let Some(d) = self.ext_device.as_ref() {
            d.borrow_mut().write(sys, offset, value);
        }
    }
}

pub struct Fsmc {
    banks: [Bank; 4],
}

impl Fsmc {
    pub fn new(name: &str, ext_devices: &ExtDevices) -> Option<Box<dyn Peripheral>> {
        if name == "FSMC" {
            let banks = [
                Bank::new(0, ext_devices),
                Bank::new(1, ext_devices),
                Bank::new(2, ext_devices),
                Bank::new(3, ext_devices),
            ];
            Some(Box::new(Self { banks }))
        } else { None }
    }

    fn access(offset: u32) -> Access {
        match offset {
            0x0000_0000..=0x0FFF_FFFF => Access::Data(0, offset),
            0x1000_0000..=0x1FFF_FFFF => Access::Data(1, offset - 0x1000_0000),
            0x2000_0000..=0x2FFF_FFFF => Access::Data(2, offset - 0x2000_0000),
            0x3000_0000..=0x3FFF_FFFF => Access::Data(3, offset - 0x3000_0000),
            0x4000_0000..=0x4FFF_FFFF => {
                match offset - 0x4000_0000 {
                    0x0000 => Access::Register(0, 0),
                    0x0004 => Access::Register(0, 1),
                    0x0008 => Access::Register(1, 0),
                    0x000C => Access::Register(1, 1),
                    0x0010 => Access::Register(2, 0),
                    0x0014 => Access::Register(2, 1),
                    0x0018 => Access::Register(3, 0),
                    0x001C => Access::Register(3, 1),
                    0x0104 => Access::Register(0, 2),
                    0x010C => Access::Register(1, 2),
                    0x0114 => Access::Register(2, 2),
                    0x011C => Access::Register(3, 2),
                    _ => Access::Register(0, 0xFF),
                }
            }
            _ => Access::Register(0, 0xFF),
        }
    }
}

enum Access {
    Data(usize, u32),
    Register(usize, u8),
}

impl Peripheral for Fsmc {
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match Self::access(offset) {
            Access::Data(bank, off) => self.banks[bank].read_data(sys, off),
            Access::Register(bank, reg) => {
                match reg {
                    0 => self.banks[bank].bcr,
                    1 => self.banks[bank].btr,
                    _ => 0,
                }
            }
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match Self::access(offset) {
            Access::Data(bank, off) => self.banks[bank].write_data(sys, off, value),
            Access::Register(bank, reg) => {
                match reg {
                    0 => self.banks[bank].bcr = value,
                    1 => self.banks[bank].btr = value & 0x3FFF_FFFF,
                    2 => {}
                    _ => {}
                }
            }
        }
    }
}
