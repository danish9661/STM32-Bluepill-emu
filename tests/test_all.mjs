import { readFileSync } from 'fs';
import * as periph from '../pkg/stm32_bluepill_wasm.js';
periph.initSync({ module: readFileSync(new URL('../pkg/stm32_bluepill_wasm_bg.wasm', import.meta.url)) });

const { init, init_svd, periph_read, periph_write, tick, step_batch, has_pending_interrupt,
        get_next_pending_interrupt, clear_current_interrupt, gpio_read_output, gpio_set_input,
        gpio_read_input, get_uart_output, uart_rx_byte, adc_set_sim_value,
        is_watchdog_reset_requested, can_inject_message, gpio_set_slew, raise_fault,
        add_fsmc_bank, gpio_set_analog, adc_set_rc_tau, register_js_peripheral,
        add_sd_card, reset_ext_devices,
        gpio_take_pin_events } = periph;

let passed = 0, failed = 0;

function assert(cond, msg) {
  if (cond) { passed++; }
  else { failed++; console.error(`FAIL: ${msg}`); }
}

function assert_eq(a, b, msg) {
  if (a === b) { passed++; }
  else { failed++; console.error(`FAIL: ${msg}: expected ${b}, got ${a}`); }
}

function assert_neq(a, b, msg) {
  if (a !== b) { passed++; }
  else { failed++; console.error(`FAIL: ${msg}: both are ${a}`); }
}

function reset() {
  init();
}

function group(name) {
  console.log(`\n=== ${name} ===`);
}

// ============================================================
// GPIO
// ============================================================
group('GPIO');

reset();
// PC13: GPIOC base=0x40011000, ODR=0x0C, BSRR=0x10, CRH=0x04
// Set PC13 as output push-pull 50MHz via CRH bits 23:20 = 0x3
let crh = periph_read(0x40011004, 4);
crh = (crh & ~(0xF << 20)) | (0x3 << 20);
periph_write(0x40011004, 4, crh);
assert_eq(periph_read(0x40011004, 4) >> 20 & 0xF, 0x3, 'GPIO CRH PC13 mode');

// Set PC13 high via BSRR bit 13
periph_write(0x40011010, 4, 1 << 13);
assert_eq(periph_read(0x4001100C, 4) >> 13 & 1, 1, 'GPIO PC13 set via BSRR');
assert_eq(gpio_read_output(2, 13), true, 'gpio_read_output PC13 after set');

// Reset PC13 via BSRR bit 29 (= 13+16)
periph_write(0x40011010, 4, 1 << 29);
assert_eq(periph_read(0x4001100C, 4) >> 13 & 1, 0, 'GPIO PC13 reset via BSRR');
assert_eq(gpio_read_output(2, 13), false, 'gpio_read_output PC13 after reset');

// GPIO input: set PA0 as input, inject value, read back
// GPIOA base=0x40010800, CRL=0x00, IDR=0x08
let gpioa_crl = periph_read(0x40010800, 4);
gpioa_crl = (gpioa_crl & ~0xF) | 0x4; // PA0 = input floating
periph_write(0x40010800, 4, gpioa_crl);

gpio_set_input(0, 0, true);  // PA0 high
assert_eq(gpio_read_input(0, 0), true, 'GPIO PA0 input set high');

gpio_set_input(0, 0, false); // PA0 low
assert_eq(gpio_read_input(0, 0), false, 'GPIO PA0 input set low');

// BSRR should only affect set bits — verify no stray change
periph_write(0x40011010, 4, 0);
assert_eq(periph_read(0x4001100C, 4) & 0x2000, 0, 'GPIO BSRR=0 no change');

// ============================================================
// GPIO pin-change events (gpio_take_pin_events)
// ============================================================
group('GPIO pin events');
reset(); // init clears the event buffer

// CRL/CRH mode change to output with ODR=0: re-drives to the same level — silent
let gpioc_crh = periph_read(0x40011004, 4);
gpioc_crh = (gpioc_crh & ~(0xF << 20)) | (0x3 << 20); // PC13 output PP 50MHz
periph_write(0x40011004, 4, gpioc_crh);
assert_eq(gpio_take_pin_events().length, 0, 'pin event: CRH->output same level silent');

// BSRR set fires (2, 13, 1)
periph_write(0x40011010, 4, 1 << 13);
let ev = gpio_take_pin_events();
assert_eq(ev.length, 3, 'pin event: BSRR set emits one triple');
assert_eq(ev[0] === 2 && ev[1] === 13 && ev[2] === 1, true, 'pin event: BSRR set = (2,13,1)');

// Same-level BSRR set is silent (old==value guard in write_port)
periph_write(0x40011010, 4, 1 << 13);
assert_eq(gpio_take_pin_events().length, 0, 'pin event: same-value BSRR set silent');

// BSRR reset fires (2, 13, 0)
periph_write(0x40011010, 4, 1 << 29);
ev = gpio_take_pin_events();
assert_eq(ev.length === 3 && ev[2] === 0, true, 'pin event: BSRR reset = (2,13,0)');

// ODR same-value write is silent (iter_port_reg_changes skips unchanged pins)
periph_write(0x4001100C, 4, 0);
assert_eq(gpio_take_pin_events().length, 0, 'pin event: same-value ODR write silent');

// ODR change write fires
periph_write(0x4001100C, 4, 1 << 13);
ev = gpio_take_pin_events();
assert_eq(ev.length === 3 && ev[2] === 1, true, 'pin event: ODR change = (2,13,1)');
// CRL/CRH re-drive: input mode (ODR writes don't drive), then back to output —
// the pin re-drives ODR=0 while the wire was high → event (2,13,0)
gpioc_crh = (gpioc_crh & ~(0xF << 20)) | (0x4 << 20); // PC13 input floating
periph_write(0x40011004, 4, gpioc_crh);
assert_eq(gpio_take_pin_events().length, 0, 'pin event: input mode change silent');
periph_write(0x4001100C, 4, 0); // ODR=0 while input (no drive)
assert_eq(gpio_take_pin_events().length, 0, 'pin event: ODR write while input silent');
gpioc_crh = (gpioc_crh & ~(0xF << 20)) | (0x3 << 20); // back to output
periph_write(0x40011004, 4, gpioc_crh);
ev = gpio_take_pin_events();
assert_eq(ev.length === 3 && ev[2] === 0, true, 'pin event: CRH re-drive fires (2,13,0)');

// AF pins (cnf=0b10) emit nothing: PA7 AF push-pull, toggle ODR bit 7
let gpioa_crl2 = periph_read(0x40010800, 4);
gpioa_crl2 = (gpioa_crl2 & ~(0xF << 28)) | (0xB << 28); // PA7 AF output PP 50MHz
periph_write(0x40010800, 4, gpioa_crl2);
periph_write(0x40010810, 4, 1 << 7); // BSRR set PA7
periph_write(0x40010810, 4, 1 << 23); // BSRR reset PA7
assert_eq(gpio_take_pin_events().length, 0, 'pin event: AF pin toggles emit nothing');

// Drain twice: second drain is empty
assert_eq(gpio_take_pin_events().length, 0, 'pin event: drain empties the buffer');

// ============================================================
// USART (UART)
// ============================================================
group('USART');

reset();
const USART1 = 0x40013800;

// Enable clocks: USART1EN on APB2, GPIOAEN
periph_write(0x40021018, 4, (1 << 14) | (1 << 2)); // APB2ENR

// Configure PA9 as AFIO push-pull 50MHz (CRH bits 7:4 = 0xB)
let pa_crh = periph_read(0x40010804, 4);
pa_crh = (pa_crh & ~0xF0) | 0xB0;
periph_write(0x40010804, 4, pa_crh);

// Configure PA10 as input float (CRH bits 11:8 = 0x4)
pa_crh = (pa_crh & ~0xF00) | 0x400;
periph_write(0x40010804, 4, pa_crh);

// Set baud = 115200 @8MHz → BRR = 0x341
periph_write(USART1 + 0x08, 4, 0x341);
// Enable USART: UE | TE | RE
periph_write(USART1 + 0x0C, 4, (1 << 13) | (1 << 3) | (1 << 2));

// TX: write a byte, read SR (TXE should be set), check output buffer
let sr = periph_read(USART1 + 0x00, 4);
assert_eq(sr & (1 << 7), 1 << 7, 'USART SR TXE after init');

periph_write(USART1 + 0x04, 4, 0x41); // 'A'
assert_eq(get_uart_output(), 'A', 'USART TX output char A');

periph_write(USART1 + 0x04, 4, 0x42); // 'B'
periph_write(USART1 + 0x04, 4, 0x43); // 'C'
assert_eq(get_uart_output(), 'BC', 'USART TX output chars BC');

// UART output accumulates across reads
periph_write(USART1 + 0x04, 4, 0x58); // 'X'
periph_write(USART1 + 0x04, 4, 0x59); // 'Y'
periph_write(USART1 + 0x04, 4, 0x5A); // 'Z'
assert_eq(get_uart_output(), 'XYZ', 'USART TX output accum');

// RX: inject byte, read SR (RXNE), read DR
assert_eq(uart_rx_byte(USART1, 0x51), true, 'USART rx_byte returns true'); // 'Q'
sr = periph_read(USART1 + 0x00, 4);
assert_eq(sr & (1 << 5), 1 << 5, 'USART SR RXNE after rx_byte');
let dr = periph_read(USART1 + 0x04, 4);
assert_eq(dr & 0xFF, 0x51, 'USART RX read byte Q');

// RXNE should clear after reading DR (buffer empty)
sr = periph_read(USART1 + 0x00, 4);
// RXNE is bit 5 — firmware also clears TC/TXE on read, but those are re-set
// RXNE stays cleared when buffer is empty
assert_eq(sr & (1 << 5), 0, 'USART SR RXNE cleared after DR read');

// Multiple RX bytes
uart_rx_byte(USART1, 0x31); // '1'
uart_rx_byte(USART1, 0x32); // '2'
uart_rx_byte(USART1, 0x33); // '3'
assert_eq(periph_read(USART1 + 0x04, 4) & 0xFF, 0x31, 'USART RX first byte 1');
assert_eq(periph_read(USART1 + 0x04, 4) & 0xFF, 0x32, 'USART RX second byte 2');
assert_eq(periph_read(USART1 + 0x04, 4) & 0xFF, 0x33, 'USART RX third byte 3');

// ============================================================
// ADC
// ============================================================
group('ADC');

reset();
const ADC1 = 0x40012400;

// Enable ADC1 clock
periph_write(0x40021018, 4, 1 << 9); // ADC1EN

// Set simulated value
adc_set_sim_value(0x3FF);  // 1023

// Enable ADC: ADON = bit 0 of CR2
periph_write(ADC1 + 0x08, 4, 1);

// Trigger SWSTART: CR2 bit 22
let cr2 = periph_read(ADC1 + 0x08, 4);
periph_write(ADC1 + 0x08, 4, cr2 | (1 << 22));

// Conversion takes (SMP + 12.5) ADC cycles; default SMP=0 -> 14 instructions.
// EOC must NOT be set before the conversion completes.
let sradc = periph_read(ADC1 + 0x00, 4);
assert_eq(sradc & (1 << 1), 0, 'ADC EOC not set before conversion completes');

