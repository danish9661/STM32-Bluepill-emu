use crate::system::{System, instruction_count};
use super::Peripheral;

fn tim_irq(name: &str) -> Option<i32> {
    match name {
        "TIM1" => Some(24), "TIM2" => Some(28), "TIM3" => Some(29),
        "TIM4" => Some(30), "TIM5" => Some(50), "TIM6" => Some(54),
        "TIM7" => Some(55), "TIM8" => Some(70), "TIM9" => Some(20),
        "TIM10" => Some(25), "TIM11" => Some(26), "TIM12" => Some(43),
        "TIM13" => Some(54), "TIM14" => Some(51),
        _ => None,
    }
}

fn timer_base(name: &str) -> u32 {
    match name {
        "TIM1" => 0x4001_2C00, "TIM8" => 0x4001_3400,
        "TIM9" => 0x4001_4C00, "TIM10" => 0x4001_5000, "TIM11" => 0x4001_5400,
        "TIM2" => 0x4000_0000, "TIM3" => 0x4000_0400, "TIM4" => 0x4000_0800,
        "TIM5" => 0x4000_0C00, "TIM6" => 0x4000_1000, "TIM7" => 0x4000_1400,
        "TIM12" => 0x4000_1800, "TIM13" => 0x4000_1C00, "TIM14" => 0x4000_2000,
        _ => 0,
    }
}

/// Default + AFIO-remapped channel -> GPIO pin mapping for STM32F103 timers.
/// `remap` is the AFIO MAPR remap code for the timer (0 = default).
/// Returns (port, pin) where port: 0=A, 1=B, 2=C, 3=D.
fn tim_chan_pin(name: &str, ch: u8, remap: u32) -> Option<(u8, u8)> {
    let pins: &[(u8, u8)] = match name {
        "TIM1" => &[(0, 8), (0, 9), (0, 10), (0, 11)], // no remap on Bluepill F103C8
        "TIM2" => match remap & 3 {
            0 => &[(0, 0), (0, 1), (0, 2), (0, 3)],
            1 => &[(0, 15), (1, 3), (1, 10), (1, 11)],
            2 => &[(0, 0), (0, 1), (1, 10), (1, 11)],
            _ => &[(0, 15), (1, 3), (0, 2), (0, 3)],
        },
        "TIM3" => match remap & 3 {
            0 => &[(0, 6), (0, 7), (1, 0), (1, 1)],
            1 => &[(1, 4), (1, 5), (1, 0), (1, 1)],
            _ => &[(2, 6), (2, 7), (2, 8), (2, 9)], // full remap (PC6..PC9)
        },
        "TIM4" => if remap & 1 != 0 {
            &[(3, 12), (3, 13), (3, 14), (3, 15)] // PD12..PD15
        } else {
            &[(1, 6), (1, 7), (1, 8), (1, 9)]      // PB6..PB9
        },
        "TIM5" => &[(0, 0), (0, 1), (0, 2), (0, 3)],
        "TIM8" => &[(2, 6), (2, 7), (2, 8), (2, 9)],
        "TIM9" => &[(0, 2), (0, 3)],
        "TIM10" => &[(1, 8)],
        "TIM11" => &[(1, 9)],
        "TIM12" => &[(1, 14), (1, 15)],
        "TIM13" => &[(0, 6)],
        "TIM14" => &[(1, 1)],
        _ => return None,
    };
    pins.get(ch as usize).copied()
}

pub struct Timer {
    cr1: u32,
    cr2: u32,
    smcr: u32,
    dier: u32,
    sr: u32,
    egr: u32,
    ccmr1: u32,
    ccmr2: u32,
    ccer: u32,
    cnt: u32,
    psc: u32,
    arr: u32,
    ccr: [u32; 4],
    rcr: u32,
    dcr: u32,
    dmar: u32,
    or_: u32,
    // Extended
    ccmr3: u32,
    ccr5: u32,
    ccr6: u32,
    pwm_duty: [u32; 4],
    last_tick: u64,
    irq_num: i32,
    base: u32,
    // Input-capture support: last sampled pin level + edge counter + init flag per ch.
    last_cap: [bool; 4],
    cap_count: [u32; 4],
    cap_inited: [bool; 4],
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    one_pulse_active: bool,
}

