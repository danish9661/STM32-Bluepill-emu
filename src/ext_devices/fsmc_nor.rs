use crate::system::System;
use crate::ext_devices::ExtDevice;

pub struct FsmcNor {
    name: String,
    pub data: Vec<u8>,
}

impl FsmcNor {
    pub fn new(name: &str, data: &[u8]) -> Self {
        Self { name: name.to_string(), data: data.to_vec() }
    }

    pub fn name(&self) -> &str { &self.name }
}

impl ExtDevice<u32, u32> for FsmcNor {
    fn connect_peripheral(&mut self, _peri_name: &str) -> String {
        String::new()
    }

    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        let i = offset as usize;
        if i < self.data.len() { self.data[i] as u32 } else { 0 }
    }

    fn write(&mut self, _sys: &System, offset: u32, v: u32) {
        let i = offset as usize;
        if i < self.data.len() {
            self.data[i] = v as u8;
        }
    }
}
