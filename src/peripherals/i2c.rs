use crate::{system::System, ext_devices::{ExtDevices, I2cDeviceEntry}};
use super::Peripheral;

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
    /// DMA channel: I2C1 TX=ch4/ch6 (remap), RX=ch5/ch7 (remap); I2C2=none
    dma_channel_tx: u8,
    dma_channel_rx: u8,
    /// Clock stretching: instruction count until SCL is released by the slave.
    /// If non-zero, interrupts are deferred until the stretch period expires.
    stretch_until: u64,
}

impl Default for I2c {
    fn default() -> Self {
        Self {
            name: String::new(), devices: Vec::new(), active_device: None,
            cr1: 0, cr2: 0, oar1: 0, oar2: 0, sr1: 0, sr2: 0, ccr: 0, trise: 0, dr: 0,
            state: I2cState::Idle, sr1_addr_flag: false,
            irq_ev: 0, irq_er: 0,
            dma_channel_tx: 0, dma_channel_rx: 0,
            stretch_until: 0,
        }
    }
}

impl I2c {
    pub fn new(name: &str, ext_devices: &ExtDevices) -> Option<Box<dyn Peripheral>> {
        if !name.starts_with("I2C") { return None; }
        let (irq_ev, irq_er) = i2c_irqs(name)?;
        let devices = ext_devices.find_i2c_devices(name);
        let (dma_tx, dma_rx) = match name {
            "I2C1" => (4, 5),  // default: DMA1 ch4(TX)/ch5(RX); AFIO remap shifts to ch6/ch7
            _ => (0, 0),
        };
        Some(Box::new(Self {
            name: name.to_string(), devices, irq_ev, irq_er,
            dma_channel_tx: dma_tx, dma_channel_rx: dma_rx,
            ..Default::default()
        }))
    }

    fn i2c_channel(&self) -> u8 {
        self.name.trim_start_matches("I2C").parse::<u8>().unwrap_or(0)
    }

    fn reset(&mut self) {
        self.sr1 = 0; self.sr2 = 0;
        self.active_device = None; self.state = I2cState::Idle;
        self.sr1_addr_flag = false;
    }