// Check EOC in SR bit 1
step_batch(14);
sradc = periph_read(ADC1 + 0x00, 4);
assert_eq(sradc & (1 << 1), 1 << 1, 'ADC SR EOC after SWSTART + 14 cycles');

// Read DR (0x4C)
let dr_val = periph_read(ADC1 + 0x4C, 4) & 0xFFF;
assert_eq(dr_val, 0x3FF, 'ADC DR value matches sim value');

// EOC should clear on DR read
sradc = periph_read(ADC1 + 0x00, 4);
assert_eq(sradc & (1 << 1), 0, 'ADC EOC cleared after DR read');

// Second conversion with different value
adc_set_sim_value(0x155); // 341
periph_write(ADC1 + 0x08, 4, (1 << 0) | (1 << 22)); // ADON + SWSTART
sradc = periph_read(ADC1 + 0x00, 4);
assert_eq(sradc & (1 << 1), 0, 'ADC EOC not set before second conversion completes');
step_batch(14);
sradc = periph_read(ADC1 + 0x00, 4);
assert_eq(sradc & (1 << 1), 1 << 1, 'ADC SR EOC after second SWSTART');
dr_val = periph_read(ADC1 + 0x4C, 4) & 0xFFF;
assert_eq(dr_val, 0x155, 'ADC DR second value');

// RC sample-and-hold: wire 3.3V analog to PA0 (channel 0), sample with a
// large RC tau so the cap does NOT reach the target within one sample window.
// A newly reset cap (0 V) converts to a fraction of the full scale.
reset();
periph_write(0x40021018, 4, 1 << 9); // ADC1EN
periph_write(ADC1 + 0x08, 4, 1);     // ADON
adc_set_sim_value(0x000);            // sim source unused for wired pins
gpio_set_analog(0, 0, 0xFFF);        // PA0 = 3.3V (12-bit full scale)
adc_set_rc_tau(100);                 // very slow cap -> huge undershoot
periph_write(ADC1 + 0x08, 4, (1 << 0) | (1 << 22)); // ADON + SWSTART
step_batch(14);
const rc_full = periph_read(ADC1 + 0x4C, 4) & 0xFFF;
assert(rc_full < 0xFFF - 64, `ADC RC cap does not reach target in one sample (${rc_full})`);
assert(rc_full > 0, `ADC RC cap charges off zero (${rc_full})`);

// A second conversion directly after settles further toward the target
// (the cap holds the previous result and keeps charging).
periph_write(ADC1 + 0x08, 4, (1 << 0) | (1 << 22));
step_batch(14);
const rc_second = periph_read(ADC1 + 0x4C, 4) & 0xFFF;
assert(rc_second > rc_full, `ADC RC cap continues charging toward target (${rc_full} -> ${rc_second})`);

// With a tiny tau the cap tracks the wire within the sample window
// (still taking the RC path; result near the target but allowed to undershoot).
periph_write(ADC1 + 0x0C, 4, 7 << 0); // SMP0 = 239.5 cycles window
adc_set_rc_tau(1);
periph_write(ADC1 + 0x08, 4, (1 << 0) | (1 << 22));
step_batch(252);
const rc_fast = periph_read(ADC1 + 0x4C, 4) & 0xFFF;
assert(rc_fast > 0xF00, `RC cap settles near full scale with tau=1 (${rc_fast})`);

// Per-pin disconnection: clear the analog wire, RC path reverts to the
// exact simulated value.
gpio_set_analog(0, 0, 0xFFFF);
periph_write(ADC1 + 0x08, 4, (1 << 0) | (1 << 22));
step_batch(14);
const rc_sim = periph_read(ADC1 + 0x4C, 4) & 0xFFF;
assert_eq(rc_sim, 0x000, 'ADC without wired pin returns exact sim value');

// DAC -> ADC analog loopback: DAC1 drives PA4 (channel 4) with a 12-bit
// voltage; the ADC samples it through the RC cap (tau 100 left by the RC
// group above: first sample lands well below the target and charges up).
reset();
periph_write(0x4002101C, 4, 1 << 29);  // APB1ENR: DAC1EN
periph_write(0x40021018, 4, 1 << 9);   // APB2ENR: ADC1EN
periph_write(0x40007400, 4, 0x1);      // DAC CR: EN1
periph_write(0x40007408, 4, 0x800);    // DHR12R1 = 2048 (half scale)
adc_set_rc_tau(100);                   // slow cap: first sample undershoots
periph_write(ADC1 + 0x34, 4, 4);       // SQ1 = ch4 (PA4 = DAC1_OUT)
periph_write(ADC1 + 0x08, 4, (1 << 0) | (1 << 22)); // ADON + SWSTART
step_batch(14);
const dac_full = periph_read(ADC1 + 0x4C, 4) & 0xFFF;
// cap from 0 to 0x800 over 14 cycles, tau 100: 2048 * (1 - e^-0.14) = 267
assert(dac_full > 0x80 && dac_full < 0x600, `DAC1->ADC ch4 sampled the RC'd wire (${dac_full})`);
periph_write(ADC1 + 0x08, 4, (1 << 0) | (1 << 22));
step_batch(14);
const dac_second = periph_read(ADC1 + 0x4C, 4) & 0xFFF;
assert(dac_second > dac_full, `DAC loopback cap continues charging (${dac_full} -> ${dac_second})`);
// DAC2 on PA5 (channel 5)
periph_write(ADC1 + 0x34, 4, 5);        // SQ1 = ch5
periph_write(ADC1 + 0x08, 4, (1 << 0) | (1 << 22));   // DAC2 still disabled: sim (0x000) source
step_batch(14);
periph_write(0x40007414, 4, 0x300);     // DHR12R2 = 768
periph_write(0x40007400, 4, 0x11);      // EN1 + EN2
periph_write(ADC1 + 0x08, 4, (1 << 0) | (1 << 22));
step_batch(14);
const dac2_val = periph_read(ADC1 + 0x4C, 4) & 0xFFF;
assert(dac2_val > 0x10 && dac2_val < 0x300, `DAC2->ADC ch5 charges toward 0x300 (${dac2_val})`);

// AWD interrupt: HTR/LTR straddle the result -> AWD flag and IRQ 18 pending
reset();
periph_write(0x40021018, 4, 1 << 9);   // ADC1EN
periph_write(ADC1 + 0x08, 4, 1);       // ADON
adc_set_sim_value(0x3FF);
periph_write(ADC1 + 0x04, 4, 0x41);    // CR1: AWDEN(0) + AWDIE(6)
periph_write(0xE000E100, 4, 1 << 18);  // NVIC ISER: enable ADC IRQ 18
periph_write(ADC1 + 0x24, 4, 0x200);   // HTR = 512
periph_write(ADC1 + 0x28, 4, 0x100);   // LTR = 256
periph_write(ADC1 + 0x08, 4, (1 << 0) | (1 << 22));
step_batch(14);
assert_eq(periph_read(ADC1 + 0x00, 4) & 1, 1, 'ADC AWD flag set for out-of-range result');
assert(has_pending_interrupt() && get_next_pending_interrupt() === 18,
  'ADC AWD interrupt pending (IRQ 18)');
clear_current_interrupt();

// External trigger: TIM1 update -> TRGO -> ADC (EXTTRIG + EXTSEL=TIM1_TRGO)
reset();
periph_write(0x40021018, 4, (1 << 9) | (1 << 11)); // ADC1EN + TIM1EN
adc_set_sim_value(0x155);
periph_write(ADC1 + 0x08, 4, (1 << 0) | (1 << 20) | (7 << 17)); // ADON|EXTTRIG|EXTSEL=7
periph_write(0x40012C00 + 0x00, 4, 1);              // TIM1 CR1: CEN
periph_write(0x40012C00 + 0x04, 4, 0x20);           // TIM1 CR2: MMS=010 (update->TRGO)
periph_write(0x40012C00 + 0x2C, 4, 0x100);          // TIM1 ARR
let trig_eoc = false;
for (let i = 0; i < 6 && !trig_eoc; i++) {
  step_batch(1000);
  trig_eoc = (periph_read(ADC1 + 0x00, 4) & 2) !== 0;
}
assert(trig_eoc, 'ADC starts from TIM1 TRGO without SWSTART');
assert_eq(periph_read(ADC1 + 0x4C, 4) & 0xFFF, 0x155, 'ADC DR correct after TIM1 TRGO trigger');

// External trigger: TIM1_CC1 compare event (EXTSEL=0)
reset();
periph_write(0x40021018, 4, (1 << 9) | (1 << 11));
periph_write(ADC1 + 0x08, 4, (1 << 0) | (1 << 20)); // EXTSEL defaults to TIM1_CC1
periph_write(0x40012C00 + 0x00, 4, 0x1);            // CEN
periph_write(0x40012C00 + 0x20, 4, 0x1);            // CCER: CC1E
periph_write(0x40012C00 + 0x34, 4, 0x40);           // CCR1 = 64
periph_write(0x40012C00 + 0x2C, 4, 0x100);          // ARR
trig_eoc = false;
for (let i = 0; i < 6 && !trig_eoc; i++) {
  step_batch(1000);
  trig_eoc = (periph_read(ADC1 + 0x00, 4) & 2) !== 0;
}
assert(trig_eoc, 'ADC starts from TIM1_CC1 compare event');

// External trigger: EXTI line 11 rising edge (EXTSEL=6)
reset();
periph_write(0x40021018, 4, (1 << 9) | (1 << 0));   // ADC1EN + AFIOEN
periph_write(ADC1 + 0x08, 4, (1 << 0) | (1 << 20) | (6 << 17)); // EXTI11
periph_write(0x40010400 + 0x00, 4, 1 << 11);        // EXTI IMR bit11
periph_write(0x40010400 + 0x08, 4, 1 << 11);        // EXTI RTSR bit11
periph_write(0x40010804, 4, 0x3 << 12);             // PA11 output push-pull
periph_write(0x40010810, 4, 1 << 11);               // BSRR: PA11 high (rising edge)
trig_eoc = false;
for (let i = 0; i < 6 && !trig_eoc; i++) {
  step_batch(200);
  trig_eoc = (periph_read(ADC1 + 0x00, 4) & 2) !== 0;
}
assert(trig_eoc, 'ADC starts from EXTI11 rising edge');

// ============================================================
// RCC
// ============================================================
group('RCC');

reset();
const RCC = 0x40021000;

// Read RCC_CR — should default to HSI on (bit 1 = HSIRDY)
let cr = periph_read(RCC + 0x00, 4);
assert_eq(cr & 1, 1, 'RCC CR HSION');
assert_eq(cr & 2, 2, 'RCC CR HSIRDY');

// Enable HSE: CR bit 16 = HSEON, bit 17 = HSERDY (set by simulation)
periph_write(RCC + 0x00, 4, cr | (1 << 16));
cr = periph_read(RCC + 0x00, 4);
assert_eq(cr & (1 << 16), 1 << 16, 'RCC CR HSEON after set');
assert_eq(cr & (1 << 17), 1 << 17, 'RCC CR HSERDY set by sim');

