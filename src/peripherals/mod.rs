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
pub mod afio;
pub mod exti;
pub mod bkp;
pub mod dac;
pub mod usb;

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
    /// Width-aware read (memory-mapped devices like FSMC need the access size
    /// to assemble bytes); defaults to plain read().
    fn read_sized(&mut self, sys: &System, offset: u32, _size: u8) -> u32 {
        self.read(sys, offset)
    }
    /// Width-aware write; defaults to plain write().
    fn write_sized(&mut self, sys: &System, offset: u32, _size: u8, value: u32) {
        self.write(sys, offset, value);
    }
    fn rx_byte(&mut self, _sys: &System, _byte: u8) {}
    fn rx_pending(&self) -> u32 { 0 }
    fn can_inject_message(&mut self, _sys: &System, _tir: u32, _tdtr: u32, _tdlr: u32, _tdhr: u32) -> bool { false }
    /// Returns the GPIO port letter for the given EXTI line, if this is AFIO.
    fn exti_port(&self, _line: u32) -> Option<char> { None }
    /// Called by GPIO when a pin changes state. Returns true if handled.
    fn gpio_pin_changed(&mut self, _sys: &System, _port: u8, _pin: u8, _rising: bool) -> bool { false }
    /// Returns AFIO MAPR remap bits for this peripheral, if applicable.
    fn periph_remap(&self, _sys: &System) -> Option<u32> { None }
    /// Returns the MAPR remap bits for a named peripheral (only AFIO implements meaningfully).
    fn remap_status(&self, _name: &str) -> Option<u32> { None }
    /// Returns the current PWM duty (0-100) of a timer channel, if this is a timer.
    fn pwm_duty(&self, _channel: u32) -> Option<u32> { None }
    /// Peripheral DMA request (e.g. ADC end-of-conversion): triggers a configured
    /// DMA channel transfer if it is enabled.
    fn dma_request(&mut self, _sys: &System, _channel: u32) {}
    /// True when the CPU is in a deep-sleep mode (STOP/STANDBY), gating peripheral ticks.
    fn in_deep_sleep(&self) -> bool { false }
    /// Called instead of tick() while the peripheral is frozen in STOP/STANDBY.
    /// Instruction-delta peripherals must advance their delta base here without
    /// processing state, so they don't catch up when the CPU wakes.
    fn tick_frozen(&mut self, _sys: &System) {}
    /// Raise a fault (kind: 0=fetch, 1=read, 2=write, 3=undef instruction), setting
    /// SCB fault status registers and pending the fault exception with escalation.
    fn raise_fault(&mut self, _sys: &System, _kind: u32, _addr: u32) {}
}

pub struct PeripheralSlot<T> {
    pub start: u32,
    pub end: u32,
    pub peripheral: T,
}

pub struct Peripherals {
    pub peripherals: Vec<PeripheralSlot<RefCell<Box<dyn Peripheral>>>>,
    pub tick_indices: Vec<usize>,
    pub nvic: RefCell<nvic::Nvic>,
    pub gpio: RefCell<GpioPorts>,
    rcc_enrs: RefCell<(u32, u32, u32)>,
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

fn name_has_tick(name: &str) -> bool {
    name.starts_with("TIM") || name.starts_with("DMA") || name == "RTC" || name.starts_with("ADC")
        || name.starts_with("USART") || name.starts_with("UART")
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

        let rcc_enrs = RefCell::new((0x0000_0014, 0x0000_0000, 0x0000_0000));
        let mut peripherals = Peripherals {
            peripherals: Vec::new(),
            tick_indices: Vec::new(),
            nvic: RefCell::new(nvic::Nvic::default()),
            gpio: RefCell::new(gpio),
            rcc_enrs,
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
                .or_else(|| Spi::new(name, ext_devices, &mut peripherals.gpio.borrow_mut()))
                .or_else(|| Timer::new(name))
                .or_else(|| Adc::new(name))
                .or_else(|| Can::new(name))
                .or_else(|| Usb::new(name))
                .or_else(|| Fsmc::new(name, ext_devices))
                .or_else(|| Afio::new(name))
                .or_else(|| Exti::new(name))
                .or_else(|| Bkp::new(name))
                .or_else(|| Dac::new(name))
            ;

            if let Some(peri) = peri {
                let idx = peripherals.peripherals.len();
                peripherals.peripherals.push(PeripheralSlot {
                    start, end,
                    peripheral: RefCell::new(peri),
                });
                if name_has_tick(name) {
                    peripherals.tick_indices.push(idx);
                }
            }
        }

        peripherals.finish_registration();
        peripherals
    }

