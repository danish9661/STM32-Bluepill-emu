use crate::system::System;
use super::ExtDevice;

pub struct LcdConfig {
    pub peripheral: String,
    pub framebuffer: String,
    pub cs: Option<String>,
}

pub struct Lcd {
    pub config: LcdConfig,
    pub name: String,
    pub fb: Vec<u8>,
    pub current_x: u16, pub current_y: u16,
    pub width: u16, pub height: u16,
    pub drawing: bool,
}

pub const LCD_WIDTH: u16 = 128;
pub const LCD_HEIGHT: u16 = 64;

impl Lcd {
    pub fn new(config: LcdConfig) -> Self {
        Self {
            config, name: String::new(),
            fb: vec![0; (LCD_WIDTH as usize) * (LCD_HEIGHT as usize)],
            current_x: 0, current_y: 0,
            width: LCD_WIDTH, height: LCD_HEIGHT,
            drawing: false,
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
        // Framing bytes always win, even mid-frame (resync): 0xFB restarts
        // at pixel (0,0), 0xFC ends the session. Neither is ever stored.
        // (Pixel data here is 0xFF/0x00/0x40, so no legit byte collides.)
        // Two real bugs died here: 0xFB queued as a one-arg command ate
        // pixel 0 (whole frame shifted one left, 'L' rendered as 'C'), and
        // a post-wrap 0xFC landing in fb[0] plus stray segment bytes.
        if v == 0xFB {
            self.current_x = 0; self.current_y = 0;
            self.drawing = true;
            log::debug!("{} start drawing", self.name);
            return;
        }
        if v == 0xFC {
            self.drawing = false;
            return;
        }
        if self.drawing {
            let idx = (self.current_y as usize) * (LCD_WIDTH as usize) + (self.current_x as usize);
            if idx < self.fb.len() {
                self.fb[idx] = v;
                self.current_x += 1;
                if self.current_x >= LCD_WIDTH {
                    self.current_x = 0;
                    // Saturate at the last row (never wrap to row 0).
                    if self.current_y + 1 < LCD_HEIGHT {
                        self.current_y += 1;
                    }
                }
            }
        }
    }

    fn reset(&mut self) {
        self.fb.fill(0);
        self.current_x = 0;
        self.current_y = 0;
        self.drawing = false;
    }
}