impl Timer {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        tim_irq(name).map(|irq| {
            Box::new(Self {
                cr1: 0, cr2: 0, smcr: 0, dier: 0, sr: 0, egr: 0,
                ccmr1: 0, ccmr2: 0, ccer: 0, cnt: 0, psc: 0,
                arr: 0xFFFF_FFFF,
                ccr: [0; 4], rcr: 0, dcr: 0, dmar: 0, or_: 0,
                 ccmr3: 0, ccr5: 0, ccr6: 0, pwm_duty: [0; 4],
                 last_tick: instruction_count(),
                 irq_num: irq,
                 base: timer_base(name),
                 last_cap: [false; 4], cap_count: [0; 4], cap_inited: [false; 4],
                name: name.to_string(),
                one_pulse_active: false,
            }) as Box<dyn Peripheral>
        })
    }

    fn prescaler(&self) -> u64 {
        (self.psc as u64).saturating_add(1)
    }

    fn elapsed_ticks(&self) -> u64 {
        let now = instruction_count();
        let delta = now.wrapping_sub(self.last_tick);
        delta / self.prescaler()
    }

    fn advance(&mut self, sys: &System) {
        let ticks = self.elapsed_ticks();
        if ticks == 0 { return; }
        self.last_tick = instruction_count();

        let enabled = self.cr1 & 1;
        if enabled == 0 { return; }

        let cms = (self.cr1 >> 5) & 0x3;
        let down = cms == 0 && (self.cr1 >> 4) & 1 == 1;
        let arr = self.arr as u64;

        // Closed-form advance: skip no-event ticks in bulk and only execute the
        // per-tick body at event ticks (update wrap + CCx compare matches). CNT
        // is only observable at batch boundaries (step_batch), and all events
        // (UIF/CCxIF/TRGO/DMA-request) pend into batch-boundary queues, so the
        // final CNT + fired events are bit-identical to iterating every tick —
        // without the O(ticks) cost (3 active timers × 20K ticks × 4 channels
        // dominated step_batch: ~14.5% of runtime).
        let mut remaining = ticks;
        while remaining > 0 {
            let cnt = self.cnt as u64;
            let mut next = if down {
                // update fires on the tick AFTER cnt reaches 0 (wrap tick);
                // if cnt >= remaining the wrap lies beyond this batch
                if cnt >= remaining { remaining } else { cnt + 1 }
            } else if cnt < arr {
                arr - cnt + 1
            } else {
                1 // cnt >= arr: next tick wraps
            };
            for ch in 0..4 {
                if self.ccer & (1 << (ch * 4)) == 0 { continue; }
                let ccr = self.ccr[ch] as u64;
                // CCx match fires on the tick where cnt crosses ccr (old==ccr
                // also matches); ccr already passed only re-fires via the wrap
                // tick's overflow check in tick_once. Down-mode ccr==0 at cnt==0
                // can't match: that tick wraps (new=arr, no match_down).
                let d = if down {
                    if cnt > ccr { cnt - ccr }
                    else if cnt == ccr { if ccr == 0 { u64::MAX } else { 1 } }
                    else { u64::MAX }
                } else if cnt <= ccr {
                    (ccr - cnt).max(1)
                } else {
                    u64::MAX
                };
                next = next.min(d);
            }
            next = next.min(remaining);
            if next > 1 {
                if down { self.cnt = (cnt - (next - 1)) as u32; }
                else { self.cnt = (cnt + next - 1) as u32; }
                remaining -= next - 1;
            }
            self.tick_once(sys);
            remaining -= 1;
        }

        // Update PWM duty based on CCR/ARR
        for ch in 0..4 {
            if self.ccer & (1 << (ch * 4)) != 0 && self.arr != u32::MAX {
                self.pwm_duty[ch] = self.ccr[ch] * 100 / (self.arr + 1);
            }
        }

        self.update_interrupt(sys);
    }

    /// The original per-tick loop body, executed only at event ticks.
    fn tim_num(&self) -> u8 {
        self.name.trim_start_matches("TIM").parse::<u8>().unwrap_or(0)
    }

    fn tick_once(&mut self, sys: &System) {
        let cms = (self.cr1 >> 5) & 0x3;
        let down = cms == 0 && (self.cr1 >> 4) & 1 == 1;
        let old_cnt = self.cnt;
        let arr = self.arr as u64;
        let cnt = old_cnt as u64;

        if down {
            if old_cnt > 0 { self.cnt -= 1; }
            else {
                self.cnt = self.arr;
                self.sr |= 1; // UIF
                self.update_event_trigger(sys);
                sys.push_event(crate::system::VmEvent::TimUpdate { tim: self.tim_num() });
                if self.dier & 1 != 0 { // UIE
                    sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
                }
                if self.dier & (1 << 8) != 0 { //UDE - DMA request
                    // would trigger DMA
                }
            }
        } else if cnt < arr {
            self.cnt += 1;
        } else {
            self.cnt = 0;
            self.sr |= 1; // UIF
            self.update_event_trigger(sys);
            sys.push_event(crate::system::VmEvent::TimUpdate { tim: self.tim_num() });
            if self.dier & 1 != 0 { // UIE
                sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
            }
            if self.dier & (1 << 8) != 0 { //UDE - DMA request
                // would trigger DMA
            }
        }

        // Output compare / PWM interrupts (skip input-capture channels)
        for ch in 0..4 {
            let ccmr = if ch < 2 { self.ccmr1 } else { self.ccmr2 };
            let off = if ch < 2 { ch } else { ch - 2 };
            let ccs = (ccmr >> (off * 8)) & 3;
            if ccs != 0 { continue; } // input capture mode
            if self.ccer & (1 << (ch * 4)) != 0 { // CCxE
                let ccr_val = self.ccr[ch];
                let new_cnt = self.cnt;
                let match_up = !down && old_cnt <= ccr_val && new_cnt >= ccr_val;
                let match_down = down && old_cnt >= ccr_val && new_cnt <= ccr_val;
                let match_overflow = (old_cnt > new_cnt) && (old_cnt <= ccr_val || new_cnt >= ccr_val);
                if match_up || match_down || match_overflow {
                    self.sr |= 1 << (1 + ch); // CC1IF-CC4IF
                    // ADC external trigger on channel compare events
                    sys.p.adc_timer_trigger(sys, self.base, ch as u8);
                    let cc_irq_enable = (self.dier >> (1 + ch)) & 1;
                    if cc_irq_enable != 0 {
                        sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
                    }
                }
            }
        }
    }

    fn update_event_trigger(&mut self, sys: &System) {
        // TRGO fires on update when MMS = 010 (update)
        if (self.cr2 >> 4) & 7 == 2 {
            sys.p.adc_timer_trigger(sys, self.base, 4);
        }
    }

    fn update_interrupt(&self, _sys: &System) {
        // UIF, CCxIF, TIF, etc. already trigger during advance
    }

    fn generate_update(&mut self, sys: &System) {
        self.cnt = 0;
        self.sr |= 1; // UIF
        self.update_event_trigger(sys);
        sys.push_event(crate::system::VmEvent::TimUpdate { tim: self.tim_num() });
        if self.dier & 1 != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
        }
    }

    /// Sample input-capture channel pins once per batch. When a channel is
    /// configured for input capture (CCxS != 0) and an edge matching its
    /// polarity occurs on the source pin, latch CNT into CCRx, set CCxIF, and
    /// emit a TimCapture event.
    fn sample_input_capture(&mut self, sys: &System) {
        for ch in 0..4u8 {
            let ccmr = if ch < 2 { self.ccmr1 } else { self.ccmr2 };
            let off = if ch < 2 { ch } else { ch - 2 } as u32;
            let ccs = (ccmr >> (off * 8)) & 3;
            if ccs == 0 { continue; } // output compare mode
            let src_ch = if ccs == 1 { ch } else { ch ^ 1 }; // CCxS=10 -> partner pin
            let psc = ((ccmr >> (off * 8 + 2)) & 3) as u32; // ICxPSC prescaler
            let remap = sys.p.afio_remap_status(&self.name).unwrap_or(0);
            let (port, pin) = match tim_chan_pin(&self.name, src_ch, remap) { Some(p) => p, None => continue };
            let level = sys.p.gpio.borrow_mut().read_pin_effective(sys, port, pin);
            if !self.cap_inited[ch as usize] {
                self.cap_inited[ch as usize] = true;
                self.last_cap[ch as usize] = level;
                continue;
            }
            let prev = self.last_cap[ch as usize];
            self.last_cap[ch as usize] = level;
            if level == prev { continue; }
            let ccer_bit = (ch as usize) * 4;
            let cxp = (self.ccer >> (ccer_bit + 1)) & 1;
            let cxnp = (self.ccer >> (ccer_bit + 3)) & 1;
            // CCxP=0 & CCXNP=0 -> rising; CCxP=1 & CCXNP=1 -> both; else falling
            let rising_ok = (cxp == 0 && cxnp == 0) || (cxp == 1 && cxnp == 1);
            let falling_ok = cxp == 1;
            let is_rising = level && !prev;
            let is_falling = !level && prev;
            if !((is_rising && rising_ok) || (is_falling && falling_ok)) { continue; }
            self.cap_count[ch as usize] += 1;
            if self.cap_count[ch as usize] % (psc + 1) != 0 { continue; }
            self.ccr[ch as usize] = self.cnt;
            self.sr |= 1 << (1 + ch as u32); // CCxIF
            sys.push_event(crate::system::VmEvent::TimCapture { tim: self.tim_num(), ch, value: self.cnt });
            if (self.dier >> (1 + ch as u32)) & 1 != 0 {
                sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
            }
        }
    }
}

