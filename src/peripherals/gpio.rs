use crate::system::System;
use super::Peripheral;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

use regex::Regex;

const NUM_PORTS: usize = 8;

/// Optional output slew (rise/fall) delay in instructions, 0 = instant.
/// Affects IDR readback only (device callbacks stay instant).
static GPIO_SLEW: AtomicU32 = AtomicU32::new(0);

pub fn set_gpio_slew(inst: u32) {
    GPIO_SLEW.store(inst, Ordering::Relaxed);
}

/// Pin-change event buffer: flat [port, pin, level] triples, recorded whenever
/// the chip drives an output pin to a NEW level (ODR/BSRR/BRR writes and
/// CRL/CRH mode-change re-drives). Drained by JS via gpio_take_pin_events();
/// cleared by the next init()/init_svd(). Bounded: on overflow the buffer is
/// dropped wholesale (page drains per batch, so this never happens in practice).
const MAX_PIN_EVENTS: usize = 1024;
static GPIO_PIN_EVENTS: Mutex<Vec<u32>> = Mutex::new(Vec::new());

pub fn clear_pin_events() {
    GPIO_PIN_EVENTS.lock().unwrap().clear();
}

/// Drain buffered pin-change events as a flat [port, pin, level, ...] array.
pub fn take_pin_events() -> Vec<u32> {
    let mut ev = GPIO_PIN_EVENTS.lock().unwrap();
    std::mem::take(&mut *ev)
}

fn record_pin_event(port: u8, pin: u8, level: bool) {
    let mut ev = GPIO_PIN_EVENTS.lock().unwrap();
    if ev.len() + 3 > MAX_PIN_EVENTS {
        ev.clear();
    }
    ev.push(port as u32);
    ev.push(pin as u32);
    ev.push(level as u32);
}

#[derive(Clone, Copy)]
pub struct Pin {
    port: u8,
    pin: u8,
}

impl Pin {
    pub fn from_str(name: &str) -> Self {
        let name = name.to_uppercase();
        let re = Regex::new(r"^P?([A-E])(\d+)$").unwrap();
        let captures = re.captures(&name).expect("Pin name invalid");
        let port = captures.get(1).unwrap().as_str().chars().next().unwrap();
        let port = GpioPorts::port_index(port);
        let pin = captures.get(2).unwrap().as_str().parse().unwrap();
        assert!(pin < 16);
        Self { port, pin }
    }

    pub fn new(port: u8, pin: u8) -> Self {
        Self { port, pin }
    }
}

pub struct GpioPorts {
    read_callbacks: [Vec<(u8, Box<dyn FnMut(&System) -> bool>)>; NUM_PORTS],
    write_callbacks: [Vec<(u8, Box<dyn FnMut(&System, bool)>)>; NUM_PORTS],
    output_states: [u16; NUM_PORTS],
    input_states: [u16; NUM_PORTS],
    /// Pending output transitions for slew emulation: (pin, transition_at, old_level)
    pending_transitions: [Vec<(u8, u64, bool)>; NUM_PORTS],
    /// Analog wire voltage per pin (12-bit, 0xFFFF = no analog source).
    /// When set, ADC channels mapped to the pin sample this voltage with an
    /// RC sample-and-hold instead of the injected simulation value.
    analog_states: [u16; NUM_PORTS * 16],
}

impl Default for GpioPorts {
    fn default() -> Self {
        Self {
            read_callbacks: Default::default(),
            write_callbacks: Default::default(),
            output_states: [0; NUM_PORTS],
            input_states: [0; NUM_PORTS],
            pending_transitions: Default::default(),
            analog_states: [0xFFFF; NUM_PORTS * 16],
        }
    }
}

impl GpioPorts {
    pub fn port_index(letter: char) -> u8 {
        match letter {
            'A'..='G' => letter as u8 - 'A' as u8,
            _ => panic!("Invalid GPIO port {}", letter),
        }
    }

    pub fn add_read_callback(&mut self, pin: Pin, cb: impl FnMut(&System) -> bool + 'static) {
        self.read_callbacks[pin.port as usize].push((pin.pin, Box::new(cb)));
    }

