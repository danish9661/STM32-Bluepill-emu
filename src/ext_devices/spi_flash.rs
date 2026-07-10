use std::collections::VecDeque;
use std::convert::TryFrom;
use crate::system::System;
use super::ExtDevice;

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
                reply.push_back(0x00);
                Some(Reply::Data(reply))
            }
            ReadStatus2 => {
                let mut reply = VecDeque::new();
                reply.push_back(0x00);
                Some(Reply::Data(reply))
            }
            ReadData | FastRead => {
                let addr = match args.len() {
                    3 => ((args[0] as usize) << 16) | ((args[1] as usize) << 8) | args[2] as usize,
                    4 => ((args[1] as usize) << 16) | ((args[2] as usize) << 8) | args[3] as usize,
                    _ => 0,
                };
                Some(Reply::FileContent(addr % self.config.size))
            }
            _ => None,
        }
    }

    fn write_enabled(&self) -> bool { false }
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
        if let Some((cmd, mut args)) = self.cmd.take() {
            args.push(v);
            if let Some(reply) = self.try_process_command(cmd, &args) {
                self.reply = Some(reply);
            } else {
                self.cmd = Some((cmd, args));
            }
        } else if let Some(cmd) = Command::try_from(v).ok() {
            if let Some(reply) = self.try_process_command(cmd, &[]) {
                self.reply = Some(reply);
            } else {
                self.cmd = Some((cmd, vec![]));
            }
        }
    }
}
