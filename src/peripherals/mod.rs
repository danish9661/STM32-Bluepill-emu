pub mod rcc;
pub mod spi;
pub mod usart;
pub mod systick;
pub mod gpio;
pub mod dma;
pub mod i2c;
pub mod nvic;
pub mod scb;
pub mod tim;
pub mod adc;
pub mod flash;
pub mod pwr;
pub mod wwdg;
pub mod iwdg;
pub mod rtc;
pub mod crc;
pub mod can;
pub mod fsmc;
pub mod sw_spi;

use std::cell::RefCell;
use std::collections::HashMap;
use crate::system::System;
use crate::ext_devices::ExtDevices;
use fsmc::Fsmc;
use gpio::GpioPorts;
use svd_parser::svd::{MaybeArray, PeripheralInfo};

pub trait Peripheral {
    fn read(&mut self, sys: &System, offset: u32) -> u32;
    fn write(&mut self, sys: &System, offset: u32, value: u32);
    fn tick(&mut self, _sys: &System) {}
    fn rx_byte(&mut self, _sys: &System, _byte: u8) {}
}

pub struct PeripheralSlot<T> {
    pub start: u32,
    pub end: u32,
    pub peripheral: T,
}

pub struct Peripherals {
    pub peripherals: Vec<PeripheralSlot<RefCell<Box<dyn Peripheral>>>>,
    pub nvic: RefCell<nvic::Nvic>,
    pub gpio: RefCell<GpioPorts>,
}