// Disable HSE: should clear HSEON AND HSERDY
periph_write(RCC + 0x00, 4, cr & ~(1 << 16));
cr = periph_read(RCC + 0x00, 4);
assert_eq(cr & (1 << 16), 0, 'RCC CR HSEON cleared');
assert_eq(cr & (1 << 17), 0, 'RCC CR HSERDY cleared with HSEON');

// Enable PLL: CR bit 24 = PLLON, bit 25 = PLLRDY
periph_write(RCC + 0x00, 4, (1 << 24));
cr = periph_read(RCC + 0x00, 4);
assert_eq(cr & (1 << 24), 1 << 24, 'RCC CR PLLON');
assert_eq(cr & (1 << 25), 1 << 25, 'RCC CR PLLRDY');

// Disable PLL → PLLRDY also clears
periph_write(RCC + 0x00, 4, 0);
cr = periph_read(RCC + 0x00, 4);
assert_eq(cr & (1 << 24), 0, 'RCC CR PLLON off');
assert_eq(cr & (1 << 25), 0, 'RCC CR PLLRDY cleared');

// CFGR: configure SW=HSI (00), SWS tracks SW
periph_write(RCC + 0x04, 4, 0x00000000);
let cfgr = periph_read(RCC + 0x04, 4);
assert_eq(cfgr & 0x3, 0, 'RCC CFGR SW = HSI');
assert_eq((cfgr >> 2) & 0x3, 0, 'RCC CFGR SWS = HSI');

// APB2ENR: enable bits
periph_write(RCC + 0x18, 4, (1 << 14) | (1 << 2) | (1 << 4));
let apb2 = periph_read(RCC + 0x18, 4);
assert_eq(apb2 & (1 << 14), 1 << 14, 'RCC APB2ENR USART1EN');
assert_eq(apb2 & (1 << 2), 1 << 2, 'RCC APB2ENR GPIOAEN');
assert_eq(apb2 & (1 << 4), 1 << 4, 'RCC APB2ENR GPIOCEN');

// APB1ENR: enable TIM2, I2C1
periph_write(RCC + 0x1C, 4, (1 << 0) | (1 << 21));
let apb1 = periph_read(RCC + 0x1C, 4);
assert_eq(apb1 & (1 << 0), 1 << 0, 'RCC APB1ENR TIM2EN');
assert_eq(apb1 & (1 << 21), 1 << 21, 'RCC APB1ENR I2C1EN');

// ============================================================
// SysTick
// ============================================================
group('SysTick');

reset();
const STK = 0xE000E010;

// Set reload value
periph_write(STK + 0x04, 4, 999); // RVR = 1000-1
assert_eq(periph_read(STK + 0x04, 4), 999, 'SysTick RVR');

// Clear current value
periph_write(STK + 0x08, 4, 0);
assert_eq(periph_read(STK + 0x08, 4), 0, 'SysTick CVR after clear');

// Enable SysTick: CSR bits: 0=ENABLE, 1=TICKINT, 2=CLKSOURCE
periph_write(STK + 0x00, 4, 0x07);
let csr = periph_read(STK + 0x00, 4);
assert_eq(csr & 0x07, 0x07, 'SysTick CSR ENABLE|TICKINT|CLKSOURCE');

// Tick enough times to trigger SysTick interrupt
assert_eq(has_pending_interrupt(), false, 'SysTick no pending before count');

for (let i = 0; i < 2000; i++) tick();
assert_eq(has_pending_interrupt(), true, 'SysTick pending after >1000 ticks');

let irq = get_next_pending_interrupt();
assert_eq(irq, -1, 'SysTick IRQ number = -1');
assert_eq(has_pending_interrupt(), false, 'SysTick pending cleared after get');
clear_current_interrupt();

// Second fire: tick another 1000+
for (let i = 0; i < 1000; i++) tick();
assert_eq(has_pending_interrupt(), true, 'SysTick pending second fire');
irq = get_next_pending_interrupt();
assert_eq(irq, -1, 'SysTick IRQ number = -1 (second)');

// Disable SysTick — should stop firing
periph_write(STK + 0x00, 4, 0); // clear ENABLE
assert_eq(has_pending_interrupt(), false, 'SysTick no pending after disable');
for (let i = 0; i < 1500; i++) tick();
assert_eq(has_pending_interrupt(), false, 'SysTick still no pending (disabled)');

// ============================================================
// TIM (Timer/PWM)
// ============================================================
group('TIM');

reset();
const TIM2 = 0x40000000;

// Enable TIM2 clock
periph_write(0x4002101C, 4, 1 << 0);

// Set PSC = 7999, ARR = 999
periph_write(TIM2 + 0x28, 4, 7999);  // PSC
assert_eq(periph_read(TIM2 + 0x28, 4), 7999, 'TIM2 PSC');

periph_write(TIM2 + 0x2C, 4, 999);   // ARR
assert_eq(periph_read(TIM2 + 0x2C, 4), 999, 'TIM2 ARR');

// Set CCR1 = 500 (50% duty)
periph_write(TIM2 + 0x34, 4, 500);
assert_eq(periph_read(TIM2 + 0x34, 4), 500, 'TIM2 CCR1');

// Configure CCMR1: OC1M=110 (PWM1), OC1PE=1
periph_write(TIM2 + 0x18, 4, (0b110 << 4) | (1 << 3));
let ccmr1 = periph_read(TIM2 + 0x18, 4);
assert_eq((ccmr1 >> 4) & 0x7, 0b110, 'TIM2 CCMR1 OC1M = PWM1');
assert_eq((ccmr1 >> 3) & 1, 1, 'TIM2 CCMR1 OC1PE');

// CCER: CC1E = bit 0
periph_write(TIM2 + 0x20, 4, 1);
assert_eq(periph_read(TIM2 + 0x20, 4) & 1, 1, 'TIM2 CCER CC1E');

// CR1: CEN = bit 0
periph_write(TIM2 + 0x00, 4, 1);
assert_eq(periph_read(TIM2 + 0x00, 4) & 1, 1, 'TIM2 CR1 CEN');

// Read SR — UIF (bit 0) initially 0
let tim_sr = periph_read(TIM2 + 0x10, 4);
assert_eq(tim_sr & 1, 0, 'TIM2 SR UIF initial');

// ============================================================
// IWDG (Independent Watchdog)
// ============================================================
group('IWDG');

reset();
const IWDG = 0x40003000;

// Write key 0x5555 to KR to enable register access
periph_write(IWDG + 0x00, 4, 0x5555);

// Set prescaler = 4 (div by 64)
periph_write(IWDG + 0x04, 4, 4);
assert_eq(periph_read(IWDG + 0x04, 4), 4, 'IWDG PR prescaler');

// Set reload = 0xFFF
periph_write(IWDG + 0x08, 4, 0xFFF);
assert_eq(periph_read(IWDG + 0x08, 4), 0xFFF, 'IWDG RLR reload');

// Start watchdog: write 0xCCCC to KR
periph_write(IWDG + 0x00, 4, 0xCCCC);
// Read KR — should return 0 (reads as reserved)
assert_eq(periph_read(IWDG + 0x00, 4), 0, 'IWDG KR read = 0');

// Refresh: write 0xAAAA to KR
periph_write(IWDG + 0x00, 4, 0xAAAA);
assert_eq(is_watchdog_reset_requested(), false, 'IWDG no reset after refresh');

// ============================================================
// NVIC
// ============================================================
group('NVIC');

reset();
const NVIC = 0xE000E100;

// Enable USART1 IRQ (IRQ 37): ISER[1] bit 5 (= 37-32)
periph_write(NVIC + 0x00, 4, 0); // ISER0 = 0
periph_write(NVIC + 0x04, 4, 1 << 5); // ISER1 bit 5 = IRQ 37
let iser1 = periph_read(NVIC + 0x04, 4);
assert_eq(iser1 & (1 << 5), 1 << 5, 'NVIC ISER1 USART1 enabled');

// Set pending via ISPR
periph_write(NVIC + 0x100 + 0x04, 4, 1 << 5); // ISPR1 bit 5
assert_eq(has_pending_interrupt(), true, 'NVIC has pending after ISPR write');

// Get the interrupt
let pirq = get_next_pending_interrupt();
assert_eq(pirq, 37, 'NVIC pending IRQ = 37 (USART1)');
assert_eq(has_pending_interrupt(), false, 'NVIC pending cleared after get');

// Clear enable via ICER
periph_write(NVIC + 0x80 + 0x04, 4, 1 << 5); // ICER1 bit 5
iser1 = periph_read(NVIC + 0x04, 4);
assert_eq(iser1 & (1 << 5), 0, 'NVIC USART1 disabled after ICER');

// Set priority for USART1 (absolute 0xE000E325 = NVIC_REGS_BASE + 0x225)
periph_write(0xE000E325, 4, 0x80);
let prio = periph_read(0xE000E325, 4);
assert_eq(prio, 0x80, 'NVIC USART1 priority 0x80');

// ============================================================
// CRC
// ============================================================
group('CRC');

reset();
const CRC = 0x40023000;

// Write data to CRC DR — CRC computes actual CRC-32
periph_write(CRC + 0x00, 4, 0xDEADBEEF);
let crc_val = periph_read(CRC + 0x00, 4);
// CRC-32 of 0xDEADBEEF with init 0xFFFFFFFF should be non-zero and not the input
assert_neq(crc_val, 0xDEADBEEF, 'CRC DR changed from input');
assert_neq(crc_val, 0, 'CRC DR non-zero');
assert_neq(crc_val, 0xFFFFFFFF, 'CRC DR not init value');

// Reset CRC (write CR bit 0) then re-compute
periph_write(CRC + 0x08, 4, 1); // reset
assert_eq(periph_read(CRC + 0x00, 4), 0xFFFFFFFF, 'CRC DR reset to 0xFFFFFFFF');
periph_write(CRC + 0x00, 4, 0x00000000);
let crc_zero = periph_read(CRC + 0x00, 4);
// CRC-32 of 0 with init 0xFFFFFFFF is C704DD7B
assert_eq(crc_zero, 0xC704DD7B, 'CRC-32 of 0x00000000');

// ============================================================
// SPI
// ============================================================
group('SPI');

reset();
const SPI1 = 0x40013000;

// Enable SPI1 clock
periph_write(0x40021018, 4, 1 << 12); // SPI1EN

// Configure CR1: BR=3 (div 16), MSTR, SPE
periph_write(SPI1 + 0x00, 4, (3 << 3) | (1 << 2) | (1 << 6));
let spi_cr1 = periph_read(SPI1 + 0x00, 4);
assert_eq(spi_cr1 & (1 << 6), 1 << 6, 'SPI1 CR1 SPE');
assert_eq((spi_cr1 >> 3) & 0x7, 3, 'SPI1 CR1 BR=3');

// Write to SPI DR (offset 0x0C) — triggers xfer, no device = rx 0xFF
periph_write(SPI1 + 0x0C, 4, 0xA5);

