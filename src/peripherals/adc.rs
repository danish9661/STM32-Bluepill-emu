use crate::system::{System, instruction_count};
use super::Peripheral;
use std::sync::atomic::{AtomicU16, Ordering};

const ADC_IRQ: i32 = 18;

static ADC_SIM_VALUE: AtomicU16 = AtomicU16::new(0x3FF);

pub fn set_adc_value(val: u16) {
    ADC_SIM_VALUE.store(val, Ordering::Relaxed);
}

/// Conversion cycles for a sample-time code: Tconv = (SMP + 12.5) ADC cycles.
/// With a 1-instr = 1-cycle time base the values are exact integers.
fn conv_cycles(smp: u32) -> u32 {
    match smp & 7 {
        0 => 14,   // 1.5
        1 => 20,   // 7.5
        2 => 26,   // 13.5
        3 => 41,   // 28.5
        4 => 54,   // 41.5
        5 => 68,   // 55.5
        6 => 84,   // 71.5
        _ => 252,  // 239.5
    }
}

#[derive(Default, Clone)]
struct Conv {
    end_at: u64,     // instruction count when the current conversion completes
    pos: usize,      // index into the current sequence
    len: usize,      // sequence length
    dr: u32,         // sampled value of the channel under conversion
}

impl Default for Adc {
    fn default() -> Self {
        Self {
            sr: 0,
            cr1: 0,
            cr2: 0,
            smpr1: 0,
            smpr2: 0,
            jofr: [0; 4],
            htr: 0x0FFF,
            ltr: 0,
            sqr1: 0,
            sqr2: 0,
            sqr3: 0,
            jsqr: 0,
            jdata: [0; 4],
            dr: 0,
            ofr: [0; 4],
            conv: None,
            jconv: None,
            dma_channel: 0,
        }
    }
}
pub struct Adc {
    sr: u32,
    cr1: u32,
    cr2: u32,
    smpr1: u32,
    smpr2: u32,
    jofr: [u32; 4],
    htr: u32,
    ltr: u32,
    sqr1: u32,
    sqr2: u32,
    sqr3: u32,
    jsqr: u32,
    jdata: [u32; 4],
    dr: u32,
    #[allow(dead_code)]
    ofr: [u32; 4],
    conv: Option<Conv>,
    jconv: Option<Conv>,
    dma_channel: u8,
}

impl Adc {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name.starts_with("ADC") {
            // ADC1 -> DMA1 ch1, ADC2 -> DMA1 ch2 (F103 request mapping)
            let ch = if name == "ADC2" { 2 } else { 1 };
            Some(Box::new(Adc { dma_channel: ch, ..Self::default() }))
        } else {
            None
        }
    }

    fn adc_on(&self) -> bool { self.cr2 & 1 != 0 }

    fn fire_interrupts(&mut self, sys: &System) {
        let mut any = false;
        if self.sr & 1 != 0 && self.cr1 & (1 << 6) != 0 { any = true; } // AWD + AWDIE
        if self.sr & (1 << 1) != 0 && self.cr1 & (1 << 5) != 0 { any = true; } // EOC + EOCIE
        if self.sr & (1 << 2) != 0 && self.cr1 & (1 << 7) != 0 { any = true; } // JEOC + JEOCIE
        if any {
            sys.p.nvic.borrow_mut().set_intr_pending(ADC_IRQ);
        }
    }

    fn sample_time(&self, channel: u8) -> u32 {
        let bits = if channel < 10 {
            (self.smpr2 >> (channel as u32 * 3)) & 7
        } else {
            (self.smpr1 >> ((channel as u32 - 10) * 3)) & 7
        };
        conv_cycles(bits)
    }

    fn regular_channel(&self, pos: usize) -> u8 {
        // SQ1..SQ16: SQR3[9:0], SQR3[19:10], SQR3[29:20], SQR2[9:0]... SQR1[9:0]
        let pos = pos as u32;
        match pos {
            0 => (self.sqr3 & 0x1F) as u8,
            1 => ((self.sqr3 >> 10) & 0x1F) as u8,
            2 => ((self.sqr3 >> 20) & 0x1F) as u8,
            3 => (self.sqr2 & 0x1F) as u8,
            4 => ((self.sqr2 >> 10) & 0x1F) as u8,
            5 => ((self.sqr2 >> 20) & 0x1F) as u8,
            6 => (self.sqr1 & 0x1F) as u8,
            7 => ((self.sqr1 >> 10) & 0x1F) as u8,
            8 => ((self.sqr1 >> 20) & 0x1F) as u8,
            _ => 0,
        }
    }

    fn injected_channel(&self, pos: usize) -> u8 {
        // JSQ1..JSQ4: JSQR[4:0], [9:5], [14:10], [19:15]
        match pos as u32 {
            0 => (self.jsqr & 0x1F) as u8,
            1 => ((self.jsqr >> 5) & 0x1F) as u8,
            2 => ((self.jsqr >> 10) & 0x1F) as u8,
            _ => ((self.jsqr >> 15) & 0x1F) as u8,
        }
    }

    fn start_regular(&mut self) {
        if !self.adc_on() || self.conv.is_some() { return; }
        let len = ((self.sqr1 >> 16) & 0xF) as usize + 1;
        let ch = self.regular_channel(0);
        self.conv = Some(Conv {
            end_at: instruction_count() + self.sample_time(ch) as u64,
            pos: 0,
            len,
            dr: ADC_SIM_VALUE.load(Ordering::Relaxed) as u32 & 0xFFF,
        });
    }

    fn start_injected(&mut self) {
        if !self.adc_on() || self.jconv.is_some() { return; }
        let len = ((self.jsqr >> 20) & 0x3) as usize + 1;
        let ch = self.injected_channel(0);
        self.jconv = Some(Conv {
            end_at: instruction_count() + self.sample_time(ch) as u64,
            pos: 0,
            len,
            dr: ADC_SIM_VALUE.load(Ordering::Relaxed) as u32 & 0xFFF,
        });
    }

    /// Complete the conversion `c` and schedule the next one in the sequence.
    fn advance_regular(&mut self, sys: &System, c: &Conv, now: u64) {
        self.dr = c.dr;
        let last = c.pos + 1 >= c.len;
        // EOC: per conversion unless EOCS (CR2 bit 10) moves it to sequence end
        if self.cr2 & (1 << 10) == 0 || last {
            self.sr |= 1 << 1; // EOC
        }
        if c.pos == 0 {
            self.sr |= 1 << 4; // STRT at sequence start
        }
        // Analog watchdog: compare conversion result against HTR/LTR
        if self.cr1 & 1 != 0 && (c.dr > self.htr || c.dr < self.ltr) {
            self.sr |= 1; // AWD
        }
        self.fire_interrupts(sys);
        // DMA request (F103: ADC1->DMA1 ch1, ADC2->ch2)
        if self.cr2 & (1 << 15) != 0 {
            sys.p.dma_request(sys, self.dma_channel as u32);
        }
        if last {
            self.conv = None;
            if self.cr2 & (1 << 16) != 0 {
                self.start_regular(); // CONT: restart the sequence
            }
        } else {
            let ch = self.regular_channel(c.pos + 1);
            self.conv = Some(Conv {
                end_at: now + self.sample_time(ch) as u64,
                pos: c.pos + 1,
                len: c.len,
                dr: ADC_SIM_VALUE.load(Ordering::Relaxed) as u32 & 0xFFF,
            });
        }
    }

    fn advance_injected(&mut self, sys: &System, c: &Conv, now: u64) {
        if c.pos < 4 {
            self.jdata[c.pos] = c.dr;
        }
        self.sr |= 1 << 2; // JEOC
        if c.pos == 0 {
            self.sr |= 1 << 3; // JSTRT at sequence start
        }
        if self.cr1 & 1 != 0 && (c.dr > self.htr || c.dr < self.ltr) {
            self.sr |= 1; // AWD
        }
        self.fire_interrupts(sys);
        let last = c.pos + 1 >= c.len;
        if last {
            self.jconv = None;
            if self.cr2 & (1 << 16) != 0 {
                self.start_injected();
            }
        } else {
            let ch = self.injected_channel(c.pos + 1);
            self.jconv = Some(Conv {
                end_at: now + self.sample_time(ch) as u64,
                pos: c.pos + 1,
                len: c.len,
                dr: ADC_SIM_VALUE.load(Ordering::Relaxed) as u32 & 0xFFF,
            });
        }
    }
}

