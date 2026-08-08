use crate::system::{System, get_uart_output};
use crate::ext_devices::ExtDevices;
use super::Peripheral;

fn usart_irq(name: &str) -> Option<i32> {
    match name {
        "USART1" => Some(37),
        "USART2" => Some(38),
        "USART3" => Some(39),
        "UART4" => Some(52),
        "UART5" => Some(53),
        "USART6" => Some(71),
        "UART7" => Some(82),
        "UART8" => Some(83),
        _ => None,
    }
}

pub struct Usart {
    name: String,
    sr: u32,
    dr: u32,
    brr: u32,
    cr1: u32,
    cr2: u32,
    cr3: u32,
    gtp: u32,
    tx_data: Vec<u8>,
    rx_buf: Vec<u8>,
    irq_num: i32,
}

impl Usart {
    pub fn new(name: &str, _ext: &ExtDevices) -> Option<Box<dyn Peripheral>> {
        usart_irq(name).map(|irq| {
            Box::new(Self {
                name: name.to_string(),
                sr: 0x00C0,
                dr: 0, brr: 0, cr1: 0, cr2: 0, cr3: 0, gtp: 0,
                tx_data: Vec::new(),
                rx_buf: Vec::new(),
                irq_num: irq,
            }) as Box<dyn Peripheral>
        })
    }

    fn update_interrupt(&mut self, sys: &System) {
        let mut pending = false;
        if self.cr1 & (1 << 6) != 0 && self.sr & (1 << 6) != 0 { pending = true; } // TCIE + TC
        if self.cr1 & (1 << 7) != 0 && self.sr & (1 << 7) != 0 { pending = true; } // TXEIE + TXE
        if self.cr1 & (1 << 5) != 0 && self.sr & (1 << 5) != 0 { pending = true; } // RXNEIE + RXNE
        if pending {
            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
        }
    }

    fn read_sr(&mut self) -> u32 {
        let sr = self.sr;
        self.sr |= 0x00C0; // TXE and TC stay set after reading SR
        sr
    }

    fn read_dr(&mut self, sys: &System) -> u32 {
        let dr = if self.is_loopback() {
            // HDSEL: RX pin disconnected, DR reflects the looped TX byte;
            // external bytes stay queued for later.
            self.dr
        } else if !self.rx_buf.is_empty() {
            self.rx_buf.remove(0) as u32
        } else {
            self.dr
        };
        if self.rx_buf.is_empty() && !self.is_loopback() {
            self.sr &= !(1 << 5); // Clear RXNE only when buffer empty
        }
        self.sr |= 0x00C0; // TXE, TC
        self.update_interrupt(sys);
        dr
    }

    fn is_loopback(&self) -> bool {
        self.cr3 & (1 << 2) != 0 // HDSEL = half-duplex -> software loopback
    }

    fn rx_push(&mut self, byte: u8, sys: &System) {
        if self.rx_buf.len() < 16 {
            self.rx_buf.push(byte);
            self.sr |= 1 << 5; // RXNE
        } else {
            self.sr |= 1 << 3; // ORE
        }
        self.sr |= 0x00C0; // TXE, TC
        self.update_interrupt(sys);
    }

    fn write_dr(&mut self, value: u32, sys: &System) {
        let ch = (value & 0xFF) as u8;
        self.tx_data.push(ch);
        get_uart_output().lock().unwrap().push(ch as char);
        self.sr |= 0x00C0; // TXE=1, TC=1
        if self.is_loopback() {
            self.dr = ch as u32;
            self.sr |= 1 << 5; // RXNE: receive the looped byte
        } else {
            self.update_interrupt(sys);
        }
    }
}

impl Peripheral for Usart {
    fn periph_remap(&self, sys: &System) -> Option<u32> {
        sys.p.afio_remap_status(&self.name)
    }

    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => {
                let v = self.read_sr();
                log::debug!("USART read SR={:#x}", v);
                v
            }
            0x04 => self.read_dr(sys),
            0x08 => self.brr,
            0x0C => self.cr1,
            0x10 => self.cr2,
            0x14 => self.cr3,
            0x18 => self.gtp,
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {} // SR writes only clear some bits via read
            0x04 => self.write_dr(value, sys),
            0x08 => self.brr = value,
            0x0C => {
                self.cr1 = value & 0xFFFF;
                self.update_interrupt(sys);
            }
            0x10 => self.cr2 = value & 0xFFFF,
            0x14 => self.cr3 = value & 0xFFFF,
            0x18 => self.gtp = value,
            _ => {}
        }
    }

    fn rx_byte(&mut self, sys: &System, byte: u8) {
        self.rx_push(byte, sys);
    }
}
