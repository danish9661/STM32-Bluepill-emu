use std::collections::VecDeque;
use crate::system::System;
use super::ExtDevice;

pub struct DisplayConfig {
    pub peripheral: String,
    pub cmd_addr_bit: u32,
    pub swap_bytes: Option<bool>,
    pub replies: Option<Vec<ReplyConfig>>,
    pub framebuffer: String,
}

pub struct ReplyConfig {
    pub cmd: u8,
    pub data: Vec<u16>,
}

pub struct Display {
    pub config: DisplayConfig,
    name: String,
    cmd: Option<(u8, Vec<u16>)>,
    reply: VecDeque<u16>,
    drawing: bool,
    current_x: u16, current_y: u16,
    #[allow(dead_code)]
    width: u16, #[allow(dead_code)]
    height: u16,
    draw_left: u16, draw_right: u16, draw_top: u16, draw_bottom: u16,
}

impl Display {
    pub fn new(config: DisplayConfig) -> Self {
        let width = 240;
        let height = 240;
        Self {
            config, name: String::new(),
            cmd: None, reply: VecDeque::new(),
            drawing: false,
            current_x: 0, current_y: 0,
            width, height,
            draw_left: 0, draw_right: width - 1, draw_top: 0, draw_bottom: height - 1,
        }
    }

    fn handle_cmd(&mut self) {
        if let Some((cmd, args)) = self.cmd.take() {
            match (cmd, args.len()) {
                (0x2A, 4) => {
                    self.draw_left = (args[0] << 8) | args[1];
                    self.draw_right = (args[2] << 8) | args[3];
                }
                (0x2B, 4) => {
                    self.draw_top = (args[0] << 8) | args[1];
                    self.draw_bottom = (args[2] << 8) | args[3];
                }
                (0x2C, 0) => {
                    self.drawing = true;
                    self.current_x = self.draw_left;
                    self.current_y = self.draw_top;
                }
                _ => {
                    if let Some(replies) = self.config.replies.as_ref() {
                        if let Some(reply) = replies.iter().find(|r| r.cmd == cmd) {
                            self.reply = reply.data.iter().cloned().collect();
                            return;
                        }
                    }
                    self.cmd = Some((cmd, args));
                }
            }
        }
    }

    fn finish_cmd(&mut self) {
        self.drawing = false;
        self.cmd = None;
    }
}

impl ExtDevice<u32, u32> for Display {
    fn connect_peripheral(&mut self, peri_name: &str) -> String {
        self.name = format!("{} display", peri_name);
        self.name.clone()
    }

    fn read(&mut self, _sys: &System, addr: u32) -> u32 {
        let mode = if addr & self.config.cmd_addr_bit != 0 { 1 } else { 0 };
        if mode == 1 {
            self.reply.pop_front().unwrap_or(0) as u32
        } else { 0 }
    }

    fn write(&mut self, _sys: &System, addr: u32, value: u32) {
        let mode = if addr & self.config.cmd_addr_bit != 0 { 1 } else { 0 };
        if mode == 0 {
            self.finish_cmd();
            self.cmd = Some((value as u8, vec![]));
        } else {
            if self.drawing {
                self.current_x += 1;
                if self.current_x > self.draw_right {
                    self.current_x = self.draw_left;
                    self.current_y += 1;
                    if self.current_y > self.draw_bottom {
                        self.current_y = self.draw_top;
                    }
                }
            } else if let Some((_cmd, ref mut args)) = self.cmd {
                args.push(value as u16);
            }
        }
        self.handle_cmd();
    }
}
