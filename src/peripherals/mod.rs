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
pub mod sdio;

use std::cell::RefCell;
use std::collections::HashMap;
use crate::system::System;
use crate::ext_devices::ExtDevices;
use crate::bus::{Bus, JsPeripheral};
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
    /// ADC external trigger from a timer source: (timer base, channel index,
    /// 4 = TRGO update). ADC checks its own EXTSEL configuration.
    fn adc_timer_trigger(&mut self, _sys: &System, _tim_base: u32, _ch: u8) {}
    /// ADC external trigger from an EXTI line (regular: 11, injected: 15).
    fn adc_exti_trigger(&mut self, _sys: &System, _line: u32) {}
    /// 12-bit voltage a peripheral drives on a GPIO pin (DAC output), if any.
    fn dac_output(&self, _port: u8, _pin: u8) -> Option<u32> { None }
    /// True when the CPU is in a deep-sleep mode (STOP/STANDBY), gating peripheral ticks.
    fn in_deep_sleep(&self) -> bool { false }
    /// Called instead of tick() while the peripheral is frozen in STOP/STANDBY.
    /// Instruction-delta peripherals must advance their delta base here without
    /// processing state, so they don't catch up when the CPU wakes.
    fn tick_frozen(&mut self, _sys: &System) {}
    /// Whether the peripheral is currently enabled/running (e.g. TIM CEN bit).
    fn is_enabled(&self) -> bool { false }
    /// Raise a fault (kind: 0=fetch, 1=read, 2=write, 3=undef instruction), setting
    /// SCB fault status registers and pending the fault exception with escalation.
    fn raise_fault(&mut self, _sys: &System, _kind: u32, _addr: u32) {}
}

pub struct PeripheralSlot<T> {
    pub start: u32,
    pub end: u32,
    pub tick: bool,
    pub peripheral: T,
}

pub struct Peripherals {
    pub bus: RefCell<Bus>,
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
            bus: RefCell::new(Bus::new()),
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
            let peri: Option<Box<dyn Peripheral>> =
                Self::build_peripheral(name, ext_devices, &mut peripherals.gpio.borrow_mut());