// Read SR first (before reading DR clears RXNE)
let spi_sr = periph_read(SPI1 + 0x08, 4);
assert_eq(spi_sr & (1 << 1), 1 << 1, 'SPI1 SR TXE');
assert_eq(spi_sr & (1 << 0), 1 << 0, 'SPI1 SR RXNE');

// Now read DR
assert_eq(periph_read(SPI1 + 0x0C, 4), 0xFF, 'SPI1 DR xfer returns 0xFF (no device)');

// CRCPR at offset 0x10 stores value directly
periph_write(SPI1 + 0x10, 4, 0x07);
assert_eq(periph_read(SPI1 + 0x10, 4), 0x07, 'SPI1 CRCPR');

// ============================================================
// I2C
// ============================================================
group('I2C');

reset();
const I2C1 = 0x40005400;

// Enable I2C1 clock (APB1)
periph_write(0x4002101C, 4, 1 << 21);

// Configure CR1: PE (bit 0)
periph_write(I2C1 + 0x00, 4, 1);
assert_eq(periph_read(I2C1 + 0x00, 4) & 1, 1, 'I2C1 CR1 PE');

// Set own address (OAR1)
periph_write(I2C1 + 0x08, 4, 0x42 << 1); // addr 0x42
let oar1 = periph_read(I2C1 + 0x08, 4);
assert_eq((oar1 >> 1) & 0x7F, 0x42, 'I2C1 OAR1 addr 0x42');

// Set CCR = 0x50 (100kHz @ 8MHz)
periph_write(I2C1 + 0x1C, 4, 0x50);
assert_eq(periph_read(I2C1 + 0x1C, 4), 0x50, 'I2C1 CCR');

// Check SR2: busy flag (bit 1) should be 0 when idle
let i2c_sr2 = periph_read(I2C1 + 0x18, 4);
assert_eq(i2c_sr2 & (1 << 1), 0, 'I2C1 SR2 BUSY = 0 (idle)');

// ============================================================
// RTC
// ============================================================
group('RTC');

reset();
const RTC = 0x40002800;

// Write to RTC PRL (prescaler load)
periph_write(RTC + 0x0C, 4, 0x7FFF);
assert_eq(periph_read(RTC + 0x0C, 4), 0x7FFF, 'RTC PRL');

// Write to RTC CNT (counter) — low half
periph_write(RTC + 0x1C, 4, 0x1234);
assert_eq(periph_read(RTC + 0x1C, 4), 0x1234, 'RTC CNT');

// ============================================================
// PWR
// ============================================================
group('PWR');

reset();
const PWR = 0x40007000;

// Write CR: clear PDDS (bit 1), set LPDS (bit 0)
periph_write(PWR + 0x00, 4, 1);
assert_eq(periph_read(PWR + 0x00, 4) & 1, 1, 'PWR CR LPDS');
assert_eq(periph_read(PWR + 0x00, 4) & 2, 0, 'PWR CR PDDS = 0');

// ============================================================
// Flash
// ============================================================
group('FLASH');

reset();
const FLASH = 0x40022000;

// Write ACR: LATENCY=1 only, then verify
periph_write(FLASH + 0x00, 4, 1); // just LATENCY=1
let acr = periph_read(FLASH + 0x00, 4);
assert_eq(acr & 7, 1, 'FLASH ACR LATENCY=1');
// Enable PRFTEN + ICEN + DCEN
periph_write(FLASH + 0x00, 4, (1 << 0) | (1 << 2) | (1 << 3) | (1 << 4));
acr = periph_read(FLASH + 0x00, 4);
assert_eq(acr & (1 << 2), 1 << 2, 'FLASH ACR PRFTEN');
assert_eq(acr & (1 << 3), 1 << 3, 'FLASH ACR ICEN');
assert_eq(acr & (1 << 4), 1 << 4, 'FLASH ACR DCEN');

// ============================================================
// CAN
// ============================================================
group('CAN');

reset();
const CAN1 = 0x40006400;

// Write CAN MCR: INRQ (bit 0) = 1
periph_write(CAN1 + 0x00, 4, 1);
assert_eq(periph_read(CAN1 + 0x00, 4) & 1, 1, 'CAN1 MCR INRQ');

// Write CAN BTR
periph_write(CAN1 + 0x1C, 4, 0x001C0033);
assert_eq(periph_read(CAN1 + 0x1C, 4), 0x001C0033, 'CAN1 BTR');

// ============================================================
// DMA
// ============================================================
group('DMA');

reset();
const DMA1 = 0x40020000;

// Enable DMA1 clock
periph_write(0x40021014, 4, 1 << 0); // DMA1EN on APB1

// Configure channel 1: CCR
// M2M=1, PL=11 (very high), MSIZE=01 (16-bit), PSIZE=01, MINC=1, PINC=0, CIRC=0, DIR=1 (read from mem), EN=0 first
periph_write(DMA1 + 0x08 + 0*0x14, 4, (1 << 14) | (3 << 12) | (1 << 10) | (1 << 8) | (1 << 7) | (1 << 4));
let ccr = periph_read(DMA1 + 0x08 + 0*0x14, 4);
assert_eq(ccr & (1 << 14), 1 << 14, 'DMA1 CH1 CCR M2M');

// Set source (CNDTR), source addr, dest addr
periph_write(DMA1 + 0x08 + 2*4, 4, 0x20000000); // CPAR = source
periph_write(DMA1 + 0x08 + 3*4, 4, 0x20001000); // CMAR = dest
periph_write(DMA1 + 0x08 + 1*4, 4, 16); // CNDTR = 16 bytes
assert_eq(periph_read(DMA1 + 0x08 + 1*4, 4), 16, 'DMA1 CH1 CNDTR');

// ============================================================
// DMA pending transfer check
// ============================================================
group('DMA Transfer');

reset();
// Set up DMA channel 1
periph_write(0x40021014, 4, 1 << 0);
periph_write(DMA1 + 0x08 + 0*0x14, 4, 0); // disable first
periph_write(DMA1 + 0x08 + 2*4, 4, 0x20000000); // CPAR
periph_write(DMA1 + 0x08 + 3*4, 4, 0x20001000); // CMAR
periph_write(DMA1 + 0x08 + 1*4, 4, 8); // CNDTR = 8
// Enable with EN bit and DIR=memory-to-memory, MINC, PINC
periph_write(DMA1 + 0x08 + 0*0x14, 4, (1 << 14) | (1 << 7) | (1 << 6) | 1);
let dma_pending = periph.dma_get_pending_count();
assert_eq(dma_pending >= 1, true, 'DMA has pending transfers');
periph.dma_set_completed_many(1 << 0); // JS bridge moves the data, then signals completion
for (let i = 0; i < 3; i++) periph.tick();
assert_eq(periph_read(DMA1 + 0x08 + 1*4, 4), 0, 'DMA1 CNDTR 0 after transfer completes');
let dma_en = periph_read(DMA1 + 0x08 + 0*0x14, 4) & 1;
assert_eq(dma_en, 0, 'DMA1 EN cleared after transfer completes');

// ============================================================
// DMA pump exports (Rust-side periph byte movement, replaces
// the JS per-chunk periph_read/periph_write loops in processDma)
// ============================================================
group('DMA pump exports');

reset();
const USART1_PUMP = 0x40013800;
periph_write(0x40021014, 4, 1 << 14); // USART1EN on APB2
periph_write(0x40021014, 4, 1 << 0);  // DMA1EN on APB1
// USART1 TX on PA9, RX on PA10
periph_write(0x40010804, 4, (0x4 << 14) | 0x4); // CRH PA9-10 AF push-pull (10MHz)
periph_write(0x4001380C, 4, 0x200C);            // CR1 UE|TE|RE
// absorb: pops RX FIFO bytes in order via the periph_read path
periph.uart_rx_byte(USART1_PUMP, 0x41); // 'A'
periph.uart_rx_byte(USART1_PUMP, 0x42); // 'B'
periph.uart_rx_byte(USART1_PUMP, 0x43); // 'C'
// NOTE: wasm-bindgen returns Vec<u8> as a plain JS number array; Uint8Array
// wrap keeps join() from stringifying element 65 as "65"
let popped = Array.from(new Uint8Array(periph.dma_absorb_periph(USART1_PUMP + 0x04, 3)), b => String.fromCharCode(b)).join('');
assert_eq(popped, 'A\x00\x00', 'dma_absorb_periph pops RX FIFO byte first, pads chunk tail');
assert_eq(periph.uart_rx_pending(USART1_PUMP), 2, 'dma_absorb_periph pops ONE FIFO byte per read');
// absorb with odd size: chunk=4 read returns the FIFO byte + zero pad (JS loop semantics)
periph.uart_rx_byte(USART1_PUMP, 0x51); // 'Q'
popped = Array.from(new Uint8Array(periph.dma_absorb_periph(USART1_PUMP + 0x04, 5)), b => String.fromCharCode(b)).join('');
assert_eq(popped, 'B\x00\x00\x00C', 'absorb 5-byte read pops a FIFO byte per chunk, zero pads');
assert_eq(periph.uart_rx_pending(USART1_PUMP), 1, 'absorb consumed 2 of 3 queued bytes');
// push: USART DR consumes ONE byte per write (FIFO), so a 3-byte buffer lands byte 0
periph.dma_push_periph(USART1_PUMP + 0x04, new Uint8Array([0x41, 0x42, 0x43]));
let out = get_uart_output();
assert_eq(out, 'A', 'dma_push_periph feeds the chunk-leading byte to the TX FIFO');
// uneven chunk (4+3): leading bytes of each chunk are consumed, rest return to FIFO
periph.dma_push_periph(USART1_PUMP + 0x04, new Uint8Array([1, 2, 3, 4, 5, 6, 7]));
out = get_uart_output();
assert_eq(out, '\x01\x05', 'dma_push_periph handles uneven chunks (per-chunk lead byte)');
// per-byte pushes (real DMA TX pattern: size 1 per transfer) land every byte
periph.dma_push_periph(USART1_PUMP + 0x04, new Uint8Array([0x50]));
periph.dma_push_periph(USART1_PUMP + 0x04, new Uint8Array([0x51]));
out = get_uart_output();
assert_eq(out, 'PQ', 'dma_push_periph per-byte pushes land in order');

reset();
const SCB = 0xE000ED00;

// Read CPUID
let cpuid = periph_read(SCB + 0x00, 4);
// ARM implementer=0x41, part=0xC24 → 0x41_0_C24_1 = 0x410FC241
assert_eq(cpuid >> 24, 0x41, 'SCB CPUID implementer = ARM');

// Write VTOR
periph_write(SCB + 0x08, 4, 0x08000000);
assert_eq(periph_read(SCB + 0x08, 4), 0x08000000, 'SCB VTOR');

// Pend SysTick via ICSR
periph_write(SCB + 0x04, 4, 1 << 26); // ICSR PENDSTSET
assert_eq(has_pending_interrupt(), true, 'SCB PendSV via ICSR pending');

// ============================================================
// TIM Real Counting Test
// ============================================================
group('TIM Counting');

reset();
const T2 = 0x40000000;

// Enable TIM2 clock
periph_write(0x40021014, 4, 1 << 0);