    pub fn add_write_callback(&mut self, pin: Pin, cb: impl FnMut(&System, bool) + 'static) {
        self.write_callbacks[pin.port as usize].push((pin.pin, Box::new(cb)));
    }

    #[allow(dead_code)]
    pub fn read_port(&mut self, sys: &System, port: u8) -> u16 {
        let mut v = 0;
        for (pin, cb) in &mut self.read_callbacks[port as usize] {
            if cb(sys) {
                v |= 1 << *pin;
            }
        }
        v
    }

    /// External driver level for a pin (read callback), if one is registered.
    pub fn read_pin_option(&mut self, sys: &System, port: u8, pin: u8) -> Option<bool> {
        for (p, cb) in &mut self.read_callbacks[port as usize] {
            if *p == pin {
                return Some(cb(sys));
            }
        }
        None
    }

    /// Effective pin level: read callback if registered, otherwise the last driven output state.
    pub fn read_pin_effective(&mut self, sys: &System, port: u8, pin: u8) -> bool {
        for (p, cb) in &mut self.read_callbacks[port as usize] {
            if *p == pin {
                return cb(sys);
            }
        }
        (self.output_states[port as usize] >> pin) & 1 != 0
    }

    /// Set an analog wire voltage on a pin (12-bit). 0xFFFF clears it.
    pub fn set_analog(&mut self, port: u8, pin: u8, level: u16) {
        self.analog_states[port as usize * 16 + pin as usize] = level;
    }

    /// Analog voltage present on the pin, if one is wired.
    pub fn analog_pin_value(&self, port: u8, pin: u8) -> Option<u16> {
        let v = self.analog_states[port as usize * 16 + pin as usize];
        if v == 0xFFFF { None } else { Some(v) }
    }

    /// Drive an output pin. `record_event` controls whether a NEW driven level
    /// emits a pin-change event (false for alternate-function pins — PWM/SPI
    /// clocks churn at MHz and are observed via pwmDuty/onPeriphWrite instead).
    pub fn write_port(&mut self, sys: &System, port: u8, pin: u8, value: bool, record_event: bool) {
        let old = (self.output_states[port as usize] >> pin) & 1 != 0;
        if old != value && record_event {
            record_pin_event(port, pin, value);
        }
        let slew = GPIO_SLEW.load(Ordering::Relaxed) as u64;
        if slew > 0 {
            if old != value {
                self.pending_transitions[port as usize]
                    .push((pin, crate::system::instruction_count() + slew, old));
            }
        }
        if value {
            self.output_states[port as usize] |= 1 << pin;
        } else {
            self.output_states[port as usize] &= !(1 << pin);
        }
        for (pin_cb, cb) in &mut self.write_callbacks[port as usize] {
            if *pin_cb == pin {
                cb(sys, value);
            }
        }
    }

    /// Wire level of an output pin, honoring pending slew transitions.
    /// External drivers (read callbacks) always win over driven state.
    pub fn read_output_pin(&mut self, sys: &System, port: u8, pin: u8) -> bool {
        if let Some(v) = self.read_pin_option(sys, port, pin) {
            return v;
        }
        self.driven_pin_level(port, pin)
    }

    /// Driven output level of a pin, honoring pending slew transitions and
    /// ignoring external drivers (used where the drive state always wins,
    /// e.g. an open-drain output driving low).
    pub fn driven_pin_level(&mut self, port: u8, pin: u8) -> bool {
        let now = crate::system::instruction_count();
        let mut v = (self.output_states[port as usize] >> pin) & 1 != 0;
        self.pending_transitions[port as usize].retain(|(p, at, old)| {
            if *p != pin {
                return true;
            }
            if now >= *at {
                false // transition complete: final level is the driven state in v
            } else {
                v = *old;
                true
            }
        });
        v
    }

    pub fn set_input_pin(&mut self, sys: &System, port: u8, pin: u8, value: bool) {
        let prev = (self.input_states[port as usize] >> pin) & 1 != 0;
        self.set_input_pin_raw(port, pin, value);
        // A real push-button wired to an input pin produces EXTI edges. Fire
        // the same edge detection as GPIO output writes so attachInterrupt()
        // works for page-driven input pins (button widgets).
        if prev != value {
            let rising = value && !prev;
            sys.p.gpio_exti_trigger(sys, port, pin, rising);
        }
    }

