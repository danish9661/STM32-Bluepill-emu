use crate::system::System;
use super::ExtDevice;

pub struct LcdConfig {
    pub peripheral: String,
    pub framebuffer: String,
    pub cs: Option<String>,
}

pub struct Lcd {
    pub config: LcdConfig,
    name: String,
    current_x: u16, current_y: u16,
    width: u16, height: u16,
    cmd: Option<(u8, Vec<u8>)>,
    drawing: bool,
}

impl Lcd {
    pub fn new(config: LcdConfig) -> Self {
        let width = 128;
        let height = 64;
        Self {
            config, name: String::new(),
            current_x: 0, current_y: 0,
            width, height, cmd: None, drawing: false,
        }
    }
}

impl ExtDevice<(), u8> for Lcd {
    fn connect_peripheral(&mut self, peri_name: &str) -> String {
        self.name = format!("{} LCD", peri_name);
        self.name.clone()
    }

    fn read(&mut self, _sys: &System, _addr: ()) -> u8 { 0 }

    fn write(&mut self, _sys: &System, _addr: (), v: u8) {
        if self.drawing {
            self.current_x += 1;
            if self.current_x >= self.width {
                self.current_x = 0;
                self.current_y += 1;
                if self.current_y >= self.height {
                    self.current_y = 0;
                }
            }
            return;
        }

        if let Some((cmd, mut args)) = self.cmd.take() {
            args.push(v);
            if cmd == 0xFB {
                self.current_x = 0; self.current_y = 0;
                self.drawing = true;
                log::debug!("{} start drawing", self.name);
            } else {
                self.cmd = Some((cmd, args));
            }
        } else if v == 0xFB {
            self.cmd = Some((v, vec![]));
        }
    }
}