// Set PSC=0 (no prescaler), ARR=999 → counts 0..999 then wraps
periph_write(T2 + 0x28, 4, 0);    // PSC = 0
periph_write(T2 + 0x2C, 4, 999);  // ARR = 999

// Enable timer (CR1.CEN = bit 0)
periph_write(T2 + 0x00, 4, 1);

// Run 500 ticks — CNT should be ~500
for (let i = 0; i < 500; i++) tick();
let tcnt = periph_read(T2 + 0x24, 4);
assert_eq(tcnt, 500, 'TIM2 CNT = 500 after 500 ticks with PSC=0');

// Run another 600 ticks → should wrap (500+600=1100, wraps to 1000 mod 1000 = 100)
for (let i = 0; i < 600; i++) tick();
tcnt = periph_read(T2 + 0x24, 4);
assert_eq(tcnt, 100, 'TIM2 CNT wrapped to 100 after 1100 total ticks');

// Check SR.UIF (bit 0) set on overflow
let tsr = periph_read(T2 + 0x10, 4);
assert_eq(tsr & 1, 1, 'TIM2 SR UIF set after overflow');

// Clear UIF by writing SR (write 0 to clear)
periph_write(T2 + 0x10, 4, 0);
tsr = periph_read(T2 + 0x10, 4);
assert_eq(tsr & 1, 0, 'TIM2 SR UIF cleared');

// ============================================================
// TIM Update Interrupt Test
// ============================================================
group('TIM Interrupt');

reset();

periph_write(0x40021014, 4, 1 << 0);
periph_write(T2 + 0x28, 4, 0);    // PSC = 0
periph_write(T2 + 0x2C, 4, 99);   // ARR = 99 (wrap every 100 ticks)
periph_write(T2 + 0x0C, 4, 1);   // DIER.UIE = 1 (enable update interrupt)
periph_write(T2 + 0x00, 4, 1);   // CEN = 1

// Enable TIM2 IRQ (28) in NVIC ISER0
periph_write(0xE000E100, 4, 1 << 28);

// No interrupt should be pending yet
assert_eq(has_pending_interrupt(), false, 'TIM2 no IRQ pending before overflow');

// Run 200 ticks → should overflow twice
for (let i = 0; i < 200; i++) tick();

// Update interrupt should fire (TIM2 IRQ = 28)
assert_eq(has_pending_interrupt(), true, 'TIM2 IRQ pending after overflow');
let tim_irq = get_next_pending_interrupt();
assert_eq(tim_irq, 28, 'TIM2 IRQ number = 28');
assert_eq(has_pending_interrupt(), false, 'TIM2 IRQ cleared after get');

// ============================================================
// TIM Prescaler Test
// ============================================================
group('TIM Prescaler');

reset();

periph_write(0x40021014, 4, 1 << 0);
periph_write(T2 + 0x28, 4, 9);    // PSC = 9 (tick every 10 instructions)
periph_write(T2 + 0x2C, 4, 99);   // ARR = 99
periph_write(T2 + 0x00, 4, 1);    // CEN = 1

// Run 500 ticks → prescaler divides by 10, so CNT should be 50
for (let i = 0; i < 500; i++) tick();
tcnt = periph_read(T2 + 0x24, 4);
assert_eq(tcnt, 50, 'TIM2 CNT = 50 after 500 ticks with PSC=9');

// ============================================================
// RTC Real Counting Test
// ============================================================
group('RTC Counting');

reset();

// Enable RTC (CRL bit 5 = RTOFF)
periph_write(0x40002800 + 0x04, 4, 1 << 5);

// Set prescaler = 99 (CNT increments every 100 ticks)
periph_write(0x40002800 + 0x0C, 4, 99);

// Write initial CNT = 0
periph_write(0x40002800 + 0x1C, 4, 0); // cntl
periph_write(0x40002800 + 0x18, 4, 0); // cnth

assert_eq(periph_read(0x40002800 + 0x1C, 4), 0, 'RTC CNTL = 0 initial');

// Run 1000 ticks → should increment CNT by ~10
for (let i = 0; i < 1000; i++) tick();

let cntl = periph_read(0x40002800 + 0x1C, 4);
assert_eq(cntl, 10, 'RTC CNT = 10 after 1000 ticks with PRL=99');

// Run 500 more ticks → CNT should be ~15
for (let i = 0; i < 500; i++) tick();
cntl = periph_read(0x40002800 + 0x1C, 4);
assert_eq(cntl, 15, 'RTC CNT = 15 after 1500 total ticks');

// ============================================================
// FLASH Unlock Test
// ============================================================
group('FLASH Unlock');

reset();

// CR should be locked initially (LOCK bit 7 = 1)
let flash_cr = periph_read(0x40022000 + 0x0C, 4);
assert_eq(flash_cr & (1 << 7), 1 << 7, 'FLASH CR LOCK = 1 locked');

// Unlock sequence: write KEY1 then KEY2 to KEYR
periph_write(0x40022000 + 0x04, 4, 0x45670123); // KEY1
periph_write(0x40022000 + 0x04, 4, 0xCDEF89AB); // KEY2

// CR should now be unlocked
flash_cr = periph_read(0x40022000 + 0x0C, 4);
assert_eq(flash_cr & (1 << 7), 0, 'FLASH CR LOCK = 0 after unlock');

// Set PG (bit 0) for programming
periph_write(0x40022000 + 0x0C, 4, 1); // PG=1
flash_cr = periph_read(0x40022000 + 0x0C, 4);
assert_eq(flash_cr & 1, 1, 'FLASH CR PG = 1 after unlock');

// Write flash address to AR
periph_write(0x40022000 + 0x10, 4, 0x08010000);
assert_eq(periph_read(0x40022000 + 0x10, 4), 0x08010000, 'FLASH AR = 0x08010000');

// Check SR — BSY should be set (bit 0) when PG is active
let flash_sr = periph_read(0x40022000 + 0x08, 4);
assert_eq(flash_sr & 1, 1, 'FLASH SR BSY = 1 while PG active');

// Clear PG bit
periph_write(0x40022000 + 0x0C, 4, 0);
flash_sr = periph_read(0x40022000 + 0x08, 4);
assert_eq(flash_sr & 1, 0, 'FLASH SR BSY = 0 after PG cleared');

// ============================================================
// CAN TX Mailbox Test
// ============================================================
group('CAN TX');

reset();

// Initial TSR — bits 26:24 (CODE) should indicate all mailboxes empty (0b100 = 4)
let can_tsr = periph_read(0x40006400 + 0x08, 4);
// TSR[31:26] = CODE field: 0x1C = 0b111 = mailbox 2 is the next empty one
assert_eq((can_tsr >> 26) & 7, 7, 'CAN TSR CODE = 7 all mailboxes empty');

// Write TX mailbox 0: TIR with TXRQ=1 (bit 0)
periph_write(0x40006400 + 0x180, 4, 0xABCD0001); // STDID=0xABCD, TXRQ=1
periph_write(0x40006400 + 0x184, 4, 8);          // DLC=8
periph_write(0x40006400 + 0x188, 4, 0xDEADBEEF); // data low
periph_write(0x40006400 + 0x18C, 4, 0x12345678); // data high

// TSR should show mailbox 0 as active (TME0 = bit 26 = 0, RQCP0 = bit 0 = 1, TXOK0 = bit 16 = 1)
can_tsr = periph_read(0x40006400 + 0x08, 4);
assert_eq(can_tsr & (1 << 0), 1 << 0, 'CAN TSR RQCP0 = 1 (request completed)');
assert_eq(can_tsr & (1 << 16), 1 << 16, 'CAN TSR TXOK0 = 1 (transmit OK)');

// Read back mailbox for verification
let can_tir = periph_read(0x40006400 + 0x180, 4);
assert_eq(can_tir, 0xABCD0001, 'CAN TX mailbox TIR preserved');
let can_tdtr = periph_read(0x40006400 + 0x184, 4);
assert_eq(can_tdtr, 8, 'CAN TX mailbox TDTR DLC=8');

// ============================================================
// CAN RX Mailbox Test
// ============================================================
group('CAN RX');

reset();

// Simulate a received message: populate RX mailbox 0
periph_write(0x40006400 + 0x1B0, 4, 0x1230001); // TIR with valid ID
periph_write(0x40006400 + 0x1B4, 4, 4);          // DLC=4
periph_write(0x40006400 + 0x1B8, 4, 0xAABBCCDD); // data
// Set FMP = 1 to indicate 1 message pending in FIFO 0
periph_write(0x40006400 + 0x0C, 4, 1);

// RF0R should show 1 message pending (FMP bits 1:0)
let can_rf0r = periph_read(0x40006400 + 0x0C, 4);
assert_eq(can_rf0r & 0x3, 1, 'CAN RF0R FMP = 1 message pending');

// Read RX mailbox 0 — should decrement FMP
let can_rx_tir = periph_read(0x40006400 + 0x1B0, 4);
assert_eq(can_rx_tir, 0x1230001, 'CAN RX mailbox TIR');
can_rf0r = periph_read(0x40006400 + 0x0C, 4);
assert_eq(can_rf0r & 0x3, 0, 'CAN RF0R FMP = 0 after reading RX mailbox');

// ============================================================
// CAN Filter Bank & RX Injection Test
// ============================================================
group('CAN Filter');

reset();
// Configure filter bank 0 as 32-bit identifier list mode
periph_write(0x40006400 + 0x200, 4, 0); // FMR: FINIT=0 (leave init mode)
// Enter init mode
periph_write(0x40006400 + 0x200, 4, 1); // FMR: FINIT=1
periph_write(0x40006400 + 0x204, 4, 0); // FM1R: all 16-bit dual, bank 0 = 0 (16-bit x2)
periph_write(0x40006400 + 0x20C, 4, 0xFFFFFFFF); // FS1R: all ID list mode
periph_write(0x40006400 + 0x214, 4, 0); // FFA1R: all FIFO 0
periph_write(0x40006400 + 0x21C, 4, 1); // FA1R: enable filter bank 0
// Set filter bank 0: in 16-bit list mode, store two 16-bit IDs per filter word
// Filter 0 word 0: 0x0555XXXX where 0x0555 = ID1(match 0x555), 0xXXXX = ID2
periph_write(0x40006400 + 0x240, 4, (0x555 << 16) | 0x321); // ID1=0x555, ID2=0x321
periph_write(0x40006400 + 0x244, 4, (0x123 << 16) | 0x456); // ID3=0x123, ID4=0x456
// Exit init mode
periph_write(0x40006400 + 0x200, 4, 0); // FMR: FINIT=0

// Now inject a message with STDID=0x555 (should match)
let msg_tir = ((0x555 << 21) | 1) >>> 0; // TXRQ + STDID=0x555 (unsigned)
let matched = can_inject_message(0x40006400, msg_tir, 8, 0xDEADBEEF, 0x12345678);
assert_eq(matched, true, 'CAN message with STDID=0x555 matched filter');

let rf0r = periph_read(0x40006400 + 0x0C, 4);
assert_eq(rf0r & 0x3, 1, 'CAN RF0R FMP=1 after filter match');