    pub fn new_wasm(gpio: GpioPorts, ext_devices: &ExtDevices) -> Self {
        let rcc_enrs = RefCell::new((0x0000_0014, 0x0000_0000, 0x0000_0000));
        let mut peripherals = Peripherals {
            peripherals: Vec::new(),
            tick_indices: Vec::new(),
            nvic: RefCell::new(nvic::Nvic::default()),
            gpio: RefCell::new(gpio),
            rcc_enrs,
        };

        let mut regs: Vec<(u32, &str)> = vec![
            (0x4000_0000, "TIM2"),  (0x4000_0400, "TIM3"),  (0x4000_0800, "TIM4"),
            (0x4000_1000, "TIM6"),  (0x4000_1400, "TIM7"),
            (0x4000_2800, "RTC"),  (0x4000_2C00, "WWDG"),  (0x4000_3000, "IWDG"),
            (0x4000_3800, "SPI2"),
            (0x4000_4400, "USART2"), (0x4000_4800, "USART3"),
            (0x4000_5400, "I2C1"), (0x4000_5800, "I2C2"),
            (0x4000_6000, "DMA1"),
            (0x4000_6400, "CAN1"),
            (0x4000_5C00, "USB"),
            (0x4000_6C00, "BKP"),
            (0x4000_7000, "PWR"),
            (0x4000_7400, "DAC"),
            (0x4001_0000, "AFIO"), (0x4001_0400, "EXTI"),
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
                .or_else(|| Spi::new(name, ext_devices, &mut peripherals.gpio.borrow_mut()))
                .or_else(|| Timer::new(name))
                .or_else(|| Adc::new(name))
                .or_else(|| Can::new(name))
                .or_else(|| Usb::new(name))
                .or_else(|| Fsmc::new(name, ext_devices))
                .or_else(|| Afio::new(name))
                .or_else(|| Exti::new(name))
                .or_else(|| Bkp::new(name))
                .or_else(|| Dac::new(name))
            ;

            if let Some(p) = p {
                let idx = peripherals.peripherals.len();
                peripherals.peripherals.push(PeripheralSlot { start, end, peripheral: RefCell::new(p) });
                if name_has_tick(name) {
                    peripherals.tick_indices.push(idx);
                }
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

    fn nvic_priority_check(addr: u32) -> bool {
        const NVIC_PRIO_BASE: u32 = 0xE000E300;
        addr >= NVIC_PRIO_BASE && addr < NVIC_PRIO_BASE + 0x100
    }

    fn clock_enabled(&self, base_addr: u32) -> bool {
        let (ahbenr, apb2enr, apb1enr) = *self.rcc_enrs.borrow();
        if base_addr >= 0xE000_0000 { return true; }
        if base_addr >= 0x4002_1000 && base_addr < 0x4002_2000 { return true; }
        match base_addr {
            0x4002_0000 => (ahbenr & 1) != 0,
            0x4002_2000 => (ahbenr & (1 << 4)) != 0,
            0x4002_3000 => (ahbenr & (1 << 6)) != 0,
            0x4001_0000 | 0x4001_0400 => (apb2enr & 1) != 0,
            0x4001_0800 => (apb2enr & (1 << 2)) != 0,
            0x4001_0C00 => (apb2enr & (1 << 3)) != 0,
            0x4001_1000 => (apb2enr & (1 << 4)) != 0,
            0x4001_1400 => (apb2enr & (1 << 5)) != 0,
            0x4001_2400 => (apb2enr & (1 << 9)) != 0,
            0x4001_2800 => (apb2enr & (1 << 10)) != 0,
            0x4001_2C00 => (apb2enr & (1 << 11)) != 0,
            0x4001_3000 => (apb2enr & (1 << 12)) != 0,
            0x4001_3800 => (apb2enr & (1 << 14)) != 0,
            0x4000_0000 => (apb1enr & 1) != 0,
            0x4000_0400 => (apb1enr & (1 << 1)) != 0,
            0x4000_0800 => (apb1enr & (1 << 2)) != 0,
            0x4000_1000 | 0x4000_1400 => true,
            0x4000_2800 => (apb1enr & (1 << 9)) != 0,
            0x4000_2C00 => (apb1enr & (1 << 11)) != 0,
            0x4000_3000 => true,
            0x4000_3800 => (apb1enr & (1 << 14)) != 0,
            0x4000_4400 => (apb1enr & (1 << 17)) != 0,
            0x4000_4800 => (apb1enr & (1 << 18)) != 0,
            0x4000_5400 => (apb1enr & (1 << 21)) != 0,
            0x4000_5800 => (apb1enr & (1 << 22)) != 0,
            0x4000_6400 => (apb1enr & (1 << 25)) != 0,
            0x4000_6C00 => (apb1enr & (1 << 27)) != 0,
            0x4000_7000 => (apb1enr & (1 << 28)) != 0,
            0x4000_7400 => (apb1enr & (1 << 29)) != 0,
            _ => true,
        }
    }

    fn update_rcc_enrs(&self, offset: u32, value: u32) {
        let mut enrs = self.rcc_enrs.borrow_mut();
        match offset {
            0x14 => { enrs.0 = value; }
            0x18 => { enrs.1 = value; }
            0x1C => { enrs.2 = value; }
            _ => {},
        }
    }

    /// PWM duty (0-100) of a timer channel, 0 if the address is not a timer.
    pub fn pwm_duty(&self, addr: u32, channel: u32) -> u32 {
        if let Some(p) = Self::get_peripheral(&self.peripherals, addr) {
            if let Ok(t) = p.peripheral.try_borrow() {
                return t.pwm_duty(channel).unwrap_or(0);
            }
        }
        0
    }

    pub fn read(&self, sys: &System, addr: u32, size: u8) -> u32 {
        if let Some((addr, bit_number)) = Self::bitbanding(addr) {
            return (self.read(sys, addr, 1) >> bit_number) & 1;
        }
        // NVIC priority registers are byte-addressable, bypass alignment
        if Self::nvic_priority_check(addr) {
            return self.nvic.borrow_mut().read(sys, addr - Self::NVIC_REGS_BASE);
        }
        let is_reg = Self::is_register(addr);
        let (addr, byte_offset) = if is_reg {
            Self::align_addr_4(addr)
        } else { (addr, 0) };
        let value = if Self::NVIC_REGS_BASE <= addr && addr < Self::NVIC_REGS_END {
            self.nvic.borrow_mut().read(sys, addr - Self::NVIC_REGS_BASE)
        } else if let Some(p) = Self::get_peripheral(&self.peripherals, addr) {
            p.peripheral.borrow_mut().read_sized(sys, addr - p.start, size)
        } else { 0 };
        if is_reg { value << (8 * byte_offset) } else { value }
    }

    pub fn write(&self, sys: &System, addr: u32, _size: u8, mut value: u32) {
        if let Some((addr, bit_number)) = Self::bitbanding(addr) {
            let mut v = self.read(sys, addr, 1);
            v &= !(1 << bit_number);
            v |= (value & 1) << bit_number;
            return self.write(sys, addr, 1, v);
        }
        // NVIC priority registers are byte-addressable, bypass alignment
        if Self::nvic_priority_check(addr) {
            self.nvic.borrow_mut().write(sys, addr - Self::NVIC_REGS_BASE, value);
            return;
        }
        let (addr, byte_offset) = if Self::is_register(addr) {
            Self::align_addr_4(addr)
        } else { (addr, 0) };
        if byte_offset != 0 && Self::is_register(addr) {
            let v = self.read(sys, addr, 4);
            value = (value << 8 * byte_offset) | (v & (0xFFFF_FFFF >> (32 - 8 * byte_offset)));
        }
        // Always allow writes to RCC (0x40021000-0x40021FFF)
        if addr >= 0x4002_1000 && addr < 0x4002_2000 {
            self.update_rcc_enrs(addr - 0x4002_1000, value);
        }
        if Self::NVIC_REGS_BASE <= addr && addr < Self::NVIC_REGS_END {
            self.nvic.borrow_mut().write(sys, addr - Self::NVIC_REGS_BASE, value);
        } else if let Some(p) = Self::get_peripheral(&self.peripherals, addr) {
            p.peripheral.borrow_mut().write_sized(sys, addr - p.start, _size, value);
        }
    }

    pub fn can_inject_message(&self, sys: &System, addr: u32, tir: u32, tdtr: u32, tdlr: u32, tdhr: u32) -> bool {
        if let Some(p) = Self::get_peripheral(&self.peripherals, addr) {
            p.peripheral.borrow_mut().can_inject_message(sys, tir, tdtr, tdlr, tdhr)
        } else { false }
    }

    pub fn rx_byte(&self, sys: &System, addr: u32, byte: u8) -> bool {
        if let Some(p) = Self::get_peripheral(&self.peripherals, addr) {
            p.peripheral.borrow_mut().rx_byte(sys, byte);
            true
        } else { false }
    }

    pub fn rx_pending(&self, addr: u32) -> u32 {
        if let Some(p) = Self::get_peripheral(&self.peripherals, addr) {
            p.peripheral.borrow().rx_pending()
        } else { 0 }
    }

    /// Queries the AFIO peripheral for the GPIO port mapped to a given EXTI line (0-15).
    pub fn exti_port_for_line(&self, line: u32) -> Option<char> {        for slot in &self.peripherals {
            if slot.start == 0x4001_0000 {
                return slot.peripheral.borrow().exti_port(line);
            }
        }
        None
    }

    /// Queries AFIO MAPR remap bits for a given peripheral name.
    /// Returns None if the peripheral has no remap bits or AFIO is unavailable.
    pub fn afio_remap_status(&self, name: &str) -> Option<u32> {
        for slot in &self.peripherals {
            if slot.start == 0x4001_0000 {
                return slot.peripheral.borrow().remap_status(name);
            }
        }
        None
    }

    /// Called from GPIO when a pin changes state. Triggers EXTI if the port/pin
    /// matches the AFIO EXTICR mapping and EXTI edge configuration.
    pub fn gpio_exti_trigger(&self, sys: &System, port: u8, pin: u8, rising: bool) {
        for slot in &self.peripherals {
            if slot.start == 0x4001_0400 {
                slot.peripheral.borrow_mut().gpio_pin_changed(sys, port, pin, rising);
                return;
            }
        }
    }

    /// Peripheral DMA request: fires the enabled DMA channel if configured.
    pub fn dma_request(&self, sys: &System, channel: u32) {
        if let Some(p) = Self::get_peripheral(&self.peripherals, 0x4000_6000) {
            p.peripheral.borrow_mut().dma_request(sys, channel);
        }
    }

    /// STOP/STANDBY detection: SCB SCR SLEEPDEEP (bit 2). In deep sleep the core is
    /// halted; only LSI/LSE-clocked peripherals (IWDG, RTC) keep running.
    pub fn in_deep_sleep(&self) -> bool {
        if let Some(p) = Self::get_peripheral(&self.peripherals, 0xE000_ED00) {
            if let Ok(s) = p.peripheral.try_borrow() {
                return s.in_deep_sleep();
            }
        }
        false
    }

    /// Raise a fault through the SCB (CFSR/HFSR/BFAR + NVIC escalation).
    pub fn raise_fault(&self, sys: &System, kind: u32, addr: u32) {
        if let Some(p) = Self::get_peripheral(&self.peripherals, 0xE000_ED00) {
            p.peripheral.borrow_mut().raise_fault(sys, kind, addr);
        }
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
use afio::Afio;
use exti::Exti;
use bkp::Bkp;
use dac::Dac;
use usb::Usb;