    fn fire_interrupts(&mut self, sys: &System) {
        // Clock stretching: defer interrupts while SCL is held low by a slave device.
        // The stretch_until is set when a byte/address transfer completes; this
        // method defers interrupt delivery until the stretch period expires.
        if self.stretch_until != 0 {
            let ic = crate::system::instruction_count();
            if ic < self.stretch_until {
                return; // defer until SCL released
            }
            self.stretch_until = 0; // stretch period expired
        }

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
                            let byte = sys.i2c_take_rx(self.i2c_channel()).unwrap_or_else(|| d.read(sys, ()) as u8);
                            self.dr = byte as u32;
                            self.sr1 |= 1 << 6; // RXNE
                            sys.push_event(crate::system::VmEvent::I2cRead { channel: self.i2c_channel() });
                            if self.cr2 & (1 << 11) != 0 && self.dma_channel_rx != 0 {
                                sys.p.dma_request(sys, self.dma_channel_rx as u32);
                            }
                        }
                    }
                    self.fire_interrupts(sys);
                    v
                }
             0x14 => {
                self.sr1_addr_flag = (self.sr1 & (1 << 1)) != 0;
                self.sr1
            }
            0x18 => {
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
                        if self.cr2 & (1 << 11) != 0 && self.dma_channel_rx != 0 {
                            sys.p.dma_request(sys, self.dma_channel_rx as u32);
                        }
                    } else {
                        self.sr1 |= 1 << 7; // TXE
                        self.sr2 |= 1 << 2; // TRA=1 (transmitter)
                        if self.cr2 & (1 << 11) != 0 && self.dma_channel_tx != 0 {
                            sys.p.dma_request(sys, self.dma_channel_tx as u32);
                        }
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
                    self.fire_interrupts(sys);
                }

                // STOP generation
                if stop != 0 {
                    sys.push_event(crate::system::VmEvent::I2cStop { channel: self.i2c_channel() });
                    if matches!(self.state, I2cState::Active { .. } | I2cState::AddrSent { .. }) {
                        let rxne_pending = self.sr1 & (1 << 6);
                        if matches!(self.state, I2cState::Active { is_read: true }) && rxne_pending != 0 {
                            // Master receiver: HAL writes STOP in the ADDR handler
                            // (single-byte reads: NACK + STOP immediately), then reads
                            // the byte via RXNE. Keep the pending byte readable.
                            self.state = I2cState::Idle;
                            self.active_device = None;
                            self.sr1_addr_flag = false;
                        } else {
                            self.reset();
                        }
                    } else {
                        // STOP in any other state (e.g. StartSent) — clear BUSY/MSL
                        self.reset();
                    }
                    self.cr1 &= !(1 << 9); // Clear STOP
                }
            }
            0x04 => {
                let prev_buf = self.cr2 & (1 << 10);
                self.cr2 = value & 0x07FF;
                if prev_buf != 0 && value & (1 << 10) == 0 {
                    if matches!(self.state, I2cState::Active { .. }) {
                        self.sr1 |= 1 << 2; // BTF
                    }
                }
                if value & (1 << 8 | 1 << 9 | 1 << 10) != 0 {
                    sys.p.nvic.borrow_mut().enable_irq(31);
                    sys.p.nvic.borrow_mut().enable_irq(32);
                }
                self.fire_interrupts(sys);
            }
            0x08 => self.oar1 = value & 0x3FFF,
            0x0C => self.oar2 = value & 0x3FF,
            0x10 => {
                match self.state {
                    I2cState::StartSent => {
                        let addr = ((value >> 1) & 0x7F) as u8;
                        let is_read = (value & 1) != 0;
                        sys.push_event(crate::system::VmEvent::I2cStart { channel: self.i2c_channel(), addr });
                        let found = self.devices.iter().position(|d| d.address == addr);

                        if let Some(idx) = found {
                            self.active_device = Some(idx);
                            self.devices[idx].device.borrow_mut().reset();
                            self.sr1 = 1 << 1; // ADDR
                            self.sr2 = (1 << 0) | (1 << 1); // BUSY=1, MSL=1
                            if is_read {
                                let mut d = self.devices[idx].device.borrow_mut();
                                self.dr = d.read(sys, ()) as u32;
                            }
                            self.state = I2cState::AddrSent { is_read };
                        } else {
                            // NACK: set AF (Acknowledge Failure, bit 10)
                            // Real HW generates STOP automatically on NACK, clearing BUSY/MSL
                            self.sr1 = 1 << 10;
                            self.sr2 = 0; // BUSY=0, MSL=0 (STOP generated)
                            self.state = I2cState::Idle;
                            self.active_device = None;
                            self.sr1_addr_flag = false;
                            sys.push_event(crate::system::VmEvent::I2cStop { channel: self.i2c_channel() });
                        }
                        self.fire_interrupts(sys);
                    }
                    I2cState::Active { is_read: false } => {
                        // Master transmitter: push byte
                        if let Some(idx) = self.active_device {
                            let mut d = self.devices[idx].device.borrow_mut();
                            d.write(sys, (), value as u8);
                            sys.push_event(crate::system::VmEvent::I2cWrite { channel: self.i2c_channel(), byte: value as u8 });
                            // Clock stretching: slave holds SCL low during write cycle
                            self.stretch_until = crate::system::instruction_count() + 200;
                        }
                        self.sr1 |= 1 << 7; // TXE
                        if self.cr2 & (1 << 11) != 0 && self.dma_channel_tx != 0 {
                            sys.p.dma_request(sys, self.dma_channel_tx as u32);
                        }
                        self.fire_interrupts(sys);
                    }
                    _ => {}
                }
            }
            0x1C => self.ccr = value & 0xFFF,
            0x20 => self.trise = value & 0x3F,
            _ => {}
        }
    }
}