    fn set_input_pin_raw(&mut self, port: u8, pin: u8, value: bool) {
        let mut found = false;
        for (p, ref mut cb) in &mut self.read_callbacks[port as usize] {
            if *p == pin {
                found = true;
                *cb = Box::new(move |_| value);
            }
        }
        if !found {
            self.read_callbacks[port as usize].push((pin, Box::new(move |_| value)));
        }
        if value {
            self.input_states[port as usize] |= 1 << pin;
        } else {
            self.input_states[port as usize] &= !(1 << pin);
        }
    }

    pub fn read_input_pin(&self, port: u8, pin: u8) -> bool {
        (self.input_states[port as usize] >> pin) & 1 != 0
    }
}

#[derive(Default)]
pub struct Gpio {
    #[allow(dead_code)]
    port_letter: char,
    port: u8,
    crl: u32,
    crh: u32,
    odr: u32,
    bsrr: u32,
    brr: u32,
    lckr: u32,
}

impl Gpio {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if let Some(block) = name.strip_prefix("GPIO") {
            let port_letter = block.chars().next().unwrap();
            let port = GpioPorts::port_index(port_letter);
            Some(Box::new(Self { port_letter, port, ..Self::default() }))
        } else {
            None
        }
    }

    fn pin_mode(&self, pin: u8) -> u8 {
        if pin < 8 {
            ((self.crl >> (pin * 4)) & 0b11) as u8
        } else {
            ((self.crh >> ((pin - 8) * 4)) & 0b11) as u8
        }
    }

    fn pin_cnf(&self, pin: u8) -> u8 {
        if pin < 8 {
            ((self.crl >> (pin * 4 + 2)) & 0b11) as u8
        } else {
            ((self.crh >> ((pin - 8) * 4 + 2)) & 0b11) as u8
        }
    }

    fn pin_is_output(&self, pin: u8) -> bool {
        self.pin_mode(pin) != 0
    }

    /// True for alternate-function output pins (cnf=0b10, mode!=0): driven by a
    /// peripheral, not by ODR — excluded from pin-change events.
    fn pin_is_af(&self, pin: u8) -> bool {
        self.pin_mode(pin) != 0 && self.pin_cnf(pin) == 2
    }

    #[allow(dead_code)]
    fn pin_is_analog(&self, pin: u8) -> bool {
        self.pin_mode(pin) == 0 && self.pin_cnf(pin) == 0
    }

    /// Electrical wire level of a pin:
    /// - external driver (read callback) wins whenever present;
    /// - input floating: external or 0;
    /// - input with pull-up/down: ODR bit selects pull direction;
    /// - output push-pull: ODR drives both levels;
    /// - output open-drain: low is driven, high releases the line (external/pull/0);
    /// - analog: always 0.
    fn pin_level(&self, sys: &System, gpio: &mut GpioPorts, pin: u8) -> bool {
        let mode = self.pin_mode(pin);
        let cnf = self.pin_cnf(pin);
        let odr = (self.odr >> pin) & 1 != 0;
        if mode == 0 {
            match cnf {
                0 => gpio.read_pin_option(sys, self.port, pin).unwrap_or(false), // floating
                1 => gpio.read_pin_option(sys, self.port, pin).unwrap_or(odr),   // pull-up if ODR=1
                _ => false, // reserved or analog
            }
        } else if cnf == 0 {
            // push-pull: ODR drives both levels; honor pending slew transitions
            gpio.read_output_pin(sys, self.port, pin)
        } else {
            // open-drain: 0 drives low; 1 releases the line
            if odr {
                gpio.read_pin_option(sys, self.port, pin).unwrap_or(false)
            } else {
                // driven low (wins over any external pull); honor pending slew
                gpio.driven_pin_level(self.port, pin)
            }
        }
    }

    fn iter_port_reg_changes(old_value: u32, new_value: u32, stride: u8, mut f: impl FnMut(u8, u8)) {
        let mut changes = old_value ^ new_value;
        let stride_mask = 0xFF >> (8 - stride);
        while changes != 0 {
            let right_most_bit = changes.trailing_zeros() as u8;
            let pin = right_most_bit / stride;
            if pin <= 16 {
                let v = (new_value >> (pin * stride)) as u8 & stride_mask;
                f(pin, v);
            }
            changes &= !(stride_mask as u32) << (pin * stride);
        }
    }

    #[allow(dead_code)]
    fn port_str(&self, pin: u8) -> String {
        format!("GPIO{} P{}{}", self.port_letter, self.port_letter, pin)
    }
}

