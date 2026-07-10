use crate::system::System;
use super::ExtDevice;

pub struct UsartProbeConfig {
    pub peripheral: String,
}

pub struct UsartProbe {
    pub config: UsartProbeConfig,
    name: String,
    rx: Vec<u8>,
}

impl UsartProbe {
    pub fn new(config: UsartProbeConfig) -> Self {
        Self { config, name: String::new(), rx: Vec::new() }
    }
}

impl ExtDevice<(), u8> for UsartProbe {
    fn connect_peripheral(&mut self, peri_name: &str) -> String {
        self.name = format!("{} usart-probe", peri_name);
        self.name.clone()
    }

    fn read(&mut self, _sys: &System, _addr: ()) -> u8 { 0 }

    fn write(&mut self, _sys: &System, _addr: (), v: u8) {
        if v == 0x0a {
            let line = String::from_utf8_lossy(&self.rx);
            let line = line.trim();
            log::info!("{} '{}'", self.name, line);
            self.rx.clear();
        } else {
            self.rx.push(v);
        }
    }
}
