use crate::{system::System, ext_devices::{ExtDevices, I2cDeviceEntry}};
use super::Peripheral;

macro_rules! i2clog {
    ($($arg:tt)*) => { crate::i2c_log(format!($($arg)*)); };
}

#[derive(Clone, PartialEq, Debug)]
enum I2cState { Idle, StartSent, AddrSent { is_read: bool }, Active { is_read: bool } }

impl Default for I2cState { fn default() -> Self { I2cState::Idle } }

fn i2c_irqs(name: &str) -> Option<(i32, i32)> {
    match name {
        "I2C1" => Some((31, 32)),
        "I2C2" => Some((33, 34)),
        "I2C3" => Some((72, 73)),
        _ => None,
    }
}

#[derive(Clone)]
pub struct I2c {
    #[allow(dead_code)]
    name: String,
    devices: Vec<I2cDeviceEntry>,
    active_device: Option<usize>,
    cr1: u32,
    cr2: u32,
    oar1: u32,
    oar2: u32,
    sr1: u32,
    sr2: u32,
    ccr: u32,
    trise: u32,
    dr: u32,
    state: I2cState,
    sr1_addr_flag: bool,
    irq_ev: i32,
    irq_er: i32,
}

impl Default for I2c {
    fn default() -> Self {
        Self {
            name: String::new(), devices: Vec::new(), active_device: None,
            cr1: 0, cr2: 0, oar1: 0, oar2: 0, sr1: 0, sr2: 0, ccr: 0, trise: 0, dr: 0,
            state: I2cState::Idle, sr1_addr_flag: false,
            irq_ev: 0, irq_er: 0,
        }
    }
}

impl I2c {
    pub fn new(name: &str, ext_devices: &ExtDevices) -> Option<Box<dyn Peripheral>> {
        if !name.starts_with("I2C") { return None; }
        let (irq_ev, irq_er) = i2c_irqs(name)?;
        let devices = ext_devices.find_i2c_devices(name);
        Some(Box::new(Self { name: name.to_string(), devices, irq_ev, irq_er, ..Default::default() }))
    }

    fn reset(&mut self) {
        self.sr1 = 0; self.sr2 = 0;
        self.active_device = None; self.state = I2cState::Idle;
        self.sr1_addr_flag = false;
    }

    fn fire_interrupts(&mut self, sys: &System) {
        let itevten = (self.cr2 >> 9) & 1;  // bit 9 = ITEVTEN
        let iterren = (self.cr2 >> 8) & 1;  // bit 8 = ITERREN
        let itbufen = (self.cr2 >> 10) & 1; // bit 10 = ITBUFEN

        let ev_flags = self.sr1 & 0x17;
        let buf_flags = self.sr1 & 0xC0;
        let err_flags = self.sr1 & 0x0E00;

        if ev_flags != 0 && itevten != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_ev);
        }
        if buf_flags != 0 && itbufen != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_ev);
        }
        if err_flags != 0 && iterren != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_er);
        }
    }
}

impl Peripheral for I2c {
    fn periph_remap(&self, sys: &System) -> Option<u32> {
        sys.p.afio_remap_status(&self.name)
    }

    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.oar1,
            0x0C => self.oar2,
            0x1C => self.ccr,
            0x20 => self.trise,
            0x10 => {
                let v = self.dr;
                self.sr1 &= !(1 << 6); // Clear RXNE on DR read
                if let Some(idx) = self.active_device {
                    if matches!(self.state, I2cState::Active { is_read: true }) {
                        let mut d = self.devices[idx].device.borrow_mut();
                        self.dr = d.read(sys, ()) as u32;
                        self.sr1 |= 1 << 6; // RXNE
                        i2clog!("[I2C] RX read DR=0x{:02X}, preloaded next=0x{:02X}", v, self.dr);
                    } else {
                        i2clog!("[I2C] DR read in non-read state {:?}, returning 0x{:02X}", self.state, v);
                    }
                } else {
                    i2clog!("[I2C] DR read with no active device, returning 0x{:02X}", v);
                }
                self.fire_interrupts(sys);
                v
            }
             0x14 => {
                self.sr1_addr_flag = (self.sr1 & (1 << 1)) != 0;
                i2clog!("[I2C] SR1 read: 0x{:04X} (SB={} ADDR={} TXE={} RXNE={} BTF={})", self.sr1, self.sr1 & 1, (self.sr1 >> 1) & 1, (self.sr1 >> 7) & 1, (self.sr1 >> 6) & 1, (self.sr1 >> 2) & 1);
                self.sr1
            }
            0x18 => {
                i2clog!("[I2C] SR2 read: addr_flag={} state={:?} sr1=0x{:04X}", self.sr1_addr_flag, self.state, self.sr1);
                // Reading SR2 clears ADDR flag
                if self.sr1_addr_flag {
                    self.sr1 &= !(1 << 1); // Clear ADDR
                    self.sr1_addr_flag = false;
                    let is_read = match std::mem::replace(&mut self.state, I2cState::Idle) {
                        I2cState::AddrSent { is_read } => {
                            self.state = I2cState::Active { is_read };
                            is_read
                        }
                        s => { self.state = s; false }
                    };
                    if is_read {
                        self.sr1 |= 1 << 6; // RXNE
                        self.sr2 &= !(1 << 2); // TRA=0 (receiver)
                    } else {
                        self.sr1 |= 1 << 7; // TXE
                        self.sr2 |= 1 << 2; // TRA=1 (transmitter)
                    }
                    self.fire_interrupts(sys);
                }
                self.sr2
            }
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                let prev_start = self.cr1 & (1 << 8);
                let prev_pe = self.cr1 & 1;
                self.cr1 = value;