impl Peripheral for Gpio {
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.crl,
            0x04 => self.crh,
            0x08 => {
                let mut gpio = sys.p.gpio.borrow_mut();
                let mut v = 0u16;
                for pin in 0..16 {
                    if self.pin_level(sys, &mut gpio, pin) {
                        v |= 1 << pin;
                    }
                }
                v as u32
            }
            0x0C => self.odr,
            0x10 => self.bsrr,
            0x14 => self.brr,
            0x18 => self.lckr,
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                let old = self.crl;
                self.crl = value;
                let mut gpio = sys.p.gpio.borrow_mut();
                // Mode change re-drives pins now in output mode with their ODR
                // level (real HW behavior); write_port records pin events only
                // when the driven level actually changes.
                Self::iter_port_reg_changes(old, value, 4, |pin, _| {
                    if self.pin_is_output(pin) {
                        gpio.write_port(sys, self.port, pin, ((self.odr >> pin) & 1) != 0, !self.pin_is_af(pin));
                    }
                });
            }
            0x04 => {
                let old = self.crh;
                self.crh = value;
                let mut gpio = sys.p.gpio.borrow_mut();
                Self::iter_port_reg_changes(old, value, 4, |pin, _| {
                    let pin = pin + 8;
                    if self.pin_is_output(pin) {
                        gpio.write_port(sys, self.port, pin, ((self.odr >> pin) & 1) != 0, !self.pin_is_af(pin));
                    }
                });
            }
            0x08 => {}
            0x0C => {
                let old_odr = self.odr;
                let mut gpio = sys.p.gpio.borrow_mut();
                Self::iter_port_reg_changes(old_odr, value, 1, |pin, v| {
                    if self.pin_is_output(pin) {
                        gpio.write_port(sys, self.port, pin, v != 0, !self.pin_is_af(pin));
                        sys.p.gpio_exti_trigger(sys, self.port, pin, v != 0);
                    }
                });
                // Full register write: input pins use ODR to select pull-up/down
                self.odr = value;
            }
            0x10 => {
                let reset = value >> 16;
                let set = value & 0xFFFF;
                let mut gpio = sys.p.gpio.borrow_mut();
                Self::iter_port_reg_changes(0, set, 1, |pin, _| {
                    if self.pin_is_output(pin) {
                        gpio.write_port(sys, self.port, pin, true, !self.pin_is_af(pin));
                        sys.p.gpio_exti_trigger(sys, self.port, pin, true);
                    }
                });
                Self::iter_port_reg_changes(0, reset, 1, |pin, _| {
                    if self.pin_is_output(pin) {
                        gpio.write_port(sys, self.port, pin, false, !self.pin_is_af(pin));
                        sys.p.gpio_exti_trigger(sys, self.port, pin, false);
                    }
                });
                self.odr = (self.odr & !reset) | set;
                self.bsrr = value;
            }
            0x14 => {
                let mut gpio = sys.p.gpio.borrow_mut();
                Self::iter_port_reg_changes(0, value, 1, |pin, _| {
                    if self.pin_is_output(pin) {
                        gpio.write_port(sys, self.port, pin, false, !self.pin_is_af(pin));
                        sys.p.gpio_exti_trigger(sys, self.port, pin, false);
                    }
                });
                self.odr &= !value;
                self.brr = value;
            }
            0x18 => self.lckr = value,
            _ => {}
        }
    }

    fn tick(&mut self, _sys: &System) {}
}
