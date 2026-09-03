use crate::system::{System, instruction_count};
use super::Peripheral;

const RTC_IRQ: i32 = 3;

pub struct Rtc {
    crh: u32,
    crl: u32,
    prlh: u32,
    prll: u32,
    divh: u32,
    divl: u32,
    cnth: u32,
    cntl: u32,
    alrh: u32,
    alrl: u32,
    cnt: u32,
    div_counter: u32,
    last_tick: u64,
    last_cnt: u32,
}

impl Rtc {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "RTC" {
            Some(Box::new(Rtc {
                crh: 0,
                crl: 0x0020,
                prlh: 0,
                prll: 0x7FFF,
                divh: 0,
                divl: 0,
                cnth: 0,
                cntl: 0,
                alrh: 0xFFFF,
                alrl: 0xFFFF,
                cnt: 0,
                div_counter: 0,
                last_tick: instruction_count(),
                last_cnt: 0,
            }))
        } else {
            None
        }
    }

    fn prescaler(&self) -> u32 {
        let p = ((self.prlh as u32) << 16) | self.prll as u32;
        if p == 0 { 1 } else { p }
    }

    fn sync_cnt_regs(&mut self) {
        self.cnth = (self.cnt >> 16) as u32;
        self.cntl = self.cnt as u32;
    }

    fn sync_div_regs(&mut self) {
        let prl = self.prescaler();
        let rem = if prl > 0 { prl - self.div_counter % prl } else { 0 };
        self.divh = (rem >> 16) as u32;
        self.divl = rem as u32;
    }
}

impl Peripheral for Rtc {
    fn tick(&mut self, sys: &System) {
        if self.crl & (1 << 5) == 0 { return; } // RTOFF — not enabled

        let now = instruction_count();
        let delta = now.wrapping_sub(self.last_tick);
        if delta == 0 { return; }
        self.last_tick = now;

        let prl = self.prescaler();
        let steps = delta as u32;
        self.div_counter += steps;

        let inc = self.div_counter / prl;
        if inc > 0 {
            self.div_counter %= prl;
            self.cnt = self.cnt.wrapping_add(inc);
            self.sync_cnt_regs();
            self.sync_div_regs();

            // CRH interrupt enables (RM0008 §18.4.1): bit 0 SECIE, bit 1
            // ALRIE, bit 2 OWIE. CRL status flags SECF/ALRF/OWF (bits 0-2)
            // are set here and cleared by writing 0 (see write path).
            let alarm = ((self.alrh as u32) << 16) | self.alrl as u32;
            if self.crh & 2 != 0 && self.last_cnt < alarm && self.cnt >= alarm {
                self.crl |= 1 << 1; // ALRF
                sys.push_event(crate::system::VmEvent::RtcAlarm { alarm });
                sys.p.nvic.borrow_mut().set_intr_pending(RTC_IRQ);
            }
            if self.crh & 1 != 0 {
                self.crl |= 1 << 0; // SECF: a second elapsed
                sys.p.nvic.borrow_mut().set_intr_pending(RTC_IRQ);
            }
            if self.cnt < self.last_cnt {
                self.crl |= 1 << 2; // OWF: counter wrapped
                if self.crh & 4 != 0 {
                    sys.p.nvic.borrow_mut().set_intr_pending(RTC_IRQ);
                }
            }
            self.last_cnt = self.cnt;
        }
    }

    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.crh,
            0x04 => self.crl,
            0x08 => self.prlh,
            0x0C => self.prll,
            0x10 => self.divh,
            0x14 => self.divl,
            0x18 => self.cnth,
            0x1C => self.cntl,
            0x20 => self.alrh,
            0x24 => self.alrl,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.crh = value & 0x07, // SECIE/ALRIE/OWIE
            0x04 => {
                // Status flags SECF/ALRF/OWF (bits 0-2): hardware-set here,
                // cleared by writing 0 (writing 1 leaves them).
                self.crl &= value | !0x07;
                // RSF (bit 3): set by hardware after register sync; writing
                // 0 clears (writing 1 has no effect).
                if value & (1 << 3) == 0 {
                    self.crl &= !(1 << 3);
                }
                if value & (1 << 4) != 0 {
                    self.crl |= 1 << 4;
                }
                let rtoff = value & (1 << 5);
                if rtoff != 0 {
                    self.crl = (self.crl & !(1 << 5) & !(1 << 4)) | (value & (1 << 5));
                }
                let was_enabled = self.crl & (1 << 5);
                if (self.crl & (1 << 5)) != 0 && was_enabled == 0 {
                    self.last_tick = instruction_count();
                    self.div_counter = 0;
                }
            }
            0x08 => self.prlh = value & 0xFFFF,
            0x0C => self.prll = value & 0xFFFF,
            0x10 => self.divh = value & 0xFFFF,
            0x14 => self.divl = value & 0xFFFF,
            0x18 => {
                self.cnth = value & 0xFFFF;
                self.cnt = ((self.cnth as u32) << 16) | (self.cntl as u32);
                self.last_cnt = self.cnt;
            }
            0x1C => {
                self.cntl = value & 0xFFFF;
                self.cnt = ((self.cnth as u32) << 16) | (self.cntl as u32);
                self.last_cnt = self.cnt;
            }
            0x20 => self.alrh = value & 0xFFFF,
            0x24 => self.alrl = value & 0xFFFF,
            _ => {}
        }
    }
}
