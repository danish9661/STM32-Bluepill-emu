use crate::system::System;
use super::Peripheral;

pub struct Usb;

impl Usb {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "USB" { Some(Box::new(Usb)) } else { None }
    }
}

impl Peripheral for Usb {
    fn read(&mut self, _sys: &System, _offset: u32) -> u32 { 0 }
    fn write(&mut self, _sys: &System, _offset: u32, _value: u32) {}
}