// Read the message back
let rx_tir = periph_read(0x40006400 + 0x1B0, 4);
assert_eq(rx_tir, msg_tir, 'CAN RX TIR matches injected message');
let rx_tdtr = periph_read(0x40006400 + 0x1B4, 4);
assert_eq(rx_tdtr, 8, 'CAN RX TDTR DLC=8');

// Inject a message that should NOT match (STDID=0x999)
let unmatched = can_inject_message(0x40006400, (0x999 << 21) | 1, 4, 0, 0);
assert_eq(unmatched, false, 'CAN message STDID=0x999 rejected by filter');

// ============================================================
// AFIO Register Test
// ============================================================
group('AFIO');

reset();

// Read default AFIO MAPR
let mapr = periph_read(0x40010004, 4);
assert_eq(mapr, 0, 'AFIO MAPR default = 0');

// Write MAPR (remap USART1 to PB6/PB7)
periph_write(0x40010004, 4, 0x40000004);
mapr = periph_read(0x40010004, 4);
assert_eq(mapr, 0x40000004, 'AFIO MAPR write preserves value');

// Write MAPR2 (remap SPI1)
periph_write(0x4001001C, 4, 0x01);
let mapr2 = periph_read(0x4001001C, 4);
assert_eq(mapr2, 0x01, 'AFIO MAPR2 write/read');

// ============================================================
// EXTI Register Test
// ============================================================
group('EXTI');

reset();

// Configure EXTI line 0 for rising edge
periph_write(0x40010400, 4, 1); // IMR: unmask line 0
periph_write(0x40010408, 4, 1); // RTSR: rising edge trigger line 0

// Software trigger line 0
periph_write(0x40010410, 4, 1); // SWIER: set line 0
let swier = periph_read(0x40010410, 4);
assert_eq(swier & 1, 1, 'EXTI SWIER line 0 pending');
let pr = periph_read(0x40010414, 4);
assert_eq(pr & 1, 1, 'EXTI PR line 0 set');

// Clear pending by writing 1 to PR
periph_write(0x40010414, 4, 1);
pr = periph_read(0x40010414, 4);
assert_eq(pr & 1, 0, 'EXTI PR cleared');

// ============================================================
// I2C NACK Test
// ============================================================
group('I2C NACK');

reset();

// Enable I2C1 clock and configure
periph_write(0x4002101C, 4, 1 << 22); // I2C1 clock enable
periph_write(0x40005400, 4, 1); // CR1: PE=1 (enable)

// Generate START condition
periph_write(0x40005400, 4, 0x101); // CR1: PE=1, START=1

// Check SB flag set
let sr1 = periph_read(0x40005414, 4);
assert_eq(sr1 & 1, 1, 'I2C SR1 SB set after START');

// Send address to non-existent device (0x50, write)
periph_write(0x40005410, 4, (0x50 << 1) | 0); // DR = (addr << 1) | R/W

// Should get AF (Acknowledge Failure) in SR1
sr1 = periph_read(0x40005414, 4);
assert_eq(sr1 & (1 << 10), 1 << 10, 'I2C SR1 AF set on missing address');

// ============================================================
// UART RX Interrupt Test
// ============================================================
group('UART RX');

reset();

// Enable USART1 (CR1: UE=1, RE=1, TE=1, RXNEIE=1) and set BRR
periph_write(0x40021018, 4, 1 << 14); // USART1 clock enable
periph_write(0x4001380C, 4, 0x202D); // CR1: UE=1, RE=1, RXNEIE=1, TE=1... actually UE=0
// UE is bit 13, RE is bit 2, TE is bit 3, RXNEIE is bit 5
// CR1 = (1<<13) | (1<<3) | (1<<2) | (1<<5) = 0x202C
periph_write(0x4001380C, 4, (1<<13) | (1<<3) | (1<<2) | (1<<5));
// Enable USART1 IRQ (37) in NVIC
periph_write(0xE000E104, 4, 1 << 5); // ISER1 bit 5

// Inject a byte via RX
uart_rx_byte(0x40013800, 0x42);

// Check RXNE is set
let uart_sr = periph_read(0x40013800, 4);
assert_eq(uart_sr & (1 << 5), 1 << 5, 'UART SR RXNE set after rx_byte');

// Check interrupt is pending (RXNEIE enabled, RXNE set)
assert_eq(has_pending_interrupt(), true, 'UART RX interrupt pending after byte');

// Read the byte from DR
let rx_dr = periph_read(0x40013804, 4);
assert_eq(rx_dr, 0x42, 'UART DR contains injected byte');

// ============================================================
// RTC Alarm Interrupt Test
// ============================================================
group('RTC Alarm');

reset();

// Enable RTC IRQ (3) in NVIC ISER0
periph_write(0xE000E100, 4, 1 << 3);

// Configure RTC: enable, set PRL=99, set ALR=5, enable ALRIE
periph_write(0x40002820, 4, 0);   // ALRH = 0
periph_write(0x40002824, 4, 5);   // ALRL = 5 (alarm = 0x00000005)
periph_write(0x4000280C, 4, 99);  // PRLL = 99 (count every 100 ticks)
periph_write(0x40002800, 4, 1);   // CRH: ALRIE=1
periph_write(0x40002804, 4, 1 << 5);   // CRL: RTOFF=1 (enable)

// No interrupt should be pending yet
assert_eq(has_pending_interrupt(), false, 'RTC no IRQ before alarm');

// Run 600 ticks — RTC should count to 6, passing alarm at 5
for (let i = 0; i < 600; i++) tick();

// Alarm should have fired (IRQ 3)
assert_eq(has_pending_interrupt(), true, 'RTC alarm IRQ pending');
let rtc_irq = get_next_pending_interrupt();
assert_eq(rtc_irq, 3, 'RTC alarm IRQ number = 3');

// ============================================================
// BKP Register Test
// ============================================================
group('BKP');

reset();

// Enable BKP and PWR clock
periph_write(0x4002101C, 4, 1 << 28); // PWREN
periph_write(0x4002101C, 4, (1 << 28) | (1 << 27)); // BKPEN bit 27 + PWREN
// Write and read BKP data register 1
periph_write(0x40006C04, 4, 0x1234);
let bkp_dr1 = periph_read(0x40006C04, 4);
assert_eq(bkp_dr1, 0x1234, 'BKP DR1 write/read');

// Write and read BKP data register 10
periph_write(0x40006C28, 4, 0xABCD);
let bkp_dr10 = periph_read(0x40006C28, 4);
assert_eq(bkp_dr10, 0xABCD, 'BKP DR10 write/read');

// Write BKP RTCCR
periph_write(0x40006C00, 4, 0x0100);
let rtccr = periph_read(0x40006C00, 4);
assert_eq(rtccr, 0x0100, 'BKP RTCCR write/read');

// ============================================================
// DAC Register Test
// ============================================================
group('DAC');

reset();

// Enable DAC clock
periph_write(0x4002101C, 4, 1 << 29); // DACEN

// Write DAC channel 1 12-bit right-aligned data
periph_write(0x40007408, 4, 0x7FF);
let dhr1 = periph_read(0x40007408, 4);
assert_eq(dhr1, 0x7FF, 'DAC DHR12R1 write/read');

// Check DOR1 updated
let dor1 = periph_read(0x4000742C, 4);
assert_eq(dor1, 0x7FF, 'DAC DOR1 reflects DHR12R1');

// Write DAC channel 1 8-bit right-aligned data
periph_write(0x40007410, 4, 0xAB);
let dhr8r1 = periph_read(0x40007410, 4);
assert_eq(dhr8r1, 0xAB, 'DAC DHR8R1 write/read');
dor1 = periph_read(0x4000742C, 4);
assert_eq(dor1, 0xAB0, 'DAC DOR1 = DHR8R1 << 4');

// Read DAC SR
let sr_dac = periph_read(0x40007434, 4);
assert_eq(sr_dac, 0, 'DAC SR default = 0');

// ============================================================
// TIM6 Basic Timer Test
// ============================================================
group('TIM6');

reset();
const T6 = 0x40001000;

// Enable TIM6 clock (APB1, bit 4)
periph_write(0x4002101C, 4, 1 << 4);

// Set PSC=0, ARR=49 (wrap every 50 ticks)
periph_write(T6 + 0x28, 4, 0);
periph_write(T6 + 0x2C, 4, 49);
periph_write(T6 + 0x00, 4, 1); // CEN=1

// Run 30 ticks → CNT should be 30
for (let i = 0; i < 30; i++) tick();
let cnt6 = periph_read(T6 + 0x24, 4);
assert_eq(cnt6, 30, 'TIM6 CNT = 30 after 30 ticks');

// Run 30 more ticks → should wrap (60 % 50 = 10)
for (let i = 0; i < 30; i++) tick();
cnt6 = periph_read(T6 + 0x24, 4);
assert_eq(cnt6, 10, 'TIM6 CNT wrapped to 10');

// ============================================================
// GPIO electrical model (pull-ups, open-drain, slew readback)
// ============================================================
group('GPIO electrical');

reset();
const GPIOA = 0x40010800;

// PA0 input pull-up: CNF=01 (pull), MODE=00, ODR bit0 = 1
periph_write(GPIOA + 0x00, 4, (0b01 << 2) | 0); // CRL[3:0] = 0b01xx? -> CNF=01, MODE=00
periph_write(GPIOA + 0x0C, 4, 0x0001);          // ODR bit0 = 1 (pull-up)
assert_eq(periph_read(GPIOA + 0x08, 4) & 1, 1, 'GPIO PA0 pull-up reads 1');

// PA0 input pull-down: ODR bit0 = 0
periph_write(GPIOA + 0x0C, 4, 0x0000);
assert_eq(periph_read(GPIOA + 0x08, 4) & 1, 0, 'GPIO PA0 pull-down reads 0');

// PA0 input floating, no external driver -> 0
periph_write(GPIOA + 0x00, 4, 0b0100);          // CNF=00 (floating), MODE=00
assert_eq(periph_read(GPIOA + 0x08, 4) & 1, 0, 'GPIO PA0 floating reads 0');

// PA1 push-pull output: CNF=00, MODE=10 (2MHz); ODR drives IDR
periph_write(GPIOA + 0x00, 4, 0b0010 << 4);     // PA1 = push-pull out
periph_write(GPIOA + 0x0C, 4, 0x0002);          // ODR bit1 = 1
assert_eq(periph_read(GPIOA + 0x08, 4) >> 1 & 1, 1, 'GPIO PA1 push-pull high reads 1');
periph_write(GPIOA + 0x0C, 4, 0x0000);
assert_eq(periph_read(GPIOA + 0x08, 4) >> 1 & 1, 0, 'GPIO PA1 push-pull low reads 0');

// PA2 open-drain: CNF=11, MODE=10. Released (ODR=1) + external pull-up -> 1
periph_write(GPIOA + 0x00, 4, (0b1110 << 8) | (0b0010 << 4) | 0b0100);
gpio_set_input(0, 2, true);                     // external pull-up on PA2
periph_write(GPIOA + 0x0C, 4, 0x0004);          // released
assert_eq(periph_read(GPIOA + 0x08, 4) >> 2 & 1, 1, 'GPIO PA2 open-drain released + pull-up reads 1');
periph_write(GPIOA + 0x0C, 4, 0x0000);          // drive low
assert_eq(periph_read(GPIOA + 0x08, 4) >> 2 & 1, 0, 'GPIO PA2 open-drain driven low reads 0');