            if let Some(peri) = peri {
                peripherals.bus.get_mut().register(start, end, name_has_tick(name), peri);
            }
        }

        // ARM core peripherals are architectural: fixed addresses on every
        // Cortex-M3. Some SVDs omit them (e.g. STM32F105xx has no SCB/SysTick) —
        // register defaults so SysTick, faults and deep-sleep always work.
        for (name, base, size) in [
            ("NVIC", 0xE000_E000u32, 0x100u32),
            ("SysTick", 0xE000_E010u32, 0x20u32),
            ("SCB", 0xE000_ED00u32, 0x100u32),
        ] {
            if peripherals.bus.get_mut().get(base).is_none() {
                if let Some(p) = Self::build_peripheral(name, ext_devices, &mut peripherals.gpio.borrow_mut()) {
                    peripherals.bus.get_mut().register(base, base + size, false, p);
                }
            }
        }

        peripherals.bus.get_mut().finish_assert_no_overlap();
        peripherals
    }

    /// Construct a peripheral implementation from its SVD peripheral name.
    /// Returns None for peripherals this emulator doesn't model (ETH, USB-OTG
    /// on some chips, ...) — those are silently skipped.
    fn build_peripheral(name: &str, ext_devices: &ExtDevices, gpio: &mut GpioPorts) -> Option<Box<dyn Peripheral>> {
        None
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
            .or_else(|| Spi::new(name, ext_devices, gpio))
            .or_else(|| Timer::new(name))
            .or_else(|| Adc::new(name))
            .or_else(|| Can::new(name))
            .or_else(|| Usb::new(name))
            .or_else(|| Fsmc::new(name, ext_devices))
            .or_else(|| Afio::new(name))
            .or_else(|| Exti::new(name))
            .or_else(|| Bkp::new(name))
            .or_else(|| Dac::new(name))
            .or_else(|| Sdio::new(name))
    }

    pub fn new_wasm(gpio: GpioPorts, ext_devices: &ExtDevices) -> Self {
        let rcc_enrs = RefCell::new((0x0000_0014, 0x0000_0000, 0x0000_0000));
        let mut peripherals = Peripherals {
            bus: RefCell::new(Bus::new()),
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
            (0x4000_5C00, "USB"),
            (0x4000_6400, "CAN1"),
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
            (0x4001_8000, "SDIO"),
            (0x4002_0000, "DMA1"), (0x4002_0400, "DMA2"),
            (0x4002_1000, "RCC"),  (0x4002_2000, "FLASH"),
            (0x4002_3000, "CRC"),
            (0xE000_E000, "NVIC"), (0xE000_E010, "SysTick"), (0xE000_ED00, "SCB"),
        ];
        regs.sort_by_key(|k| k.0);

        for (i, &(base, name)) in regs.iter().enumerate() {
            let size = regs.get(i + 1)
                .map(|&(next, _)| (next - base).min(0x400))
                .unwrap_or(0x100);

            let p: Option<Box<dyn Peripheral>> =
                Self::build_peripheral(name, ext_devices, &mut peripherals.gpio.borrow_mut());

            if let Some(p) = p {
                peripherals.bus.get_mut().register(base, base + size, name_has_tick(name), p);
            }
        }

        if let Some(p) = Fsmc::new("FSMC", ext_devices) {
            peripherals.bus.get_mut().register(0x6000_0000, 0xA000_1000, false, p);
        }

        peripherals.bus.get_mut().finish_assert_no_overlap();
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

    /// Register a peripheral implemented in JS (read/write callbacks receive
    /// the absolute address + access width). Last registration wins on
    /// overlap, so custom peripherals can shadow built-ins.
    pub fn register_js(&self, base: u32, size: u32, read: js_sys::Function, write: js_sys::Function) {
        self.bus.borrow_mut().register(
            base, base + size, false,
            Box::new(JsPeripheral::new(base, read, write)),
        );
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

    /// Mask of the `size` low-order bytes (size >= 4 → all 32 bits).
    fn width_mask(size: u8) -> u32 {
        match size {
            1 => 0x0000_00FF,
            2 => 0x0000_FFFF,
            3 => 0x00FF_FFFF,
            _ => 0xFFFF_FFFF,
        }
    }

    fn nvic_priority_check(addr: u32) -> bool {
        const NVIC_PRIO_BASE: u32 = 0xE000E300;
        addr >= NVIC_PRIO_BASE && addr < NVIC_PRIO_BASE + 0x100
    }

    /// Whether the RCC clock for the peripheral at `base_addr` is enabled.
    ///
    /// NOT enforced by read()/write(): the emulator deliberately answers
    /// accesses to unclocked peripherals, so firmware that forgets its
    /// `RCC_APBxENR` bit still runs here even though it would read back zeros
    /// on real silicon. Gating on this is a behaviour change (some test
    /// firmware relies on the lenient path), so it stays opt-in rather than
    /// being wired in silently — the mapping is kept ready for that switch.
    #[allow(dead_code)]
    fn clock_enabled(&self, base_addr: u32) -> bool {
        let (ahbenr, apb2enr, apb1enr) = *self.rcc_enrs.borrow();
        if base_addr >= 0xE000_0000 { return true; }
        if base_addr >= 0x4002_1000 && base_addr < 0x4002_2000 { return true; }
        match base_addr {
            0x4002_0000 => (ahbenr & 1) != 0,
            0x4002_0400 => (ahbenr & (1 << 1)) != 0,
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
            0x4001_8000 => (ahbenr & (1 << 10)) != 0, // SDIOEN
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
        if let Some(p) = self.bus.borrow().get(addr) {
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
        } else if let Some(p) = self.bus.borrow().get(addr) {
            p.peripheral.borrow_mut().read_sized(sys, addr - p.start, size)
        } else { 0 };
        // Registers are read as a whole word; a sub-word access wants the byte
        // lane it addressed, so shift it DOWN into bits [0, 8*size) — the JS
        // memory hook (and the bit-band path above) take the low bytes of the
        // returned value.
        if is_reg && byte_offset != 0 {
            (value >> (8 * byte_offset as u32)) & Self::width_mask(size)
        } else if is_reg {
            value & Self::width_mask(size)
        } else {
            value
        }
    }

    pub fn write(&self, sys: &System, addr: u32, size: u8, mut value: u32) {
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
        // NOTE: an ALIGNED sub-word store (byte_offset == 0, size < 4) is
        // deliberately passed through as a whole-word write instead of being
        // merged. Merging would require reading the register back, and the
        // registers written that way are exactly the ones with read side
        // effects — CMSIS types SPI/USART DR as __IO uint16_t, so `SPI->DR = b`
        // compiles to strh, and a read-back would consume RXNE/flags. Those
        // registers have no meaningful bits above the lane, so writing the
        // narrow value directly is correct for them.
        if byte_offset != 0 {
            // Sub-word store into a byte lane above bit 0: merge into the
            // current word so the bytes OUTSIDE the addressed lane survive —
            // both below the lane and above it. Only the lanes the access
            // actually covers (`size` bytes at `byte_offset`) are replaced.
            let shift = 8 * byte_offset as u32;
            let lane = Self::width_mask(size).checked_shl(shift).unwrap_or(0);
            let v = self.read(sys, addr, 4);
            value = (v & !lane) | ((value << shift) & lane);
        }
        // Always allow writes to RCC (0x40021000-0x40021FFF)
        if addr >= 0x4002_1000 && addr < 0x4002_2000 {
            self.update_rcc_enrs(addr - 0x4002_1000, value);
        }
        if Self::NVIC_REGS_BASE <= addr && addr < Self::NVIC_REGS_END {
            self.nvic.borrow_mut().write(sys, addr - Self::NVIC_REGS_BASE, value);
        } else if let Some(p) = self.bus.borrow().get(addr) {
            p.peripheral.borrow_mut().write_sized(sys, addr - p.start, size, value);
        }
    }

    pub fn can_inject_message(&self, sys: &System, addr: u32, tir: u32, tdtr: u32, tdlr: u32, tdhr: u32) -> bool {
        if let Some(p) = self.bus.borrow().get(addr) {
            p.peripheral.borrow_mut().can_inject_message(sys, tir, tdtr, tdlr, tdhr)
        } else { false }
    }

    pub fn rx_byte(&self, sys: &System, addr: u32, byte: u8) -> bool {
        if let Some(p) = self.bus.borrow().get(addr) {
            p.peripheral.borrow_mut().rx_byte(sys, byte);
            true
        } else { false }
    }

    pub fn rx_pending(&self, addr: u32) -> u32 {
        if let Some(p) = self.bus.borrow().get(addr) {
            p.peripheral.borrow().rx_pending()
        } else { 0 }
    }

    /// Queries the AFIO peripheral for the GPIO port mapped to a given EXTI line (0-15).
    pub fn exti_port_for_line(&self, line: u32) -> Option<char> {
        if let Some(slot) = self.bus.borrow().get(0x4001_0000) {
            return slot.peripheral.borrow().exti_port(line);
        }
        None
    }

    /// Queries AFIO MAPR remap bits for a given peripheral name.
    /// Returns None if the peripheral has no remap bits or AFIO is unavailable.
    pub fn afio_remap_status(&self, name: &str) -> Option<u32> {
        if let Some(slot) = self.bus.borrow().get(0x4001_0000) {
            return slot.peripheral.borrow().remap_status(name);
        }
        None
    }

    /// Called from GPIO when a pin changes state. Triggers EXTI if the port/pin
    /// matches the AFIO EXTICR mapping and EXTI edge configuration.
    pub fn gpio_exti_trigger(&self, sys: &System, port: u8, pin: u8, rising: bool) {
        if let Some(slot) = self.bus.borrow().get(0x4001_0400) {
            slot.peripheral.borrow_mut().gpio_pin_changed(sys, port, pin, rising);
        }
    }

    /// Peripheral DMA request: fires the enabled DMA channel if configured.
    /// Channels 1-7 go to DMA1; channels 8-12 go to DMA2 (ch = channel - 8).
    pub fn dma_request(&self, sys: &System, channel: u32) {
        if channel <= 7 {
            if let Some(p) = self.bus.borrow().get(0x4002_0000) {
                p.peripheral.borrow_mut().dma_request(sys, channel);
            }
        } else if channel <= 12 {
            if let Some(p) = self.bus.borrow().get(0x4002_0400) {
                p.peripheral.borrow_mut().dma_request(sys, channel - 8);
            }
        }
    }

    /// Timer-originated ADC external trigger: fanned out to every ADC, which
    /// gates on its own EXTSEL/JEXTSEL configuration.
    pub fn adc_timer_trigger(&self, sys: &System, tim_base: u32, ch: u8) {
        if let Some(slot) = self.bus.borrow().get(0x4001_2400) {
            slot.peripheral.borrow_mut().adc_timer_trigger(sys, tim_base, ch);
        }
        if let Some(slot) = self.bus.borrow().get(0x4001_2800) {
            slot.peripheral.borrow_mut().adc_timer_trigger(sys, tim_base, ch);
        }
    }

    /// EXTI-originated ADC trigger (line 11 = regular, 15 = injected).
    pub fn adc_exti_trigger(&self, sys: &System, line: u32) {
        if let Some(slot) = self.bus.borrow().get(0x4001_2400) {
            slot.peripheral.borrow_mut().adc_exti_trigger(sys, line);
        }
        if let Some(slot) = self.bus.borrow().get(0x4001_2800) {
            slot.peripheral.borrow_mut().adc_exti_trigger(sys, line);
        }
    }

    /// Analog voltage driven on a pin by a peripheral (DAC output), if any.
    pub fn dac_output(&self, port: u8, pin: u8) -> Option<u32> {
        if let Some(p) = self.bus.borrow().get(0x4000_7400) {
            if let Ok(dac) = p.peripheral.try_borrow() {
                return dac.dac_output(port, pin);
            }
        }
        None
    }

    /// STOP/STANDBY detection: SCB SCR SLEEPDEEP (bit 2). In deep sleep the core is
    /// halted; only LSI/LSE-clocked peripherals (IWDG, RTC) keep running.
    pub fn in_deep_sleep(&self) -> bool {
        if let Some(p) = self.bus.borrow().get(0xE000_ED00) {
            if let Ok(s) = p.peripheral.try_borrow() {
                return s.in_deep_sleep();
            }
        }
        false
    }

    /// Raise a fault through the SCB (CFSR/HFSR/BFAR + NVIC escalation).
    pub fn raise_fault(&self, sys: &System, kind: u32, addr: u32) {
        if let Some(p) = self.bus.borrow().get(0xE000_ED00) {
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
use sdio::Sdio;

#[cfg(test)]
mod tests {
    use super::Peripherals;
    use crate::test_util::with_sys;

    /// DMA1 channel 1 CMAR: a plain 32-bit scratch register (stored and read
    /// back unmasked, no read side effects) — ideal for byte-lane assertions.
    const MAR: u32 = 0x4002_0014;

    #[test]
    fn reads_the_addressed_byte_lane() {
        with_sys(|sys| {
            sys.p.write(sys, MAR, 4, 0xAABB_CCDD);

            assert_eq!(sys.p.read(sys, MAR, 4), 0xAABB_CCDD, "word read");
            // Upper half-word: shifted DOWN into the low bytes, since the JS
            // memory hook takes the low `size` bytes of the returned value.
            assert_eq!(sys.p.read(sys, MAR + 2, 2), 0x0000_AABB, "upper half");
            assert_eq!(sys.p.read(sys, MAR, 2), 0x0000_CCDD, "lower half");
            assert_eq!(sys.p.read(sys, MAR + 3, 1), 0x0000_00AA, "byte 3");
            assert_eq!(sys.p.read(sys, MAR + 1, 1), 0x0000_00CC, "byte 1");
            assert_eq!(sys.p.read(sys, MAR, 1), 0x0000_00DD, "byte 0");
        });
    }

    #[test]
    fn sub_word_write_preserves_the_other_lanes() {
        with_sys(|sys| {
            sys.p.write(sys, MAR, 4, 0xAABB_CCDD);
            sys.p.write(sys, MAR + 2, 2, 0x0000_1234);
            assert_eq!(sys.p.read(sys, MAR, 4), 0x1234_CCDD, "upper half store");

            sys.p.write(sys, MAR, 4, 0xAABB_CCDD);
            sys.p.write(sys, MAR + 1, 1, 0x0000_0099);
            // Bytes ABOVE the written lane must survive too, not just below.
            assert_eq!(sys.p.read(sys, MAR, 4), 0xAABB_99DD, "byte 1 store");

            sys.p.write(sys, MAR, 4, 0xAABB_CCDD);
            sys.p.write(sys, MAR + 3, 1, 0x0000_0011);
            assert_eq!(sys.p.read(sys, MAR, 4), 0x11BB_CCDD, "byte 3 store");
        });
    }

    #[test]
    fn bit_band_touches_only_its_own_bit() {
        with_sys(|sys| {
            // Alias of bit 0 of the byte at MAR+2, i.e. bit 16 of the word.
            let alias = 0x4200_0000 + (MAR - 0x4000_0000) * 32;
            sys.p.write(sys, MAR, 4, 0xAABA_CCDD);

            assert_eq!(sys.p.read(sys, alias + 2 * 32, 4), 0, "bit reads clear");
            sys.p.write(sys, alias + 2 * 32, 4, 1);
            assert_eq!(sys.p.read(sys, MAR, 4), 0xAABB_CCDD, "set bit 16");
            assert_eq!(sys.p.read(sys, alias + 2 * 32, 4), 1, "bit reads set");

            sys.p.write(sys, alias + 2 * 32, 4, 0);
            assert_eq!(sys.p.read(sys, MAR, 4), 0xAABA_CCDD, "clear bit 16");
        });
    }

    #[test]
    fn width_mask_covers_each_access_size() {
        assert_eq!(Peripherals::width_mask(1), 0x0000_00FF);
        assert_eq!(Peripherals::width_mask(2), 0x0000_FFFF);
        assert_eq!(Peripherals::width_mask(4), 0xFFFF_FFFF);
    }
}
