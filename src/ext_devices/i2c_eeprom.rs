use crate::system::System;
use super::ExtDevice;

pub struct I2cEepromConfig {
    pub peripheral: String,
    pub address: u8,
    pub content: Vec<u8>,
    pub size: usize,
}

pub struct I2cEeprom {
    pub config: I2cEepromConfig,
    name: String,
    mem: Vec<u8>,
    addr: usize,
    addr_bytes: Vec<u8>,
    phase: EepromPhase,
}

enum EepromPhase { Addr, Data }

impl I2cEeprom {
    pub fn new(config: I2cEepromConfig) -> Self {
        let mut mem = config.content.clone();
        mem.resize(config.size, 0);
        I2cEeprom {
            config,
            name: String::new(),
            mem,
            addr: 0,
            addr_bytes: Vec::new(),
            phase: EepromPhase::Addr,
        }
    }
}

impl ExtDevice<(), u8> for I2cEeprom {
    fn connect_peripheral(&mut self, peri_name: &str) -> String {
        self.name = format!("{} i2c-eeprom", peri_name);
        self.name.clone()
    }

    fn reset(&mut self) {
        self.phase = EepromPhase::Addr;
        self.addr_bytes.clear();
    }

    fn read(&mut self, _sys: &System, _addr: ()) -> u8 {
        self.phase = EepromPhase::Data;
        let v = self.mem.get(self.addr).copied().unwrap_or(0);
        self.addr = (self.addr + 1) % self.config.size;
        v
    }

    fn write(&mut self, _sys: &System, _addr: (), v: u8) {
        match self.phase {
            EepromPhase::Addr => {
                self.addr_bytes.push(v);
                let addr_width = if self.config.size > 256 { 2 } else { 1 };
                if self.addr_bytes.len() >= addr_width {
                    self.addr = if addr_width == 2 {
                        (self.addr_bytes[0] as usize) << 8 | self.addr_bytes[1] as usize
                    } else {
                        self.addr_bytes[0] as usize
                    };
                    self.addr %= self.config.size;
                    self.phase = EepromPhase::Data;
                }
            }
            EepromPhase::Data => {
                if self.addr < self.mem.len() {
                    self.mem[self.addr] = v;
                }
                self.addr = (self.addr + 1) % self.config.size;
            }
        }
    }
}
