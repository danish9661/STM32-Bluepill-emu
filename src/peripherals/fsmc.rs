use std::{rc::Rc, cell::RefCell};
use crate::ext_devices::{ExtDevices, ExtDevice};
use crate::system::System;
use super::Peripheral;

/// FSMC memory regions. Bank1 = NOR/PSRAM with 4 chip-selects (NE1-4, 16MB each);
/// banks 2/3 = NAND, bank 4 = PC Card. Register block at 0xA000_0000.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Region {
    Nor(usize), // 0..3 -> NE1..NE4
    Nand(usize),// 0..1 -> NAND banks 2/3
    PcCard,
}

impl Region {
    fn index(self) -> usize {
        match self {
            Region::Nor(i) => i,          // 0..3 -> FSMC.BANK1..4
            Region::Nand(i) => 4 + i,     // 4..5 -> FSMC.BANK5..6
            Region::PcCard => 6,          //      -> FSMC.BANK7
        }
    }

    fn name(self) -> String {
        format!("FSMC.BANK{}", self.index() + 1)
    }

    fn from_index(i: usize) -> Region {
        match i {
            0..=3 => Region::Nor(i),
            4..=5 => Region::Nand(i - 4),
            _ => Region::PcCard,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reg {
    Bcr, Btr, Bwtr,
    Pcr, Pmem, Patt,
}

enum Access {
    Data(Region, u32),
    Reg(Region, Reg),
}

pub struct Bank {
    pub name: String,
    ext_device: Option<Rc<RefCell<dyn ExtDevice<u32, u32>>>>,
    bcr: u32,
    btr: u32,
    bwtr: u32,
    pcr: u32,
    pmem: u32,
    patt: u32,
}

impl Bank {
    fn new(region: Region, ext_devices: &ExtDevices) -> Self {
        let name = region.name();
        let ext_device = ext_devices.find_mem_device(&name);
        let name = ext_device.as_ref()
            .map(|d| d.borrow_mut().connect_peripheral(&name))
            .unwrap_or(name);
        Self { name, ext_device, bcr: 0, btr: 0, bwtr: 0, pcr: 0, pmem: 0, patt: 0 }
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
    banks: [Bank; 7],
}

impl Fsmc {
    pub fn new(name: &str, ext_devices: &ExtDevices) -> Option<Box<dyn Peripheral>> {
        if name == "FSMC" {
            Some(Box::new(Self { banks: Self::make_banks(ext_devices) }))
        } else { None }
    }

    fn make_banks(ext_devices: &ExtDevices) -> [Bank; 7] {
        std::array::from_fn(|i| Bank::new(Region::from_index(i), ext_devices))
    }

    fn access(offset: u32) -> Access {
        match offset {
            // Bank1 NE1-4: 4 x 16MB
            0x0000_0000..=0x03FF_FFFF => Access::Data(Region::Nor(0), offset),
            0x0400_0000..=0x07FF_FFFF => Access::Data(Region::Nor(1), offset - 0x0400_0000),
            0x0800_0000..=0x0BFF_FFFF => Access::Data(Region::Nor(2), offset - 0x0800_0000),
            0x0C00_0000..=0x0FFF_FFFF => Access::Data(Region::Nor(3), offset - 0x0C00_0000),
            // NAND banks 2/3
            0x1000_0000..=0x1FFF_FFFF => Access::Data(Region::Nand(0), offset - 0x1000_0000),
            0x2000_0000..=0x2FFF_FFFF => Access::Data(Region::Nand(1), offset - 0x2000_0000),
            // PC Card bank 4
            0x3000_0000..=0x3FFF_FFFF => Access::Data(Region::PcCard, offset - 0x3000_0000),
            // Register block
            0x4000_0000..=0x4FFF_FFFF => {
                match offset - 0x4000_0000 {
                    0x0000 => Access::Reg(Region::Nor(0), Reg::Bcr),
                    0x0004 => Access::Reg(Region::Nor(0), Reg::Btr),
                    0x0008 => Access::Reg(Region::Nor(1), Reg::Bcr),
                    0x000C => Access::Reg(Region::Nor(1), Reg::Btr),
                    0x0010 => Access::Reg(Region::Nor(2), Reg::Bcr),
                    0x0014 => Access::Reg(Region::Nor(2), Reg::Btr),
                    0x0018 => Access::Reg(Region::Nor(3), Reg::Bcr),
                    0x001C => Access::Reg(Region::Nor(3), Reg::Btr),
                    0x0060 => Access::Reg(Region::Nand(0), Reg::Pcr),
                    0x0068 => Access::Reg(Region::Nand(0), Reg::Pmem),
                    0x0070 => Access::Reg(Region::Nand(0), Reg::Patt),
                    0x0080 => Access::Reg(Region::Nand(1), Reg::Pcr),
                    0x0088 => Access::Reg(Region::Nand(1), Reg::Pmem),
                    0x0090 => Access::Reg(Region::Nand(1), Reg::Patt),
                    0x00A0 => Access::Reg(Region::PcCard, Reg::Pcr),
                    0x00A8 => Access::Reg(Region::PcCard, Reg::Pmem),
                    0x00B0 => Access::Reg(Region::PcCard, Reg::Patt),
                    0x0104 => Access::Reg(Region::Nor(0), Reg::Bwtr),
                    0x010C => Access::Reg(Region::Nor(1), Reg::Bwtr),
                    0x0114 => Access::Reg(Region::Nor(2), Reg::Bwtr),
                    0x011C => Access::Reg(Region::Nor(3), Reg::Bwtr),
                    _ => Access::Reg(Region::Nor(0), Reg::Bcr),
                }
            }
            _ => Access::Reg(Region::Nor(0), Reg::Bcr),
        }
    }

    fn bank(&mut self, r: Region) -> &mut Bank {
        &mut self.banks[r.index()]
    }
}

impl Peripheral for Fsmc {
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        self.read_sized(sys, offset, 4)
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        self.write_sized(sys, offset, 4, value);
    }

    fn read_sized(&mut self, sys: &System, offset: u32, size: u8) -> u32 {
        match Self::access(offset) {
            Access::Data(region, off) => {
                let bank = self.bank(region);
                // NOR banks respect MBKEN (bit 0); NAND/PC-Card banks are always enabled
                let enabled = match region {
                    Region::Nor(_) => bank.bcr & 1 != 0,
                    _ => true,
                };
                if !enabled { return 0; }
                let mut v = 0u32;
                for i in 0..(size as u32) {
                    v |= bank.read_data(sys, off + i) << (i * 8);
                }
                let bank_idx = region.index() as u8 + 1;
                sys.push_event(crate::system::VmEvent::FsmcAccess { bank: bank_idx, offset: off, write: false, size, value: v });
                v
            }
            Access::Reg(region, reg) => {
                let bank = self.bank(region);
                match reg {
                    Reg::Bcr => bank.bcr,
                    Reg::Btr => bank.btr,
                    Reg::Bwtr => bank.bwtr,
                    Reg::Pcr => bank.pcr,
                    Reg::Pmem => bank.pmem,
                    Reg::Patt => bank.patt,
                }
            }
        }
    }

    fn write_sized(&mut self, sys: &System, offset: u32, size: u8, value: u32) {
        match Self::access(offset) {
            Access::Data(region, off) => {
                let bank = self.bank(region);
                let enabled = match region {
                    Region::Nor(_) => bank.bcr & 1 != 0,
                    _ => true,
                };
                // NOR writes also require WREN (bit 1)
                let writable = match region {
                    Region::Nor(_) => enabled && bank.bcr & (1 << 1) != 0,
                    _ => true,
                };
                if !writable { return; }
                for i in 0..(size as u32) {
                    bank.write_data(sys, off + i, (value >> (i * 8)) & 0xFF);
                }
                let bank_idx = region.index() as u8 + 1;
                sys.push_event(crate::system::VmEvent::FsmcAccess { bank: bank_idx, offset: off, write: true, size, value });
            }
            Access::Reg(region, reg) => {
                let bank = self.bank(region);
                match reg {
                    Reg::Bcr => bank.bcr = value,
                    Reg::Btr => bank.btr = value & 0x3FFF_FFFF,
                    Reg::Bwtr => bank.bwtr = value & 0x3FFF_FFFF,
                    Reg::Pcr => bank.pcr = value & 0x3FFF_FFFF,
                    Reg::Pmem => bank.pmem = value & 0x3FFF_FFFF,
                    Reg::Patt => bank.patt = value & 0x3FFF_FFFF,
                }
            }
        }
    }
}