impl Peripheral for Adc {
    fn tick(&mut self, sys: &System) {
        let now = instruction_count();
        loop {
            let done = self.conv.as_ref().filter(|c| now >= c.end_at).cloned();
            match done {
                Some(c) => self.advance_regular(sys, &c, now),
                None => break,
            }
        }
        loop {
            let done = self.jconv.as_ref().filter(|c| now >= c.end_at).cloned();
            match done {
                Some(c) => self.advance_injected(sys, &c, now),
                None => break,
            }
        }
        self.fire_interrupts(sys);
    }

    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.sr,
            0x04 => self.cr1,
            0x08 => self.cr2,
            0x0C => self.smpr1,
            0x10 => self.smpr2,
            0x14 => self.jofr[0],
            0x18 => self.jofr[1],
            0x1C => self.jofr[2],
            0x20 => self.jofr[3],
            0x24 => self.htr,
            0x28 => self.ltr,
            0x2C => self.sqr1,
            0x30 => self.sqr2,
            0x34 => self.sqr3,
            0x38 => self.jsqr,
            0x3C => self.jdata[0],
            0x40 => self.jdata[1],
            0x44 => self.jdata[2],
            0x48 => self.jdata[3],
            0x4C => {
                self.sr &= !(1 << 1); // clear EOC on DR read
                self.fire_interrupts(sys);
                self.dr
            }
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.sr = value & 0x3F,
            0x04 => { self.cr1 = value; self.fire_interrupts(sys); }
            0x08 => {
                // CAL (bit 2) and RSTCAL (bit 3) self-clear on real hardware
                self.cr2 = value & !((1 << 2) | (1 << 3));
                if value & (1 << 22) != 0 {
                    self.start_regular(); // SWSTART
                }
                if value & (1 << 21) != 0 {
                    self.start_injected(); // JSWSTART
                }
            }
            0x0C => self.smpr1 = value,
            0x10 => self.smpr2 = value,
            0x14 => self.jofr[0] = value,
            0x18 => self.jofr[1] = value,
            0x1C => self.jofr[2] = value,
            0x20 => self.jofr[3] = value,
            0x24 => self.htr = value & 0xFFF,
            0x28 => self.ltr = value & 0xFFF,
            0x2C => self.sqr1 = value,
            0x30 => self.sqr2 = value,
            0x34 => self.sqr3 = value,
            0x38 => self.jsqr = value,
            _ => {}
        }
    }
}
