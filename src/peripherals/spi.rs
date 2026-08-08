use crate::{system::System, ext_devices::{ExtDevice, SpiDeviceEntry, ExtDevices}};
use super::Peripheral;
use crate::peripherals::gpio::{Pin, GpioPorts};
use std::{rc::Rc, cell::RefCell};

#[derive(Default)]
pub struct Spi {
    pub name: String,
    pub cr1: u32,
    pub cr2: u32,
    pub srm: u32,
    pub crcpr: u32,
    pub rxcrcr: u32,
    pub txcrcr: u32,
    pub rx_buffer: u32,
    pub txe: bool,
    pub rxne: bool,
    pub i2s_sr_toggle: bool,
    pub i2scfgr: u32,
    pub i2spr: u32,
    wave_counter: u16,
    devices: Vec<SpiDeviceEntry>,
}

impl Spi {
    pub fn new(name: &str, ext_devices: &ExtDevices, gpio: &mut GpioPorts) -> Option<Box<dyn Peripheral>> {
        if name.starts_with("SPI") {
            let mut devices = ext_devices.find_serial_devices(name);
            if devices.is_empty() {
                if let Some(d) = ext_devices.find_serial_device(name) {
                    let n = d.borrow_mut().connect_peripheral(name);
                    devices.push(SpiDeviceEntry { cs: None, device: d, name: n.clone() });
                }
            } else {
                for d in &mut devices {
                    d.name = d.device.borrow_mut().connect_peripheral(&d.name);
                }
            }
            for d in &devices {
                if let Some((port, pin)) = d.cs {
                    let dev = d.device.clone();
                    gpio.add_write_callback(Pin::new(port, pin), move |sys, v| {
                        dev.borrow_mut().cs_changed(sys, !v);
                    });
                }
            }
            Some(Box::new(Self { name: name.to_string(), devices, txe: true, ..Default::default() }))
        } else { None }
    }

    pub fn is_16bits(&self) -> bool { self.cr1 & (1 << 11) != 0 }
    fn is_i2s(&self) -> bool { self.i2scfgr & 1 != 0 } // I2SMOD

    fn active_device(&self, sys: &System) -> Option<Rc<RefCell<dyn ExtDevice<(), u8>>>> {
        let mut gpio = sys.p.gpio.borrow_mut();
        for d in &self.devices {
            let selected = match d.cs {
                Some((port, pin)) => (gpio.read_port(sys, port) >> pin) & 1 == 0,
                None => true,
            };
            if selected { return Some(d.device.clone()); }
        }
        self.devices.first().map(|d| d.device.clone())
    }

    fn generate_i2s_audio(&mut self) -> u32 {
        let idx = self.wave_counter;
        self.wave_counter = self.wave_counter.wrapping_add(1);
        let phase = idx & 0xFF;
        let sample = if phase < 128 { phase } else { 255 - phase };
        let sample_16 = ((sample as u32) << 7) | (sample as u32);
        if idx & 1 != 0 { sample_16 } else { sample_16 ^ 0x8000 }
    }

    fn fire_interrupts(&mut self, sys: &System) {
        if self.name.starts_with("SPI") && !self.is_i2s() {
            let irq = match self.name.as_str() {
                "SPI1" | "SPI4" => Some(35),
                "SPI2" | "SPI5" => Some(36),
                "SPI3" | "SPI6" => Some(51),
                _ => None,
            };
            if let Some(irq) = irq {
                let txeie = (self.cr2 >> 1) & 1;
                let rxneie = self.cr2 & 1;
                if (txeie != 0 && self.txe) || (rxneie != 0 && self.rxne) {
                    sys.p.nvic.borrow_mut().set_intr_pending(irq);
                }
            }
        }
    }
}

impl Peripheral for Spi {
    fn periph_remap(&self, sys: &System) -> Option<u32> {
        sys.p.afio_remap_status(&self.name)
    }

    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x0000 => self.cr1,
            0x0004 => self.cr2,
            0x0008 => {
                if self.is_i2s() {
                    self.i2s_sr_toggle = !self.i2s_sr_toggle;
                    if self.i2s_sr_toggle { 0b11 } else { 0 }
                } else {
                    let sr = (if self.txe { 2 } else { 0 }) | (if self.rxne { 1 } else { 0 });
                    self.fire_interrupts(sys);
                    sr
                }
            }
            0x000C => {
                if self.is_i2s() {
                    self.generate_i2s_audio()
                } else {
                    let v = self.rx_buffer;
                    self.rx_buffer = 0;
                    self.rxne = false;
                    v
                }
            }
             0x0010 => self.crcpr,
             0x0014 => self.rxcrcr,
             0x0018 => self.txcrcr,
             0x001C => self.i2scfgr,
             0x0020 => self.i2spr,
            _ => 0
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x0000 => self.cr1 = value,
            0x0004 => {
                self.cr2 = value;
                self.fire_interrupts(sys);
            }
            0x000C => {
                if self.is_i2s() {
                    self.rx_buffer = self.generate_i2s_audio();
                } else {
                    self.txe = false;
                    let device = self.active_device(sys);
                    if let Some(ref d) = device {
                        let mut d = d.borrow_mut();
                        if self.is_16bits() {
                            d.write(sys, (), (value >> 8) as u8);
                            self.rx_buffer = (d.read(sys, ()) as u32) << 8;
                            d.write(sys, (), value as u8);
                            self.rx_buffer |= d.read(sys, ()) as u32;
                        } else {
                            let v = value as u8;
                            d.write(sys, (), v);
                            self.rx_buffer = d.read(sys, ()) as u32;
                        }
                    } else {
                        self.rx_buffer = 0xFF;
                    }
                    self.txe = true;
                    self.rxne = true;
                }
            }
             0x0010 => self.crcpr = value,
             0x0014 => self.rxcrcr = value,
             0x0018 => self.txcrcr = value,
             0x001C => self.i2scfgr = value & 0xFFF,
             0x0020 => self.i2spr = value & 0x3FF,
            _ => {}
        }
    }
}
