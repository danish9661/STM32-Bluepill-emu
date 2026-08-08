use std::collections::VecDeque;
use std::convert::TryFrom;
use std::sync::atomic::{AtomicU32, Ordering};
use crate::system::System;
use super::ExtDevice;

static CS_CHANGES: AtomicU32 = AtomicU32::new(0);
pub fn cs_change_count() -> u32 { CS_CHANGES.load(Ordering::Relaxed) }

pub struct SpiFlashConfig {
    pub peripheral: String,
    pub jedec_id: u32,
    pub content: Vec<u8>,
    pub size: usize,
    pub cs: Option<String>,
}

pub struct SpiFlash {
    pub config: SpiFlashConfig,
    name: String,
    reply: Option<Reply>,
    cmd: Option<(Command, Vec<u8>)>,
    write_enable: bool,
    page: Option<(usize, Vec<u8>, u8)>, // (addr, data, addr bytes collected)
    dbg_trace: Vec<u8>,
}

impl SpiFlash {
    pub fn trace_hex(&self) -> String {
        self.dbg_trace.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ")
    }
}

enum Reply {
    Data(VecDeque<u8>),
    FileContent(usize),
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    WriteEnable = 0x06,
    WriteDisable = 0x04,
    ReadId = 0x9F,
    ReadDeviceId = 0x90,
    ReadStatus1 = 0x05,
    ReadStatus2 = 0x35,
    WriteStatus1 = 0x01,
    WriteStatus2 = 0x31,
    PageProgram = 0x02,
    QuadPageProgram = 0x32,
    ReadData = 0x03,
    FastRead = 0x0B,
    SectorErase4k = 0x20,
    BlockErase32k = 0x52,
    BlockErase64k = 0xD8,
    ChipErase = 0xC7,
    DeepPowerDown = 0xB9,
    ReleasePowerDown = 0xAB,
}

impl TryFrom<u8> for Command {
    type Error = ();
    fn try_from(v: u8) -> Result<Self, ()> {
        use Command::*;
        match v {
            0x06 => Ok(WriteEnable),
            0x04 => Ok(WriteDisable),
            0x9F => Ok(ReadId),
            0x90 => Ok(ReadDeviceId),
            0x05 => Ok(ReadStatus1),
            0x35 => Ok(ReadStatus2),
            0x01 => Ok(WriteStatus1),
            0x31 => Ok(WriteStatus2),
            0x02 => Ok(PageProgram),
            0x32 => Ok(QuadPageProgram),
            0x03 => Ok(ReadData),
            0x0B => Ok(FastRead),
            0x20 => Ok(SectorErase4k),
            0x52 => Ok(BlockErase32k),
            0xD8 => Ok(BlockErase64k),
            0xC7 => Ok(ChipErase),
            0xB9 => Ok(DeepPowerDown),
            0xAB => Ok(ReleasePowerDown),
            _ => Err(()),
        }
    }
}

impl SpiFlash {
    pub fn new(config: SpiFlashConfig) -> Self {
        SpiFlash {
            config,
            name: String::new(),
            reply: None,
            cmd: None,
            write_enable: false,
            page: None,
            dbg_trace: Vec::new(),
        }
    }

    fn try_process_command(&mut self, cmd: Command, args: &[u8]) -> Option<Reply> {
        use Command::*;
        match cmd {
            ReadId => {
                let jedec = self.config.jedec_id;
                let mut reply = VecDeque::new();
                reply.push_back((jedec >> 16) as u8);
                reply.push_back((jedec >> 8) as u8);
                reply.push_back(jedec as u8);
                Some(Reply::Data(reply))
            }
            ReadDeviceId => {
                let mut reply = VecDeque::new();
                // 0x90: 3 dummy/address bytes then manufacturer ID then device ID
                reply.push_back(0xFF);
                reply.push_back(0xFF);
                reply.push_back(0xFF);
                reply.push_back(0xAA);
                reply.push_back(0xBB);
                Some(Reply::Data(reply))
            }
            ReadStatus1 => {
                let mut reply = VecDeque::new();
                reply.push_back(if self.write_enable { 0x02 } else { 0x00 }); // WEL bit
                Some(Reply::Data(reply))
            }
            ReadStatus2 => {
                let mut reply = VecDeque::new();
                reply.push_back(0x00);
                Some(Reply::Data(reply))
            }
            ReadData => {
                if args.len() == 4 {
                    // 3 address bytes latched; data streams from the 4th byte on
                    let addr = ((args[0] as usize) << 16) | ((args[1] as usize) << 8) | args[2] as usize;
                    Some(Reply::FileContent(addr % self.config.size))
                } else {
                    None
                }
            }
            FastRead => {
                if args.len() == 5 {
                    // 1 dummy byte, then 3 address bytes
                    let addr = ((args[2] as usize) << 16) | ((args[3] as usize) << 8) | args[4] as usize;
                    Some(Reply::FileContent(addr % self.config.size))
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

impl ExtDevice<(), u8> for SpiFlash {
    fn connect_peripheral(&mut self, peri_name: &str) -> String {
        self.name = format!("{} spi-flash", peri_name);
        self.name.clone()
    }

    fn read(&mut self, _sys: &System, _addr: ()) -> u8 {
        match self.reply.as_mut() {
            Some(Reply::Data(d)) => d.pop_front().unwrap_or_default(),
            Some(Reply::FileContent(addr)) => {
                let c = self.config.content[*addr];
                *addr = (*addr + 1) % self.config.size;
                c
            }
            None => 0,
        }
    }

    fn write(&mut self, _sys: &System, _addr: (), v: u8) {
        self.dbg_trace.push(v);
        if let Some((mut addr, mut data, mut collected)) = self.page.take() {
            if collected < 3 {
                addr = (addr << 8) | v as usize;
                collected += 1;
            } else {
                data.push(v);
            }
            self.page = Some((addr, data, collected));
            return;
        }
        if let Some((cmd, mut args)) = self.cmd.take() {
            args.push(v);
            if let Some(reply) = self.try_process_command(cmd, &args) {
                self.reply = Some(reply);
            } else {
                self.cmd = Some((cmd, args));
            }
        } else if let Ok(cmd) = Command::try_from(v) {
            match cmd {
                Command::WriteEnable => self.write_enable = true,
                Command::WriteDisable => self.write_enable = false,
                Command::PageProgram | Command::QuadPageProgram => {
                    self.page = Some((0, Vec::new(), 0));
                }
                _ => {
                    if let Some(reply) = self.try_process_command(cmd, &[]) {
                        self.reply = Some(reply);
                    } else {
                        self.cmd = Some((cmd, vec![]));
                    }
                }
            }
        }
    }

    fn cs_changed(&mut self, _sys: &System, selected: bool) {
        CS_CHANGES.fetch_add(1, Ordering::Relaxed);
        self.dbg_trace.push(if selected { 0xF0 } else { 0xF1 });
        if !selected {
            if let Some((addr, data, collected)) = self.page.take() {
                if collected >= 3 && self.write_enable && !data.is_empty() {
                    let size = self.config.content.len();
                    let start = addr % size;
                    let count = data.len().min(size - start);
                    self.config.content[start..start + count].copy_from_slice(&data[..count]);
                }
                self.write_enable = false;
            }
            self.reply = None;
            self.cmd = None;
        } else {
            self.reply = None;
            self.cmd = None;
            self.page = None;
        }
    }
}
