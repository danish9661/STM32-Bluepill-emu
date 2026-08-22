pub mod spi_flash;
pub mod i2c_eeprom;
pub mod usart_probe;
pub mod lcd;
pub mod touchscreen;
pub mod display;
pub mod i2c_oled;
pub mod fsmc_nor;

pub use spi_flash::SpiFlash;
pub use i2c_eeprom::I2cEeprom;
pub use usart_probe::UsartProbe;
pub use lcd::Lcd;
pub use touchscreen::Touchscreen;
pub use display::Display;
pub use i2c_oled::I2cOled;
pub use fsmc_nor::FsmcNor;

use std::{rc::Rc, cell::RefCell};

pub struct SpiDeviceEntry {
    pub cs: Option<(u8, u8)>,
    pub device: Rc<RefCell<dyn ExtDevice<(), u8>>>,
    pub name: String,
}

#[derive(Clone)]
pub struct I2cDeviceEntry {
    pub address: u8,
    pub device: Rc<RefCell<dyn ExtDevice<(), u8>>>,
    pub name: String,
}

#[derive(Default)]
pub struct ExtDevices {
    pub spi_flashes: Vec<Rc<RefCell<SpiFlash>>>,
    pub i2c_eeproms: Vec<Rc<RefCell<I2cEeprom>>>,
    pub usart_probes: Vec<Rc<RefCell<UsartProbe>>>,
    pub lcds: Vec<Rc<RefCell<Lcd>>>,
    pub touchscreens: Vec<Rc<RefCell<Touchscreen>>>,
    pub displays: Vec<Rc<RefCell<Display>>>,
    pub i2c_oleds: Vec<Rc<RefCell<I2cOled>>>,
    pub fsmc_nors: Vec<Rc<RefCell<FsmcNor>>>,
}

impl ExtDevices {
    pub fn find_serial_devices(&self, peri_name: &str) -> Vec<SpiDeviceEntry> {
        let mut result: Vec<SpiDeviceEntry> = Vec::new();
        for d in &self.spi_flashes {
            if d.borrow().config.peripheral == peri_name {
                result.push(SpiDeviceEntry {
                    cs: d.borrow().config.cs.as_ref().and_then(|s| parse_pin(s)),
                    device: d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>,
                    name: format!("{} spi-flash", peri_name),
                });
            }
        }
        for d in &self.usart_probes {
            if d.borrow().config.peripheral == peri_name {
                result.push(SpiDeviceEntry {
                    cs: None,
                    device: d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>,
                    name: format!("{} usart-probe", peri_name),
                });
            }
        }
        for d in &self.lcds {
            if d.borrow().config.peripheral == peri_name {
                result.push(SpiDeviceEntry {
                    cs: d.borrow().config.cs.as_ref().and_then(|s| parse_pin(s)),
                    device: d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>,
                    name: format!("{} lcd", peri_name),
                });
            }
        }
        for d in &self.touchscreens {
            if d.borrow().config.peripheral == peri_name {
                result.push(SpiDeviceEntry {
                    cs: d.borrow().config.cs.as_ref().and_then(|s| parse_pin(s)),
                    device: d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>,
                    name: format!("{} touchscreen", peri_name),
                });
            }
        }
        result
    }

    pub fn find_serial_device(&self, peri_name: &str) -> Option<Rc<RefCell<dyn ExtDevice<(), u8>>>> {
        self.spi_flashes.iter()
            .filter(|d| d.borrow().config.peripheral == peri_name)
            .next()
            .map(|d| d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>)
        .or_else(||
        self.usart_probes.iter()
            .filter(|d| d.borrow().config.peripheral == peri_name)
            .next()
            .map(|d| d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>))
        .or_else(||
        self.lcds.iter()
            .filter(|d| d.borrow().config.peripheral == peri_name)
            .next()
            .map(|d| d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>))
        .or_else(||
        self.touchscreens.iter()
            .filter(|d| d.borrow().config.peripheral == peri_name)
            .next()
            .map(|d| d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>))
    }

    pub fn find_i2c_devices(&self, peri_name: &str) -> Vec<I2cDeviceEntry> {
        let mut result: Vec<I2cDeviceEntry> = Vec::new();
        for d in &self.i2c_eeproms {
            if d.borrow().config.peripheral == peri_name {
                result.push(I2cDeviceEntry {
                    address: d.borrow().config.address,
                    device: d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>,
                    name: format!("{} i2c-eeprom", peri_name),
                });
            }
        }
        for d in &self.i2c_oleds {
            if d.borrow().config.peripheral == peri_name {
                result.push(I2cDeviceEntry {
                    address: d.borrow().config.address,
                    device: d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>,
                    name: format!("{} i2c-oled", peri_name),
                });
            }
        }
        result
    }

    pub fn find_mem_device(&self, peri_name: &str) -> Option<Rc<RefCell<dyn ExtDevice<u32, u32>>>> {
        self.displays.iter()
            .filter(|d| d.borrow().config.peripheral == peri_name)
            .next()
            .map(|d| d.clone() as Rc<RefCell<dyn ExtDevice<u32, u32>>>)
        .or_else(||
        self.fsmc_nors.iter()
            .filter(|d| d.borrow().name() == peri_name)
            .next()
            .map(|d| d.clone() as Rc<RefCell<dyn ExtDevice<u32, u32>>>))
    }
}

pub trait ExtDevice<A, T> {
    fn connect_peripheral(&mut self, peri_name: &str) -> String;
    fn read(&mut self, sys: &crate::system::System, addr: A) -> T;
    fn write(&mut self, sys: &crate::system::System, addr: A, v: T);
    fn reset(&mut self) {}
    /// Called when the device's CS pin changes state (true = selected/CS low).
    fn cs_changed(&mut self, _sys: &crate::system::System, _selected: bool) {}
}

// SAFETY: WASM is single-threaded; Rc/RefCell are safe
unsafe impl Send for ExtDevices {}
unsafe impl Sync for ExtDevices {}

fn parse_pin(s: &str) -> Option<(u8, u8)> {
    crate::peripherals::gpio::parse_pin_name(s)
}
