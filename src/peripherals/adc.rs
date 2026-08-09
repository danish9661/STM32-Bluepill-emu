use crate::system::{System, instruction_count};
use super::Peripheral;
use std::sync::atomic::{AtomicU16, Ordering};

const ADC_IRQ: i32 = 18;

static ADC_SIM_VALUE: AtomicU16 = AtomicU16::new(0x3FF);

/// RC sample-and-hold time constant in instructions (ADC cycles).
/// The sampling capacitor charges toward the pin voltage as
/// V(t) = Vc0 + (Vpin - Vc0) * (1 - e^(-t/tau)). Default 12 cycles.
static ADC_RC_TAU: AtomicU16 = AtomicU16::new(12);

pub fn set_adc_value(val: u16) {
    ADC_SIM_VALUE.store(val, Ordering::Relaxed);
}

pub fn set_adc_rc_tau(cycles: u16) {
    ADC_RC_TAU.store(if cycles == 0 { 1 } else { cycles }, Ordering::Relaxed);
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

/// Map an ADC channel to its GPIO pin (F103: ch0-7 = PA0-7, ch8-9 = PB0-1,
/// ch10-15 = PC0-5, 16 = temp sensor, 17 = VREFINT, 18 = VBAT).
fn channel_pin(ch: u8) -> Option<(u8, u8)> {
    match ch {
        0..=7 => Some((0, ch)),
        8..=9 => Some((1, ch - 8)),
        10..=15 => Some((2, ch - 10)),
        _ => None,
    }
}

/// Nominal internal-channel values: temp sensor ~25C (0x1F8), VREFINT 1.2 V
/// (0x5D2), VBAT (0xC7F) — emulated statistically, not modeled.
fn nominal_channel(ch: u8) -> Option<u32> {
    match ch {
        16 => Some(0x1F8),
        17 => Some(0x5D2),
        18 => Some(0xC7F),
        _ => None,
    }
}

#[derive(Default, Clone)]
struct Conv {
    end_at: u64,     // instruction count when the current conversion completes
    pos: usize,      // index into the current sequence
    len: usize,      // sequence length
    cycles: u32,     // conversion cycles for this channel (sample + convert)
    cap_start: u32,  // sampling capacitor voltage at the start of the sample window
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
            cap_voltage: 0,
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
    /// Sampling-capacitor voltage (12-bit) held between conversions — the
    /// first sample after reset charges from 0 toward the source voltage.
    cap_voltage: u32,
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

    /// Target voltage (0..4095) for a channel at sample time.
    /// If an analog voltage is configured on the mapped pin (or the channel
    /// is internal 16/17/18), the RC sample-and-hold path engages; otherwise
    /// the injected simulation value is sampled directly (legacy behavior).
    fn channel_voltage(&self, sys: &System, ch: u8) -> (u32, bool) {
        if let Some((port, pin)) = channel_pin(ch) {
            let gpio = sys.p.gpio.borrow();
            if let Some(v) = gpio.analog_pin_value(port, pin) {
                return (v as u32 & 0xFFF, true);
            }
        }
        if let Some(v) = nominal_channel(ch) {
            return (v, true);
        }
        (ADC_SIM_VALUE.load(Ordering::Relaxed) as u32 & 0xFFF, false)
    }

    /// RC sample-and-hold settle: the cap charges from its held voltage toward
    /// the source over `cycles` (1 instr = 1 cycle). Returns the 12-bit
    /// voltage the converter samples and updates the held cap level.
    fn rc_settle(&mut self, from: u32, to: u32, cycles: u32) -> u32 {
        let tau = ADC_RC_TAU.load(Ordering::Relaxed) as u32;
        let frac = 1.0 - (-(cycles as f64) / (tau as f64)).exp();
        let v = from as f64 + (to as f64 - from as f64) * frac;
        let v = v.round().max(0.0).min(4095.0) as u32;
        self.cap_voltage = v;
        v
    }

    fn start_regular(&mut self, sys: &System) {
        if !self.adc_on() || self.conv.is_some() { return; }
        let len = ((self.sqr1 >> 16) & 0xF) as usize + 1;
        let ch = self.regular_channel(0);
        let cycles = self.sample_time(ch);
        let _ = self.channel_voltage(sys, ch); // source resolved at completion
        self.conv = Some(Conv {
            end_at: instruction_count() + cycles as u64,
            pos: 0,
            len,
            cycles,
            cap_start: self.cap_voltage,
        });
    }

    fn start_injected(&mut self, sys: &System) {
        if !self.adc_on() || self.jconv.is_some() { return; }
        let len = ((self.jsqr >> 20) & 0x3) as usize + 1;
        let ch = self.injected_channel(0);
        let cycles = self.sample_time(ch);
        let (target, _real) = self.channel_voltage(sys, ch);
        self.jconv = Some(Conv {
            end_at: instruction_count() + cycles as u64,
            pos: 0,
            len,
            cycles,
            cap_start: self.cap_voltage,
        });
    }

    /// Complete the conversion `c` and schedule the next one in the sequence.
    fn advance_regular(&mut self, sys: &System, c: &Conv, now: u64) {
        // For any channel the value settles via RC from the cap's held level;
        // the simulation source (adc_set_sim_value) is served exactly, so the
        // legacy tests and firmware keep their deterministic readings.
        let (target, real) = {
            let ch = self.regular_channel(c.pos);
            self.channel_voltage(sys, ch)
        };
        let _ = c;
        self.dr = if real {
            self.rc_settle(c.cap_start, target, c.cycles)
        } else {
            target
        };
        let last = c.pos + 1 >= c.len;
        // EOC: per conversion unless EOCS (CR2 bit 10) moves it to sequence end
        if self.cr2 & (1 << 10) == 0 || last {
            self.sr |= 1 << 1; // EOC
        }
        if c.pos == 0 {
            self.sr |= 1 << 4; // STRT at sequence start
        }
        // Analog watchdog: compare conversion result against HTR/LTR
        if self.cr1 & 1 != 0 && (self.dr > self.htr || self.dr < self.ltr) {
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
                self.start_regular(sys); // CONT: restart the sequence
            }
        } else {
            let ch = self.regular_channel(c.pos + 1);
            let cycles = self.sample_time(ch);
            self.conv = Some(Conv {
                end_at: now + cycles as u64,
                pos: c.pos + 1,
                len: c.len,
                cycles,
                cap_start: self.cap_voltage,
            });
        }
    }

    fn advance_injected(&mut self, sys: &System, c: &Conv, now: u64) {
        let (target, real) = {
            let ch = self.injected_channel(c.pos);
            self.channel_voltage(sys, ch)
        };
        let dr = if real {
            self.rc_settle(c.cap_start, target, c.cycles)
        } else {
            target
        };
        if c.pos < 4 {
            self.jdata[c.pos] = dr;
        }
        self.sr |= 1 << 2; // JEOC
        if c.pos == 0 {
            self.sr |= 1 << 3; // JSTRT at sequence start
        }
        if self.cr1 & 1 != 0 && (dr > self.htr || dr < self.ltr) {
            self.sr |= 1; // AWD
        }
        self.fire_interrupts(sys);
        let last = c.pos + 1 >= c.len;
        if last {
            self.jconv = None;
            if self.cr2 & (1 << 16) != 0 {
                self.start_injected(sys);
            }
        } else {
            let ch = self.injected_channel(c.pos + 1);
            let cycles = self.sample_time(ch);
            self.jconv = Some(Conv {
                end_at: now + cycles as u64,
                pos: c.pos + 1,
                len: c.len,
                cycles,
                cap_start: self.cap_voltage,
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
                    self.start_regular(sys); // SWSTART
                }
                if value & (1 << 21) != 0 {
                    self.start_injected(sys); // JSWSTART
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