// External driver wins over push-pull output
periph_write(GPIOA + 0x0C, 4, 0x0002);          // PA1 push-pull high
gpio_set_input(0, 1, false);                    // external driver pulls low
assert_eq(periph_read(GPIOA + 0x08, 4) >> 1 & 1, 0, 'GPIO external driver beats push-pull');
assert_eq(gpio_read_output(0, 1), false, 'gpio_read_output honors external driver');

// Slew: transitions take N instructions; IDR shows the old level meanwhile
// (PA3 push-pull, no external driver registered)
periph_write(GPIOA + 0x00, 4, 0b0010 << 12);    // PA3 = push-pull out
gpio_set_slew(100);
periph_write(GPIOA + 0x10, 4, 1 << 3);          // BSRR set PA3
assert_eq(periph_read(GPIOA + 0x08, 4) >> 3 & 1, 0, 'GPIO slew: IDR still old level during transition');
step_batch(100);
assert_eq(periph_read(GPIOA + 0x08, 4) >> 3 & 1, 1, 'GPIO slew: IDR settled after transition');
gpio_set_slew(0);

// ============================================================
// FSMC external memory (NOR banks, MBKEN/WREN gating)
// ============================================================
group('FSMC');

add_fsmc_bank('FSMC.BANK1', new Uint8Array([0x11, 0x22, 0x33, 0x44]));
reset();
const FSMC_BCR1 = 0xA0000000;
const NE1 = 0x60000000;

// Disabled: reads return 0
assert_eq(periph_read(NE1, 4), 0, 'FSMC NE1 reads 0 when MBKEN=0');

// Enable BCR1 (MBKEN) + WREN, write and read back
periph_write(FSMC_BCR1, 4, 0x3);                // MBKEN | WREN
periph_write(NE1, 4, 0x44332211);
assert_eq(periph_read(NE1, 4), 0x44332211, 'FSMC NE1 32-bit write/read round-trip');

// Byte access
periph_write(NE1 + 1, 1, 0xAB);
assert_eq(periph_read(NE1, 4), 0x4433AB11, 'FSMC NE1 byte write preserves other bytes');
assert_eq(periph_read(NE1 + 1, 1), 0xAB, 'FSMC NE1 byte read');

// Writes ignored without WREN
periph_write(FSMC_BCR1, 4, 0x1);                // MBKEN only
periph_write(NE1, 4, 0xDEADBEEF);
assert_eq(periph_read(NE1, 4), 0x4433AB11, 'FSMC NE1 write ignored without WREN');

// ============================================================
// Sleep state timing (STOP/STANDBY gating)
// ============================================================
group('Sleep');

reset();
const SCB_SCR = 0xE000ED10;
const TIM2S = 0x40000000;

periph_write(0x4002101C, 4, 1 << 0);            // TIM2EN
periph_write(TIM2S + 0x28, 4, 0);               // PSC = 0
periph_write(TIM2S + 0x2C, 4, 0xFFFF);          // ARR
periph_write(TIM2S + 0x00, 4, 1);               // CEN
for (let i = 0; i < 50; i++) tick();
assert_eq(periph_read(TIM2S + 0x24, 4), 50, 'TIM2 CNT = 50 while running');

// Enter STOP: SCR SLEEPDEEP, WFI handled by the core
periph_write(SCB_SCR, 4, 0x4);                  // SLEEPDEEP
for (let i = 0; i < 100; i++) tick();
assert_eq(periph_read(TIM2S + 0x24, 4), 50, 'TIM2 frozen in STOP');

// RTC (LSI/LSE clocked) keeps counting during STOP
periph_write(0x4002101C, 4, 0x200);             // RTCEN
periph_write(0x40002808, 4, 0);                 // PRLH
periph_write(0x4000280C, 4, 9);                 // PRLL: +1 per 10 instr
periph_write(0x4000281C, 4, 0);                 // CNTL = 0
for (let i = 0; i < 100; i++) tick();
const rtcCnt = periph_read(0x4000281C, 4);
assert(rtcCnt >= 8, `RTC keeps counting in STOP (CNT=${rtcCnt})`);

// Wake: clear SLEEPDEEP, timer resumes
periph_write(SCB_SCR, 4, 0x0);
for (let i = 0; i < 20; i++) tick();
assert_eq(periph_read(TIM2S + 0x24, 4), 70, 'TIM2 resumes after wake');

// ============================================================
// Fault exceptions (BusFault/HardFault escalation, SCB state)
// ============================================================
group('Faults');

reset();
const SCB_CFSR = 0xE000ED28;
const SCB_HFSR = 0xE000ED2C;
const SCB_BFAR = 0xE000ED38;

// Fault with BUSFAULTENA disabled -> escalate to HardFault
raise_fault(1, 0x40001234);                     // data read fault
assert_eq(get_next_pending_interrupt(), -13, 'Fault escalates to HardFault when BusFault disabled');
assert_eq(periph_read(SCB_HFSR, 4) >> 30 & 1, 1, 'HFSR FORCED set');
assert_eq(periph_read(SCB_CFSR, 4) & (1 << 15), 1 << 15, 'CFSR BFARVALID set');
assert_eq(periph_read(SCB_BFAR, 4), 0x40001234, 'BFAR holds faulting address');
clear_current_interrupt();

// With BUSFAULTENA enabled -> BusFault handler pends directly
periph_write(0xE000ED24, 4, 1 << 18);           // SHCSR BUSFAULTENA
raise_fault(1, 0x5000ABCD);
assert_eq(get_next_pending_interrupt(), -11, 'BusFault pends when BUSFAULTENA set');
assert_eq(periph_read(SCB_CFSR, 4) & (1 << 9), 1 << 9, 'CFSR PRECISERR set');
assert_eq(periph_read(SCB_BFAR, 4), 0x5000ABCD, 'BFAR updated');
clear_current_interrupt();

// Fetch fault -> IBUSERR
raise_fault(0, 0);
assert_eq(periph_read(SCB_CFSR, 4) & (1 << 8), 1 << 8, 'CFSR IBUSERR set');

// SysTick priority is programmable via SCB SHPR
periph_write(0xE000ED20, 4, 0xFF00FF00);        // SHPR3: SysTick=0, PendSV=0xFF, SVCall=0
assert_eq(periph_read(0xE000ED20, 4) & 0xFF, 0, 'SHPR3 SVCall prio routed through SCB');

// ============================================================
// JS-registered peripheral (rp2040js-style custom chip)
// ============================================================
group('JS Peripheral');

reset();
const JS_BASE = 0x40006800; // gap between CAN1 and BKP on F103 — 4-aligned
let jsWrites = [];
let jsReads = 0;
const regOk = register_js_peripheral(JS_BASE, 0x400,
  (addr, size) => { jsReads++; return addr === JS_BASE ? 0x42 : 0; },
  (addr, value, size) => { jsWrites.push([addr, value, size]); });
assert_eq(regOk, true, 'register_js_peripheral after init returns true');

// read callback fires with the absolute address + size
assert_eq(periph_read(JS_BASE, 4), 0x42, 'JS peripheral read callback value');
assert_eq(jsReads, 1, 'JS peripheral read callback fired');

// write callback fires with (addr, value, size)
periph_write(JS_BASE + 4, 4, 0xDEADBEEF);
assert_eq(jsWrites.length, 1, 'JS peripheral write callback fired');
assert_eq(jsWrites[0][0], JS_BASE + 4, 'JS peripheral write addr absolute');
assert_eq(jsWrites[0][1], 0xDEADBEEF, 'JS peripheral write value');
assert_eq(jsWrites[0][2], 4, 'JS peripheral write size');

// shadow a built-in: register over USART1 and confirm last-wins
register_js_peripheral(0x40013800, 0x400, () => 0x77, () => {});
assert_eq(periph_read(0x40013800, 4), 0x77, 'JS peripheral shadows built-in USART1');

// re-init drops JS peripherals (fresh bus per init)
reset();
assert_eq(periph_read(JS_BASE, 4), 0, 'JS peripheral gone after re-init');

// ============================================================
// Second chip: STM32F105 (connectivity line) from SVD
// ============================================================
group('Chip: STM32F105 (SVD)');

{
  const { readFileSync } = await import('fs');
  const svd = readFileSync(new URL('../svd/STM32F105xx.svd', import.meta.url), 'utf8');
  init_svd(svd);

  // CAN2 (0x40006800) is F105-only — SVD path registers it
  periph_write(0x40006800, 4, 0x00000041); // INRQ + ABOM(6) + TTCM(7)? ABOM is bit 6 on F1 bxCAN
  const can2mcr = periph_read(0x40006800, 4);
  assert_eq(can2mcr & 1, 1, 'F105 CAN2 MCR INRQ bit set');
  assert_eq(can2mcr & (1 << 6), 1 << 6, 'F105 CAN2 MCR ABOM bit set');

  // DMA1 at the real 0x40020000 (SVD map)
  periph_write(0x4002000C, 4, 42);
  assert_eq(periph_read(0x4002000C, 4), 42, 'F105 DMA1 CNDTR at 0x40020000');

  // CAN1 still at its F1 address
  periph_write(0x4000641C, 4, 0x001C0033);
  assert_eq(periph_read(0x4000641C, 4), 0x001C0033, 'F105 CAN1 BTR');

  // Unsupported peripherals in the SVD (ETH) are skipped, not fatal
  assert_eq(periph_read(0x40028000, 4), 0, 'F105 ETH (0x40028000) not mapped (skipped)');
}

// ============================================================
// SDIO host + SD card image (CMD engine, FIFO, IRQ49, DMA2 CH4)
// ============================================================
group('SDIO');

const SDIO = 0x40018000;
const S_POWER = 0x00, S_CLKCR = 0x04, S_ARG = 0x08, S_CMD = 0x0C;
const S_RESPCMD = 0x10, S_RESP1 = 0x14, S_DLEN = 0x28, S_DCTRL = 0x2C;
const S_STA = 0x34, S_ICR = 0x38, S_MASK = 0x3C, S_FIFO = 0x80;
const F_CMDREND = 1 << 6, F_CMDSENT = 1 << 7, F_DATAEND = 1 << 8;
const F_DBCKEND = 1 << 10, F_CTIMEOUT = 1 << 2;
const CPSMEN = 1 << 10, WR_SHORT = 1 << 6, WR_LONG = 3 << 6;
// SVD path: STM32F103.svd lists SDIO @ 0x40018000 — auto-registers, no overlap panic
{
  const { readFileSync } = await import('fs');
  const svd103 = readFileSync(new URL('../svd/STM32F103.svd', import.meta.url), 'utf8');
  init_svd(svd103);
  assert_eq(periph_read(SDIO + S_POWER, 4), 0, 'F103 SVD: SDIO POWER reset 0');
}
// 2048 sectors (1 MiB): CSD C_SIZE = 1; marker pattern per sector.
const sdImg = new Uint8Array(2048 * 512);
for (let i = 0; i < sdImg.length; i++) sdImg[i] = (i >> 9) & 0xFF;
add_sd_card('SDIO', sdImg);
reset();
const sdCmd = (idx, arg, rsp = WR_SHORT) => {
    periph_write(SDIO + S_ARG, 4, arg);
    periph_write(SDIO + S_CMD, 4, (idx & 0x3F) | rsp | CPSMEN);
};