fn extract_svd_max_offset(p: &PeripheralInfo) -> u32 {
    let mut max_off = 0u32;

    use svd_parser::svd::register::{address_offsets as reg_offsets};
    use svd_parser::svd::array::{names as arr_names};
    use svd_parser::svd::cluster::{address_offsets as clus_offsets};

    for reg in p.registers() {
        match reg {
            MaybeArray::Single(r) => max_off = max_off.max(r.address_offset + 4),
            MaybeArray::Array(r, dim) => {
                for (off, _) in reg_offsets(r, dim).zip(arr_names(r, dim)) {
                    max_off = max_off.max(off + 4);
                }
            }
        }
    }

    for cluster in p.clusters() {
        match cluster {
            MaybeArray::Single(c) => {
                let base = c.address_offset;
                for reg in c.registers() {
                    match reg {
                        MaybeArray::Single(r) => max_off = max_off.max(base + r.address_offset + 4),
                        MaybeArray::Array(r, dim) => {
                            for (off, _) in reg_offsets(r, dim).zip(arr_names(r, dim)) {
                                max_off = max_off.max(base + off + 4);
                            }
                        }
                    }
                }
            }
            MaybeArray::Array(c, dim) => {
                for (clus_off, _) in clus_offsets(c, dim).zip(dim.indexes()) {
                    let base = c.address_offset + clus_off as u32;
                    for reg in c.registers() {
                        match reg {
                            MaybeArray::Single(r) => max_off = max_off.max(base + r.address_offset + 4),
                            MaybeArray::Array(r, d) => {
                                for (off, _) in reg_offsets(r, d).zip(arr_names(r, d)) {
                                    max_off = max_off.max(base + off + 4);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    max_off
}

impl Peripherals {
    pub const NVIC_REGS_BASE: u32 = 0xE000_E100;
    pub const NVIC_REGS_END: u32 = 0xE000_E500;

    pub const MEMORY_MAPS: [(u32, u32); 2] = [
        (0x4000_0000, 0xB000_0000),
        (0xE000_0000, 0xE100_0000),
    ];

    pub fn from_svd(svd_xml: &str, gpio: GpioPorts, ext_devices: &ExtDevices) -> Self {
        let mut device: svd_parser::svd::Device = svd_parser::parse(svd_xml)
            .expect("Failed to parse SVD XML");

        device.peripherals.sort_by_key(|p| p.base_address);

        let mut peripherals = Peripherals {
            peripherals: Vec::new(),
            nvic: RefCell::new(nvic::Nvic::default()),
            gpio: RefCell::new(gpio),
        };

        let svd_map: HashMap<&str, &PeripheralInfo> = device.peripherals.iter()
            .filter_map(|p| match p {
                MaybeArray::Single(p) => Some((p.name.as_str(), p)),
                MaybeArray::Array(_, _) => None,
            })
            .collect();

        for p in &device.peripherals {
            let p = match p {
                MaybeArray::Single(p) => p,
                MaybeArray::Array(_, _) => continue,
            };

            let resolved = p.derived_from.as_ref()
                .and_then(|d| svd_map.get(d.as_str()).copied())
                .unwrap_or(p);

            let name = &p.name;
            let size = extract_svd_max_offset(resolved).max(0x10).min(0x400);
            let (start, end) = if name.as_str() == "FSMC" {
                (0x6000_0000, 0xA000_1000)
            } else {
                (p.base_address as u32, p.base_address as u32 + size)
            };
            let peri: Option<Box<dyn Peripheral>> = None
                .or_else(|| nvic::NvicWrapper::new(name))
                .or_else(|| SysTick::new(name))
                .or_else(|| Scb::new(name))
                .or_else(|| Gpio::new(name))
                .or_else(|| Usart::new(name, ext_devices))
                .or_else(|| Rcc::new(name))
                .or_else(|| Flash::new(name))
                .or_else(|| Pwr::new(name))
                .or_else(|| Wwdg::new(name))
                .or_else(|| Iwdg::new(name))
                .or_else(|| Rtc::new(name))
                .or_else(|| Crc::new(name))
                .or_else(|| I2c::new(name, ext_devices))
                .or_else(|| Dma::new(name))
                .or_else(|| Spi::new(name, ext_devices))
                .or_else(|| Timer::new(name))
                .or_else(|| Adc::new(name))
                .or_else(|| Can::new(name))
                .or_else(|| Fsmc::new(name, ext_devices))
            ;

            if let Some(peri) = peri {
                peripherals.peripherals.push(PeripheralSlot {
                    start, end,
                    peripheral: RefCell::new(peri),
                });
            }
        }

        peripherals.finish_registration();
        peripherals
    }

    pub fn new_wasm(gpio: GpioPorts, ext_devices: &ExtDevices) -> Self {
        let mut peripherals = Peripherals {
            peripherals: Vec::new(),
            nvic: RefCell::new(nvic::Nvic::default()),
            gpio: RefCell::new(gpio),
        };

        let mut regs: Vec<(u32, &str)> = vec![
            (0x4000_0000, "TIM2"),  (0x4000_0400, "TIM3"),  (0x4000_0800, "TIM4"),
            (0x4000_2800, "RTC"),  (0x4000_2C00, "WWDG"),  (0x4000_3000, "IWDG"),
            (0x4000_3800, "SPI2"),
            (0x4000_4400, "USART2"), (0x4000_4800, "USART3"),
            (0x4000_5400, "I2C1"), (0x4000_5800, "I2C2"),
            (0x4000_6000, "DMA1"),
            (0x4000_6400, "CAN1"),
            (0x4000_7000, "PWR"),
            (0x4001_0800, "GPIOA"), (0x4001_0C00, "GPIOB"),
            (0x4001_1000, "GPIOC"), (0x4001_1400, "GPIOD"),
            (0x4001_2400, "ADC1"), (0x4001_2800, "ADC2"),
            (0x4001_2C00, "TIM1"),
            (0x4001_3000, "SPI1"),
            (0x4001_3800, "USART1"),
            (0x4002_1000, "RCC"),  (0x4002_2000, "FLASH"),
            (0x4002_3000, "CRC"),
            (0xE000_E000, "NVIC"), (0xE000_E010, "SysTick"), (0xE000_ED00, "SCB"),
        ];
        regs.sort_by_key(|k| k.0);

        for (i, &(base, name)) in regs.iter().enumerate() {
            let size = regs.get(i + 1)
                .map(|&(next, _)| (next - base).min(0x400))
                .unwrap_or(0x100);
            let (start, end) = (base, base + size);

            let p: Option<Box<dyn Peripheral>> = None
                .or_else(|| nvic::NvicWrapper::new(name))
                .or_else(|| SysTick::new(name))
                .or_else(|| Scb::new(name))
                .or_else(|| Gpio::new(name))
                .or_else(|| Usart::new(name, ext_devices))
                .or_else(|| Rcc::new(name))
                .or_else(|| Flash::new(name))
                .or_else(|| Pwr::new(name))
                .or_else(|| Wwdg::new(name))
                .or_else(|| Iwdg::new(name))
                .or_else(|| Rtc::new(name))
                .or_else(|| Crc::new(name))
                .or_else(|| I2c::new(name, ext_devices))
                .or_else(|| Dma::new(name))
                .or_else(|| Spi::new(name, ext_devices))
                .or_else(|| Timer::new(name))
                .or_else(|| Adc::new(name))
                .or_else(|| Can::new(name))
                .or_else(|| Fsmc::new(name, ext_devices))
            ;

            if let Some(p) = p {
                peripherals.peripherals.push(PeripheralSlot { start, end, peripheral: RefCell::new(p) });
            }
        }

        if let Some(p) = Fsmc::new("FSMC", ext_devices) {
            peripherals.peripherals.push(PeripheralSlot { start: 0x6000_0000, end: 0xA000_1000, peripheral: RefCell::new(p) });
        }

        peripherals.finish_registration();
        peripherals
    }

    pub fn register_software_spi(&self, name: &str, cs: Option<String>, clk: &str, miso: &str, mosi: &str, ext_devices: &ExtDevices) {
        let config = sw_spi::SoftwareSpiConfig {
            name: name.to_string(),
            cs,
            clk: clk.to_string(),
            miso: miso.to_string(),
            mosi: mosi.to_string(),
        };
        sw_spi::SoftwareSpi::register(config, &mut self.gpio.borrow_mut(), ext_devices);
    }

    fn finish_registration(&mut self) {
        self.peripherals.sort_by_key(|p| p.start);
        let a = self.peripherals.iter();
        let mut b = self.peripherals.iter();
        b.next();
        for (p1, p2) in a.zip(b) {
            assert!(p1.end <= p2.start, "Overlap: 0x{:08x}-0x{:08x} vs 0x{:08x}-0x{:08x}",
                p1.start, p1.end, p2.start, p2.end);
        }
    }

    fn get_peripheral<T>(slots: &[PeripheralSlot<T>], addr: u32) -> Option<&PeripheralSlot<T>> {
        let index = slots.binary_search_by_key(&addr, |p| p.start)
            .map_or_else(|e| e.checked_sub(1), |v| Some(v));
        index.map(|i| slots.get(i).filter(|p| addr <= p.end)).flatten()
    }

    fn bitbanding(addr: u32) -> Option<(u32, u8)> {
        if (0x4200_0000..0x4400_0000).contains(&addr) {
            let bit_number = (addr % 32) / 4;
            let mapped = 0x4000_0000 + (addr - 0x4200_0000) / 32;
            Some((mapped, bit_number as u8))
        } else { None }
    }

    fn is_register(addr: u32) -> bool { !(0x6000_0000..0xA000_0000).contains(&addr) }

    fn align_addr_4(addr: u32) -> (u32, u8) {
        let byte_offset = (addr % 4) as u8;
        (addr - byte_offset as u32, byte_offset)
    }

    pub fn read(&self, sys: &System, addr: u32, size: u8) -> u32 {
        if let Some((addr, bit_number)) = Self::bitbanding(addr) {
            return (self.read(sys, addr, 1) >> bit_number) & 1;
        }
        let is_reg = Self::is_register(addr);
        let (addr, byte_offset) = if is_reg {
            Self::align_addr_4(addr)
        } else { (addr, 0) };
        let value = if Self::NVIC_REGS_BASE <= addr && addr < Self::NVIC_REGS_END {
            self.nvic.borrow_mut().read(sys, addr - Self::NVIC_REGS_BASE)
        } else if let Some(p) = Self::get_peripheral(&self.peripherals, addr) {
            p.peripheral.borrow_mut().read(sys, addr - p.start)
        } else { 0 };
        if is_reg { value << (8 * byte_offset) } else { value }
    }

    pub fn write(&self, sys: &System, addr: u32, size: u8, mut value: u32) {
        if let Some((addr, bit_number)) = Self::bitbanding(addr) {
            let mut v = self.read(sys, addr, 1);
            v &= !(1 << bit_number);
            v |= (value & 1) << bit_number;
            return self.write(sys, addr, 1, v);
        }
        let (addr, byte_offset) = if Self::is_register(addr) {
            Self::align_addr_4(addr)
        } else { (addr, 0) };
        if byte_offset != 0 && Self::is_register(addr) {
            let v = self.read(sys, addr, 4);
            value = (value << 8 * byte_offset) | (v & (0xFFFF_FFFF >> (32 - 8 * byte_offset)));
        }
        if Self::NVIC_REGS_BASE <= addr && addr < Self::NVIC_REGS_END {
            self.nvic.borrow_mut().write(sys, addr - Self::NVIC_REGS_BASE, value);
        } else if let Some(p) = Self::get_peripheral(&self.peripherals, addr) {
            p.peripheral.borrow_mut().write(sys, addr - p.start, value);
        }
    }

    pub fn rx_byte(&self, sys: &System, addr: u32, byte: u8) -> bool {
        if let Some(p) = Self::get_peripheral(&self.peripherals, addr) {
            p.peripheral.borrow_mut().rx_byte(sys, byte);
            true
        } else { false }
    }

    pub fn addr_desc(&self, addr: u32) -> String {
        format!("addr=0x{:08x}", addr)
    }
}

use spi::Spi;
use usart::Usart;
use systick::SysTick;
use gpio::Gpio;
use dma::Dma;
use i2c::I2c;
use scb::Scb;
use tim::Timer;
use adc::Adc;
use flash::Flash;
use pwr::Pwr;
use wwdg::Wwdg;
use iwdg::Iwdg;
use rtc::Rtc;
use crc::Crc;
use rcc::Rcc;
use can::Can;