impl Peripheral for Timer {
    fn periph_remap(&self, sys: &System) -> Option<u32> {
        sys.p.afio_remap_status(&self.name)
    }

    fn pwm_duty(&self, channel: u32) -> Option<u32> {
        self.pwm_duty.get(channel as usize).copied()
    }

    fn tick(&mut self, sys: &System) {
        self.sample_input_capture(sys);
        self.advance(sys);
    }

    /// Frozen in STOP/STANDBY: sync the instruction-delta base without
    /// advancing the counter (the timer clock is gated in deep sleep).
    fn tick_frozen(&mut self, _sys: &System) {
        self.last_tick = instruction_count();
    }

    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        if offset == 0x24 { return self.cnt; }
        self.advance(sys);
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.smcr,
            0x0C => self.dier,
            0x10 => self.sr,
            0x14 => {
                // EGR reads as 0
                self.egr
            }
            0x18 => self.ccmr1,
            0x1C => self.ccmr2,
            0x20 => self.ccer,
            0x24 => self.cnt,
            0x28 => self.psc,
            0x2C => self.arr,
            0x30 => self.rcr,
            0x34..=0x40 => {
                let i = ((offset - 0x34) / 4) as usize;
                self.ccr.get(i).copied().unwrap_or(0)
            }
            0x48 => self.dcr,
            0x4C => self.dmar,
            0x50 => self.or_,
            0x54 => self.ccmr3,
            0x58 => self.ccr5,
            0x5C => self.ccr6,
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        self.advance(sys);
        match offset {
            0x00 => {
                let was_enabled = self.cr1 & 1;
                self.cr1 = value & 0xFFFE_F17F;
                if self.cr1 & 1 != 0 && was_enabled == 0 {
                    // Enable: reset counter to 0
                    self.cnt = 0;
                }
            }
            0x04 => self.cr2 = value & 0x3F7F,
            0x08 => self.smcr = value & 0xFFFF,
            0x0C => {
                self.dier = value & 0xFFFF;
                if value & 1 != 0 {
                    sys.p.nvic.borrow_mut().enable_irq(self.irq_num);
                }
                self.update_interrupt(sys);
            }
            0x10 => self.sr &= value,
            0x14 => {
                self.egr = value & 0xFF;
                if value & 1 != 0 { self.generate_update(sys); } // UG
            }
            0x18 => self.ccmr1 = value,
            0x1C => self.ccmr2 = value,
            0x20 => self.ccer = value & 0xFFFF,
            0x24 => self.cnt = value & 0xFFFF,
            0x28 => self.psc = value & 0xFFFF,
            0x2C => self.arr = value & 0xFFFFFFFF,
            0x30 => self.rcr = value & 0xFF,
            0x34..=0x40 => {
                let i = ((offset - 0x34) / 4) as usize;
                if let Some(ccr) = self.ccr.get_mut(i) {
                    *ccr = value & 0xFFFF;
                }
            }
            0x48 => self.dcr = value & 0x1F1F,
            0x4C => self.dmar = value,
            0x50 => self.or_ = value & 0xFF,
            0x54 => self.ccmr3 = value,
            0x58 => self.ccr5 = value & 0xFFFF,
            0x5C => self.ccr6 = value & 0xFFFF,
            _ => {}
        }
    }
}