                // SW reset (bit 15)
                if value & (1 << 15) != 0 {
                    self.reset();
                    self.cr1 = value & 1;
                    return;
                }
                // Disable (PE=0)
                if prev_pe != 0 && value & 1 == 0 {
                    self.reset();
                    return;
                }

                let start = value & (1 << 8);
                let stop = value & (1 << 9);

                // START generation
                if start != 0 && prev_start == 0 {
                    self.state = I2cState::StartSent;
                    self.sr1 = 1; // SB
                    self.sr2 = (1 << 0) | (1 << 1); // BUSY=1, MSL=1
                    self.active_device = None;
                    self.cr1 &= !(1 << 8); // Clear START
                    i2clog!("[I2C] START generated, cr2=0x{:03X}", self.cr2);
                    self.fire_interrupts(sys);
                }

                // STOP generation
                if stop != 0 {
                    if matches!(self.state, I2cState::Active { .. } | I2cState::AddrSent { .. }) {
                        i2clog!("[I2C] STOP generated, state was {:?}, cr2=0x{:03X}", self.state, self.cr2);
                        self.reset();
                    }
                    self.cr1 &= !(1 << 9); // Clear STOP
                }
            }
            0x04 => {
                let prev_buf = self.cr2 & (1 << 10);
                let old_cr2 = self.cr2;
                self.cr2 = value & 0x07FF;
                i2clog!("[I2C] CR2 write: 0x{:03X} -> 0x{:03X} (ITEVTEN={})", old_cr2, self.cr2, (self.cr2 >> 9) & 1);
                if prev_buf != 0 && value & (1 << 10) == 0 {
                    if matches!(self.state, I2cState::Active { .. }) {
                        self.sr1 |= 1 << 2; // BTF
                        i2clog!("[I2C] BUF IT disabled → BTF set, state={:?}", self.state);
                    }
                }
                if value & (1 << 8 | 1 << 9 | 1 << 10) != 0 {
                    sys.p.nvic.borrow_mut().enable_irq(31);
                    sys.p.nvic.borrow_mut().enable_irq(32);
                    i2clog!("[I2C] CR2 write enabled interrupts: irq31_enabled={} irq32_enabled={}", 
                        sys.p.nvic.borrow().is_enabled(31), sys.p.nvic.borrow().is_enabled(32));
                }
                self.fire_interrupts(sys);
                i2clog!("[I2C] after fire_interrupts: pending_31={}", sys.p.nvic.borrow().is_pending(31));
            }
            0x08 => self.oar1 = value & 0x3FFF,
            0x0C => self.oar2 = value & 0x3FF,
            0x10 => {
                match self.state {
                    I2cState::StartSent => {
                        let addr = ((value >> 1) & 0x7F) as u8;
                        let is_read = (value & 1) != 0;
                        let found = self.devices.iter().position(|d| d.address == addr);

                        if let Some(idx) = found {
                            self.active_device = Some(idx);
                            self.devices[idx].device.borrow_mut().reset();
                            self.sr1 = 1 << 1; // ADDR
                            self.sr2 = (1 << 0) | (1 << 1); // BUSY=1, MSL=1
                            i2clog!("[I2C] ADDR sent: 0x{:02X} ({}), found dev {}, is_read={}", addr, if is_read {"R"} else {"W"}, idx, is_read);
                            if is_read {
                                let mut d = self.devices[idx].device.borrow_mut();
                                self.dr = d.read(sys, ()) as u32;
                            }
                            self.state = I2cState::AddrSent { is_read };
                        } else {
                            i2clog!("[I2C] ADDR NACK: 0x{:02X}", addr);
                            // NACK: set AF (Acknowledge Failure, bit 10)
                            self.sr1 = 1 << 10;
                            self.sr2 = (1 << 0) | (1 << 1);
                            self.state = I2cState::Idle;
                        }
                        self.fire_interrupts(sys);
                    }
                    I2cState::Active { is_read: false } => {
                        // Master transmitter: push byte
                        if let Some(idx) = self.active_device {
                            let mut d = self.devices[idx].device.borrow_mut();
                            d.write(sys, (), value as u8);
                            i2clog!("[I2C] TX byte: 0x{:02X} (chr={})", value, value as u8);
                        }
                        self.sr1 |= 1 << 7; // TXE
                        self.fire_interrupts(sys);
                    }
                    _ => {
                        i2clog!("[I2C] DR write in unexpected state {:?}, val=0x{:02X}", self.state, value);
                    }
                }
            }
            0x1C => self.ccr = value & 0xFFF,
            0x20 => self.trise = value & 0x3F,
            _ => {}
        }
    }
}