// Register defaults
assert_eq(periph_read(SDIO + S_POWER, 4), 0, 'SDIO POWER reset 0');
assert_eq(periph_read(SDIO + S_STA, 4) & (1 << 19), 1 << 19, 'SDIO RXFIFOE set when idle');
assert_eq(periph_read(SDIO + S_STA, 4) & (1 << 18), 1 << 18, 'SDIO TXFIFOE set when idle');

// POWER + clock
periph_write(SDIO + S_POWER, 4, 0x03);
assert_eq(periph_read(SDIO + S_POWER, 4), 0x03, 'SDIO POWER PWRCTRL=on');
periph_write(SDIO + S_CLKCR, 4, 0x100 | 0x76);
assert_eq(periph_read(SDIO + S_CLKCR, 4), 0x176, 'SDIO CLKCR readback');

// CMD0: no response -> CMDSENT
sdCmd(0, 0, 0);
assert_eq(periph_read(SDIO + S_STA, 4) & F_CMDSENT, F_CMDSENT, 'SDIO CMD0 sets CMDSENT');
assert_eq(periph_read(SDIO + S_RESPCMD, 4), 0, 'SDIO RESPCMD=0');
periph_write(SDIO + S_ICR, 4, 0xFFFFFFFF);
assert_eq(periph_read(SDIO + S_STA, 4) & (F_CMDSENT | F_CMDREND), 0, 'SDIO ICR clears flags');

// CMD8: R7 echoes the argument
sdCmd(8, 0x1AA);
assert_eq(periph_read(SDIO + S_RESP1, 4), 0x1AA, 'SDIO CMD8 R7 echo');
assert_eq(periph_read(SDIO + S_STA, 4) & F_CMDREND, F_CMDREND, 'SDIO CMDREND set');
periph_write(SDIO + S_ICR, 4, 0xFFFFFFFF);

// ACMD41 init: busy for the first polls, then OCR ready + CCS (SDHC)
let ocr = 0;
for (let i = 0; i < 10 && !(ocr & 0x80000000); i++) {
    sdCmd(55, 0);
    assert_eq(periph_read(SDIO + S_RESP1, 4) & 0x20, 0x20, 'SDIO CMD55 R1 APP_CMD bit');
    sdCmd(41, 1 << 30);
    ocr = periph_read(SDIO + S_RESP1, 4);
}
assert((ocr >>> 31) === 1, 'SDIO ACMD41 OCR ready bit sets');
assert(((ocr >>> 30) & 1) === 1, 'SDIO ACMD41 CCS=1 (SDHC)');
periph_write(SDIO + S_ICR, 4, 0xFFFFFFFF);

// CMD2 CID / CMD3 RCA / CMD9 CSD / CMD7 select / CMD16 blocklen
sdCmd(2, 0, WR_LONG);
assert_neq(periph_read(SDIO + S_RESP1, 4), 0, 'SDIO CMD2 CID non-zero');
sdCmd(3, 0);
assert_eq(periph_read(SDIO + S_RESP1, 4) >>> 16, 0x1234, 'SDIO CMD3 R6 RCA');
sdCmd(7, 0x12340000);
assert_eq(periph_read(SDIO + S_RESP1, 4), 0x900, 'SDIO CMD7 R1 ready/tran');
sdCmd(9, 0x12340000, WR_LONG);
assert_eq(periph_read(SDIO + S_RESP1, 4) >>> 30, 1, 'SDIO CMD9 CSD v2.0 structure');
assert_eq(periph_read(SDIO + 0x18, 4) & 0x3F, 0, 'SDIO CMD9 CSD C_SIZE lo for 2048 sectors');
assert_eq(periph_read(SDIO + 0x1C, 4) >>> 16, 1, 'SDIO CMD9 CSD C_SIZE hi for 2048 sectors');
sdCmd(16, 512);
assert_eq(periph_read(SDIO + S_RESP1, 4), 0x900, 'SDIO CMD16 R1');
periph_write(SDIO + S_ICR, 4, 0xFFFFFFFF);

// CMD17 polled single-block read (block 3 = fill byte 3)
periph_write(SDIO + S_DLEN, 4, 512);
periph_write(SDIO + S_DCTRL, 4, 0x1); // DTEN
sdCmd(17, 3);
assert_eq(periph_read(SDIO + S_STA, 4) & F_CMDREND, F_CMDREND, 'SDIO CMD17 CMDREND');
assert_eq(periph_read(SDIO + S_STA, 4) & F_DATAEND, 0, 'SDIO DATAEND not set before drain');
assert_eq(periph_read(SDIO + 0x30, 4), 512, 'SDIO DCOUNT=512 at transfer start');
let word0 = periph_read(SDIO + S_FIFO, 4);
assert_eq(word0, 0x03030303, 'SDIO FIFO first word of block 3');
for (let i = 1; i < 128; i++) {
    const w = periph_read(SDIO + S_FIFO, 4);
    if (w !== 0x03030303) { assert_eq(w, 0x03030303, `SDIO FIFO word ${i} of block 3`); break; }
}
assert_eq(periph_read(SDIO + S_STA, 4) & (F_DATAEND | F_DBCKEND), F_DATAEND | F_DBCKEND, 'SDIO DATAEND+DBCKEND after drain');
assert_eq(periph_read(SDIO + 0x30, 4), 0, 'SDIO DCOUNT=0 after drain');
assert_eq(periph_read(SDIO + S_STA, 4) & (1 << 19), 1 << 19, 'SDIO RXFIFOE after drain');
periph_write(SDIO + S_ICR, 4, 0xFFFFFFFF);

// CMD24 polled write + read-back verify (block 5)
const pat = new Uint32Array(128);
for (let i = 0; i < 128; i++) pat[i] = (0xA5000000 + i) >>> 0;
periph_write(SDIO + S_DLEN, 4, 512);
periph_write(SDIO + S_DCTRL, 4, 0x3); // DTEN + DTDIR(write)
sdCmd(24, 5);
for (let i = 0; i < 128; i++) periph_write(SDIO + S_FIFO, 4, pat[i]);
assert_eq(periph_read(SDIO + S_STA, 4) & (F_DATAEND | F_DBCKEND), F_DATAEND | F_DBCKEND, 'SDIO CMD24 DATAEND after fill');
periph_write(SDIO + S_ICR, 4, 0xFFFFFFFF);
periph_write(SDIO + S_DCTRL, 4, 0x1); // back to read
sdCmd(17, 5);
for (let i = 0; i < 128; i++) {
    const w = periph_read(SDIO + S_FIFO, 4);
    if (w !== pat[i]) { assert_eq(w, pat[i], `SDIO block 5 read-back word ${i}`); break; }
}
assert_eq(periph_read(SDIO + S_STA, 4) & F_DATAEND, F_DATAEND, 'SDIO read-back DATAEND');
periph_write(SDIO + S_ICR, 4, 0xFFFFFFFF);

// IRQ49: mask CMDREND + NVIC ISER1 bit 17, CMD13 fires it
periph_write(0xE000E104, 4, 1 << 17); // ISER1: enable IRQ 49
periph_write(SDIO + S_MASK, 4, F_CMDREND);
sdCmd(13, 0x12340000);
assert(has_pending_interrupt() && get_next_pending_interrupt() === 49,
    'SDIO CMDREND pends IRQ 49 when masked+enabled');
clear_current_interrupt();
periph_write(SDIO + S_ICR, 4, 0xFFFFFFFF);
periph_write(SDIO + S_MASK, 4, 0);

// DMA RX via DMA2 CH4: program channel, CMD17 + DMAEN, pump absorbs image bytes
const DMA2 = 0x40020400, CH4 = 0x08 + 3 * 0x14;
periph_write(DMA2 + CH4, 4, (2 << 10) | (2 << 8) | (1 << 7) | 0); // MSIZE/PSIZE=32b, MINC, EN=0
periph_write(DMA2 + CH4 + 2 * 4, 4, SDIO + S_FIFO); // CPAR = FIFO
periph_write(DMA2 + CH4 + 3 * 4, 4, 0x20000000);    // CMAR (no Unicorn here: plan only)
periph_write(DMA2 + CH4 + 1 * 4, 4, 128);          // CNDTR = 128 words
periph_write(DMA2 + CH4, 4, (2 << 10) | (2 << 8) | (1 << 7) | 1); // EN (DIR=0: periph->mem)
periph_write(SDIO + S_DLEN, 4, 512);
periph_write(SDIO + S_DCTRL, 4, 0x9); // DTEN + DMAEN
sdCmd(17, 7);
step_batch(1); // DMA2 tick queues the transfer
assert_eq(periph.dma_get_pending_count() >= 1, true, 'SDIO DMA RX queues a transfer');
// The pump plan must absorb 512 B from the FIFO (op 1), served from block 7.
const plan = periph.dma_pump_all();
let absorb = null;
for (let i = 0; i + 4 <= plan.length; i += 4) {
    if (plan[i] === 1 && plan[i + 2] === 512) absorb = [plan[i + 1], plan[i + 3]];
}
assert_eq(absorb !== null, true, 'SDIO DMA pump plan absorbs 512 B');
const taken = new Uint8Array(periph.dma_take_absorbed(absorb[1], 512));
let dmaOk = taken.length === 512;
for (let i = 0; i < 512 && dmaOk; i++) if (taken[i] !== 7) dmaOk = false;
assert_eq(dmaOk, true, 'SDIO DMA absorbed bytes are block 7 fill');
periph.dma_set_completed_many(1 << 10); // global stream 10 = DMA2 CH4
step_batch(1);
assert_eq(periph_read(DMA2 + 0x00, 4) & (1 << 13), 1 << 13, 'SDIO DMA2 ISR TCIF4 after completion');
assert_eq(periph_read(DMA2 + CH4 + 1 * 4, 4), 0, 'SDIO DMA2 CH4 CNDTR=0 after completion');

// No card attached: CMD8 times out, CMD0 still sends
reset_ext_devices();
reset();
sdCmd(8, 0x1AA);
assert_eq(periph_read(SDIO + S_STA, 4) & F_CTIMEOUT, F_CTIMEOUT, 'SDIO no-card CMD8 CTIMEOUT');
assert_eq(periph_read(SDIO + S_RESP1, 4), 0, 'SDIO no-card CMD8 no response');
sdCmd(0, 0, 0);
assert_eq(periph_read(SDIO + S_STA, 4) & F_CMDSENT, F_CMDSENT, 'SDIO no-card CMD0 CMDSENT');

// ============================================================
// Summary
// ============================================================
console.log(`\n${'='.repeat(50)}`);
console.log(`Results: ${passed} passed, ${failed} failed, ${passed+failed} total`);
if (failed === 0) console.log('ALL TESTS PASSED');
else console.log('SOME TESTS FAILED');
process.exit(failed > 0 ? 1 : 0);
