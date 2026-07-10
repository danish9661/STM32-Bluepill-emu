use crate::system::System;
use super::Peripheral;

use regex::Regex;

const NUM_PORTS: usize = 8;

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
}

#[derive(Default)]
pub struct GpioPorts {
    read_callbacks: [Vec<(u8, Box<dyn FnMut(&System) -> bool>)>; NUM_PORTS],
    write_callbacks: [Vec<(u8, Box<dyn FnMut(&System, bool)>)>; NUM_PORTS],
    output_states: [u16; NUM_PORTS],
    input_states: [u16; NUM_PORTS],
}

impl GpioPorts {
    pub fn port_index(letter: char) -> u8 {
        match letter {
            'A'..='E' => letter as u8 - 'A' as u8,
            _ => panic!("Invalid GPIO port {}", letter),
        }
    }

    pub fn add_read_callback(&mut self, pin: Pin, cb: impl FnMut(&System) -> bool + 'static) {
        self.read_callbacks[pin.port as usize].push((pin.pin, Box::new(cb)));
    }

    pub fn add_write_callback(&mut self, pin: Pin, cb: impl FnMut(&System, bool) + 'static) {
        self.write_callbacks[pin.port as usize].push((pin.pin, Box::new(cb)));
    }

    pub fn read_port(&mut self, sys: &System, port: u8) -> u16 {
        let mut v = 0;
        for (pin, cb) in &mut self.read_callbacks[port as usize] {
            if cb(sys) {
                v |= 1 << *pin;
            }
        }
        v
    }

    pub fn write_port(&mut self, sys: &System, port: u8, pin: u8, value: bool) {
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

    pub fn read_output_pin(&self, port: u8, pin: u8) -> bool {
        (self.output_states[port as usize] >> pin) & 1 != 0
    }

    pub fn set_input_pin(&mut self, port: u8, pin: u8, value: bool) {
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
    port_letter: char,
    port: u8,
    crl: u32,
    crh: u32,
    idr: u32,
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
                let v = sys.p.gpio.borrow_mut().read_port(sys, self.port) as u32;
                v
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
            0x00 => self.crl = value,
            0x04 => self.crh = value,
            0x08 => {}
            0x0C => {
                let mut gpio = sys.p.gpio.borrow_mut();
                Self::iter_port_reg_changes(self.odr, value, 1, |pin, v| {
                    gpio.write_port(sys, self.port, pin, v != 0);
                });
                self.odr = value;
            }
            0x10 => {
                let reset = value >> 16;
                let set = value & 0xFFFF;
                let mut gpio = sys.p.gpio.borrow_mut();
                Self::iter_port_reg_changes(0, set, 1, |pin, _| {
                    gpio.write_port(sys, self.port, pin, true);
                });
                Self::iter_port_reg_changes(0, reset, 1, |pin, _| {
                    gpio.write_port(sys, self.port, pin, false);
                });
                self.odr &= !reset;
                self.odr |= set;
                self.bsrr = value;
            }
            0x14 => {
                let mut gpio = sys.p.gpio.borrow_mut();
                Self::iter_port_reg_changes(0, value, 1, |pin, _| {
                    gpio.write_port(sys, self.port, pin, false);
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
