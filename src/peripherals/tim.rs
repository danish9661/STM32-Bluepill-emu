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

        let dir = (self.cr1 >> 4) & 1;
        let cms = (self.cr1 >> 5) & 0x3;

        for _ in 0..ticks.min(1000) {
            let old_cnt = self.cnt;
            match (cms, dir) {
                (0, 0) => { // Up-counting
                    if self.cnt < self.arr { self.cnt += 1; }
                    else {
                        self.cnt = 0;
                        self.sr |= 1; // UIF
                        if self.dier & 1 != 0 { // UIE
                            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
                        }
                        if self.dier & (1 << 8) != 0 { //UDE - DMA request
                            // would trigger DMA
                        }
                    }
                }
                (0, 1) => { // Down-counting
                    if self.cnt > 0 { self.cnt -= 1; }
                    else {
                        self.cnt = self.arr;
                        self.sr |= 1; // UIF
                        if self.dier & 1 != 0 {
                            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
                        }
                    }
                }
                _ => { // Center-aligned modes
                    if self.cnt < self.arr { self.cnt += 1; }
                    else {
                        self.cnt = 0;
                        self.sr |= 1;
                        if self.dier & 1 != 0 {
                            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
                        }
                    }
                }
            }

            // Output compare / PWM interrupts
            for ch in 0..4 {
                if self.ccer & (1 << (ch * 4)) != 0 { // CCxE
                    let ccr_val = self.ccr[ch];
                    let match_up = dir == 0 && old_cnt <= ccr_val && self.cnt >= ccr_val;
                    let match_down = dir == 1 && old_cnt >= ccr_val && self.cnt <= ccr_val;
                    let match_overflow = (old_cnt > self.cnt) && (old_cnt <= ccr_val || self.cnt >= ccr_val);
                    if match_up || match_down || match_overflow {
                        self.sr |= 1 << (1 + ch); // CC1IF-CC4IF
                        let cc_irq_enable = (self.dier >> (1 + ch)) & 1;
                        if cc_irq_enable != 0 {
                            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
                        }
                    }
                }
            }
        }

        // Update PWM duty based on CCR/ARR
        for ch in 0..4 {
            if self.ccer & (1 << (ch * 4)) != 0 && self.arr > 0 {
                self.pwm_duty[ch] = self.ccr[ch] * 100 / (self.arr + 1);
            }
        }

        self.update_interrupt(sys);
    }

    fn update_interrupt(&self, _sys: &System) {
        // UIF, CCxIF, TIF, etc. already trigger during advance
    }

    fn generate_update(&mut self, sys: &System) {
        self.cnt = 0;
        self.sr |= 1; // UIF
        if self.dier & 1 != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
        }
    }
}

impl Peripheral for Timer {
    fn tick(&mut self, sys: &System) {
        self.advance(sys);
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
