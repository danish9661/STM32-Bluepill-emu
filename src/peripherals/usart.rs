use crate::system::{System, get_uart_output, instruction_count};
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
    txe_clear_until: u64,
    /// DMA channel: 0=none, 4/5=USART1_TX/RX, 6/7=USART2_TX/RX
    dma_channel_tx: u8,
    dma_channel_rx: u8,
}

impl Usart {
    pub fn new(name: &str, _ext: &ExtDevices) -> Option<Box<dyn Peripheral>> {
        usart_irq(name).map(|irq| {
            // DMA channels: 1-7 = DMA1, 8-12 = DMA2 (offset by 8)
            let (tx_ch, rx_ch) = match name {
                "USART1" => (4, 5),
                "USART2" => (6, 7),
                "USART3" => (2, 3),  // DMA1 ch2=TX, ch3=RX
                "UART4" => (11, 12), // DMA2 ch3=TX(11), ch4=RX(12)
                "UART5" => (12, 13), // DMA2 ch4=TX(12), ch5=RX(13)
                _ => (0, 0),
            };
            Box::new(Self {
                name: name.to_string(),
                sr: 0x00C0,
                dr: 0, brr: 0, cr1: 0, cr2: 0, cr3: 0, gtp: 0,
                tx_data: Vec::new(),
                rx_buf: Vec::new(),
                irq_num: irq,
                txe_clear_until: 0,
                dma_channel_tx: tx_ch,
                dma_channel_rx: rx_ch,
            }) as Box<dyn Peripheral>
        })
    }

    /// Instructions for one byte to shift out at the programmed baud rate
    /// (10 bits/frame, 1 instr = 1 cycle @ 72 MHz).
    fn byte_time(&self) -> u64 {
        let b = self.brr as u64;
        if b == 0 { 6250 } else { (b * 10).max(1000) }
    }

    fn txe_ready(&self) -> bool {
        self.txe_clear_until == 0 || instruction_count() >= self.txe_clear_until
    }

    fn refresh_txe(&mut self) {
        if self.sr & 0x80 == 0 && self.txe_ready() {
            self.sr |= 0x80;
        }
    }

    fn update_interrupt(&mut self, sys: &System) {
        let mut pending = false;
        if self.cr1 & (1 << 6) != 0 && self.sr & (1 << 6) != 0 { pending = true; } // TCIE + TC
        // TXEIE + TXE: the ISR drains the core's software TX ring; the
        // 16-IRQ-per-batch cap bounds re-pending, and the core ISR clears
        // TXEIE once the ring empties, so no storm is possible.
        if self.cr1 & (1 << 7) != 0 && self.sr & (1 << 7) != 0 { pending = true; }
        if self.cr1 & (1 << 5) != 0 && self.sr & (1 << 5) != 0 { pending = true; } // RXNEIE + RXNE
        if pending {
            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
        }
    }

    fn read_sr(&mut self) -> u32 {
        self.refresh_txe();
        self.sr
    }

    fn read_dr(&mut self, sys: &System) -> u32 {
        self.refresh_txe();
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
        self.sr |= 0x40; // TC stays set
        self.update_interrupt(sys);
        dr
    }

    fn is_loopback(&self) -> bool {
        self.cr3 & (1 << 2) != 0 // HDSEL = half-duplex -> software loopback
    }

    fn rx_push(&mut self, byte: u8, sys: &System) {
        self.refresh_txe();
        if self.rx_buf.len() < 16 {
            self.rx_buf.push(byte);
            self.sr |= 1 << 5; // RXNE
        } else {
            self.sr |= 1 << 3; // ORE
        }
        self.sr |= 0x40; // TC stays set
        self.update_interrupt(sys);
        // DMA request when DMAR (CR3 bit 6) is set
        if self.cr3 & (1 << 6) != 0 && self.dma_channel_rx != 0 {
            sys.p.dma_request(sys, self.dma_channel_rx as u32);
        }
    }

    fn usart_num(&self) -> u8 {
        match self.name.as_str() {
            "USART1" => 1, "USART2" => 2, "USART3" => 3, "UART4" => 4,
            "UART5" => 5, "USART6" => 6, "UART7" => 7, "UART8" => 8,
            _ => 0,
        }
    }

    fn write_dr(&mut self, value: u32, sys: &System) {
        let ch = (value & 0xFF) as u8;
        self.tx_data.push(ch);
        get_uart_output().lock().unwrap().push(ch as char);
        sys.push_event(crate::system::VmEvent::UartTx { usart: self.usart_num(), byte: ch });
        self.sr |= 0x40; // TC stays set
        if self.is_loopback() {
            self.sr |= 0x80; // TXE: loopback echoes instantly
            self.dr = ch as u32;
            self.sr |= 1 << 5; // RXNE: receive the looped byte
            self.update_interrupt(sys);
        } else {
            // TXE re-asserts immediately: polling firmware (HAL blocking
            // transmit) drains at instruction rate. The interrupt path is
            // still spaced by byte_time via update_interrupt()'s txe_ready()
            // gate, so ISR-driven transmitters cannot re-pend every batch.
            self.sr |= 0x80;
            self.txe_clear_until = instruction_count() + self.byte_time();
            self.update_interrupt(sys);
        }
        // DMA request when DMAT (CR3 bit 7) is set
        if self.cr3 & (1 << 7) != 0 && self.dma_channel_tx != 0 {
            sys.p.dma_request(sys, self.dma_channel_tx as u32);
        }
    }
}

impl Peripheral for Usart {
    fn tick(&mut self, sys: &System) {
        if self.sr & 0x80 == 0 && self.txe_ready() {
            self.sr |= 0x80;
        }
        self.update_interrupt(sys);
    }

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

    fn rx_pending(&self) -> u32 {
        self.rx_buf.len() as u32
    }
}
