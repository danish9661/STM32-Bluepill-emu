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
    /// CRC error latch (SR bit 4): set on CRCNEXT-phase mismatch, cleared
    /// on the next DR read (documented choice; HW says "cleared by SW").
    pub crcerr: bool,
    pub i2s_sr_toggle: bool,
    pub i2scfgr: u32,
    pub i2spr: u32,
    wave_counter: u16,
    devices: Vec<SpiDeviceEntry>,
    /// DMA channel: 0=none, 2/3=SPI1_RX/TX, 4/5=SPI2_RX/TX
    dma_channel_tx: u8,
    dma_channel_rx: u8,
    /// Last driven NSS-output level (master, SSOE). Idle high; only
    /// transitions emit pin events.
    nss_level: bool,
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
            Some(Box::new(Self {
                name: name.to_string(),
                devices,
                txe: true,
                // DMA channels: 1-7 = DMA1, 8-12 = DMA2 (offset by 8)
                dma_channel_tx: match name {
                    "SPI1" => 3, "SPI2" => 5,
                    "SPI3" => 10, // DMA2 ch2
                    _ => 0
                },
                dma_channel_rx: match name {
                    "SPI1" => 2, "SPI2" => 4,
                    "SPI3" => 11, // DMA2 ch3
                    _ => 0
                },
                nss_level: true, // NSS output idles high
                ..Default::default()
            }))
        } else { None }
    }

    pub fn is_16bits(&self) -> bool { self.cr1 & (1 << 11) != 0 }

    fn crc_enabled(&self) -> bool { self.cr1 & (1 << 13) != 0 }
    fn crc_next(&self) -> bool { self.cr1 & (1 << 12) != 0 }

    fn spi_channel(&self) -> u8 {
        self.name.trim_start_matches("SPI").parse::<u8>().unwrap_or(0)
    }
    /// TI frame format (SPI_CR2 FRF, bit 4): NSS pulses once per frame and
    /// CPOL/CPHA are don't-care. Transfers complete synchronously with no
    /// edge surface, so the shifted bit content is identical to Motorola
    /// mode — decoded here so the mode is explicit and pinned by test.
    fn is_ti_mode(&self) -> bool { self.cr2 & (1 << 4) != 0 }
    fn is_i2s(&self) -> bool { self.i2scfgr & 1 != 0 } // I2SMOD

    /// Default NSS-output pin (master, SSOE): SPI1→PA4, SPI2→PB12,
    /// SPI3→PA15. (SPI3 AFIO-remapped NSS on PA4 is not tracked.)
    fn nss_pin(&self) -> Option<(u8, u8)> {
        match self.name.as_str() {
            "SPI1" => Some((0, 4)),
            "SPI2" => Some((1, 12)),
            "SPI3" => Some((0, 15)),
            _ => None,
        }
    }

    /// NSS hardware output (CR2 SSOE, bit 2): driven low while the
    /// peripheral is enabled, released high otherwise. Only transitions
    /// emit pin events. Slave-mode NSS input (SSI/MODF, multimaster only)
    /// is not modeled.
    fn update_nss(&mut self, sys: &System) {
        let want_high = !((self.cr2 & (1 << 2)) != 0 && (self.cr1 & (1 << 6)) != 0);
        if want_high == self.nss_level {
            return;
        }
        self.nss_level = want_high;
        if let Some((port, pin)) = self.nss_pin() {
            sys.p.gpio.borrow_mut().write_port(sys, port, pin, want_high, true);
            sys.p.gpio_exti_trigger(sys, port, pin, want_high);
        }
    }

    fn active_device(&self, sys: &System) -> Option<Rc<RefCell<dyn ExtDevice<(), u8>>>> {
        let mut gpio = sys.p.gpio.borrow_mut();
        for d in &self.devices {
            let selected = match d.cs {
                Some((port, pin)) => !gpio.read_pin_effective(sys, port, pin),
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

/// STM32 SPI CRC update, MSB-first, no reflection: 8-bit frames use
/// poly[7:0] with init 0xFF, 16-bit frames poly[15:0] with init 0xFFFF
/// (CRCPR reset 0x0007). Returns the updated running value (masked).
fn spi_crc_update(mut crc: u32, bytes: &[u8], poly: u32, bits: u32) -> u32 {
    let mask = if bits >= 32 { 0xFFFF_FFFF } else { (1u32 << bits) - 1 };
    for &b in bytes {
        for i in (0..8).rev() {
            let bit = (b as u32 >> i) & 1;
            let msb = (crc >> (bits - 1)) & 1;
            crc = ((crc << 1) ^ if msb ^ bit != 0 { poly } else { 0 }) & mask;
        }
    }
    crc & mask
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
                    let sr = (if self.txe { 2 } else { 0 }) | (if self.rxne { 1 } else { 0 })
                        | (if self.crcerr { 1 << 4 } else { 0 });
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
                    self.crcerr = false; // CRCERR clears on DR read
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
            0x0000 => {
                // CRCEN 0->1 resets both calculators to all-ones.
                if value & (1 << 13) != 0 && self.cr1 & (1 << 13) == 0 {
                    self.txcrcr = 0xFFFF;
                    self.rxcrcr = 0xFFFF;
                    self.crcerr = false;
                }
                self.cr1 = value;
                self.update_nss(sys);
            }
            0x0004 => {
                self.cr2 = value;
                self.update_nss(sys);
                self.fire_interrupts(sys);
            }
            0x000C => {
                if self.is_i2s() {
                    self.rx_buffer = self.generate_i2s_audio();
                } else {
                    self.txe = false;
                    let channel = self.spi_channel();
                    let device = self.active_device(sys);
                    // TI mode changes only NSS phasing (no edge surface
                    // here); the data path below is shared with Motorola
                    // mode by construction, CPOL/CPHA unread in both.
                    let _ti_frame = self.is_ti_mode();
                    let crc_phase = self.crc_enabled() && self.crc_next();
                    // In a CRCNEXT phase the shifter clocks out TXCRC
                    // instead of data (per DFF width, MSB first).
                    let tx_word = if crc_phase {
                        if self.is_16bits() { self.txcrcr & 0xFFFF } else { self.txcrcr & 0xFF }
                    } else {
                        value
                    };
                    if let Some(ref d) = device {
                        let mut d = d.borrow_mut();
                        if self.is_16bits() {
                            d.write(sys, (), (tx_word >> 8) as u8);
                            let rb_hi = sys.spi_take_miso(channel).unwrap_or_else(|| d.read(sys, ()) as u8);
                            self.rx_buffer = (rb_hi as u32) << 8;
                            d.write(sys, (), tx_word as u8);
                            let rb_lo = sys.spi_take_miso(channel).unwrap_or_else(|| d.read(sys, ()) as u8);
                            self.rx_buffer |= rb_lo as u32;
                        } else {
                            let v = tx_word as u8;
                            d.write(sys, (), v);
                            let rb = sys.spi_take_miso(channel).unwrap_or_else(|| d.read(sys, ()) as u8);
                            self.rx_buffer = rb as u32;
                        }
                    } else {
                        self.rx_buffer = 0xFF;
                    }
                    // CRC calculators (RM0008 §25.3.7): normal transfers
                    // feed TX then RX; the CRCNEXT transfer compares the
                    // received check word against RXCRC (no accumulate)
                    // and re-arms both calculators.
                    if self.crc_enabled() {
                        let bits = if self.is_16bits() { 16 } else { 8 };
                        let poly = self.crcpr & if self.is_16bits() { 0xFFFF } else { 0xFF };
                        if crc_phase {
                            let rxw = self.rx_buffer & if self.is_16bits() { 0xFFFF } else { 0xFF };
                            let expect = self.rxcrcr & if self.is_16bits() { 0xFFFF } else { 0xFF };
                            if rxw != expect {
                                self.crcerr = true;
                            }
                            self.txcrcr = 0xFFFF;
                            self.rxcrcr = 0xFFFF;
                        } else {
                            // 16-bit frames feed the whole word MSB-first.
                            if self.is_16bits() {
                                let txb = [(tx_word >> 8) as u8, tx_word as u8];
                                self.txcrcr = spi_crc_update(self.txcrcr, &txb, poly, bits);
                                let rxb = [(self.rx_buffer >> 8) as u8, self.rx_buffer as u8];
                                self.rxcrcr = spi_crc_update(self.rxcrcr, &rxb, poly, bits);
                            } else {
                                let txb = [tx_word as u8];
                                self.txcrcr = spi_crc_update(self.txcrcr, &txb, poly, bits);
                                let rxb = [self.rx_buffer as u8];
                                self.rxcrcr = spi_crc_update(self.rxcrcr, &rxb, poly, bits);
                            }
                        }
                    }
                    self.txe = true;
                    self.rxne = true;
                    // Fire DMA requests when TXDMAEN/RXDMAEN are set
                    if self.cr2 & (1 << 1) != 0 && self.dma_channel_tx != 0 {
                        sys.p.dma_request(sys, self.dma_channel_tx as u32);
                    }
                    if self.cr2 & 1 != 0 && self.dma_channel_rx != 0 {
                        sys.p.dma_request(sys, self.dma_channel_rx as u32);
                    }
                    // Transaction-level event for virtual peripherals.
                    let (tx_bytes, rx_bytes) = if self.is_16bits() {
                        (vec![(value >> 8) as u8, value as u8],
                         vec![(self.rx_buffer >> 8) as u8, self.rx_buffer as u8])
                    } else {
                        (vec![value as u8], vec![self.rx_buffer as u8])
                    };
                    sys.push_event(crate::system::VmEvent::SpiTransfer { channel, tx: tx_bytes, rx: rx_bytes });
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
