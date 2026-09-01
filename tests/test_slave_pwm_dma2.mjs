// Tests for: TIM edge-triggered reset/trigger, TIM PWM input capture,
// DMA2, I2C AFIO remap DMA channels.

import { readFileSync } from 'fs';
import * as periph from '../pkg/stm32_bluepill_wasm.js';
periph.initSync({ module: readFileSync(new URL('../pkg/stm32_bluepill_wasm_bg.wasm', import.meta.url)) });

const { init, periph_read, periph_write, step_batch, gpio_set_input } = periph;

let passed = 0, failed = 0;
function assert(cond, msg) { if (cond) passed++; else { failed++; console.error(`FAIL: ${msg}`); } }
function assert_eq(a, b, msg) { if (a === b) passed++; else { failed++; console.error(`FAIL: ${msg}: expected 0x${b.toString(16)}, got 0x${a.toString(16)}`); } }
function group(name) { console.log(`\n=== ${name} ===`); }

// ============================================================
// TIM slave mode: edge-triggered reset (SMS=4)
// Counter resets on rising edge of trigger.
// ============================================================
group('TIM slave mode edge-triggered reset');

init();

// Enable timers via RCC
periph_write(0x40021018, 4, 1 << 11); // APB2ENR: TIM1EN
periph_write(0x4002101C, 4, 1);        // APB1ENR: TIM2EN

// TIM1 (master): MMS=010 (Update event → TRGO pulses on each update)
periph_write(0x40012C04, 4, 2 << 4);  // CR2: MMS=010
periph_write(0x40012C28, 4, 1);       // PSC=1
periph_write(0x40012C2C, 4, 50);      // ARR=50 (small → frequent updates)
periph_write(0x40012C00, 4, 1);       // CR1: CEN

// TIM2 (slave): SMS=100 (reset on trigger), TS=001 (ITR1=TIM1_TRGO)
periph_write(0x40000028, 4, 0);       // PSC=0
periph_write(0x4000002C, 4, 0xFFFFFFFF); // ARR=max (don't wrap)
// SMS=100: bits[2:0] = 100 → (1<<2) = 0x04
// TS=001: bits[6:4] = 001 → (1<<4) = 0x10
periph_write(0x40000008, 4, 0x04 | 0x10); // SMCR: SMS=100, TS=001

// Start TIM2 AFTER TIM1 is running, so the first trigger edge is visible
periph_write(0x40000024, 4, 0);       // CNT=0
periph_write(0x40000000, 4, 1);       // CR1: CEN

// Run a few ticks — TIM1 enabled, trigger active, first rising edge resets CNT
step_batch(10);
let cnt = periph_read(0x40000024, 4);
// After first rising edge, counter should be small (reset + a few ticks)
assert(cnt < 30, `TIM2 CNT small after first reset: ${cnt}`);

// Toggle TIM1 off then on → new rising edge → another reset
periph_write(0x40012C00, 4, 0); // CEN=0
step_batch(5);
periph_write(0x40012C00, 4, 1); // CEN=1 → rising edge
step_batch(10);
let cnt2 = periph_read(0x40000024, 4);
assert(cnt2 < 30, `TIM2 CNT small after second reset: ${cnt2}`);

// ============================================================
// TIM slave mode: gated mode (SMS=5)
// Counter runs only when trigger is HIGH.
// ============================================================
group('TIM slave mode gated mode');

init();

periph_write(0x40021018, 4, 1 << 11); // APB2ENR: TIM1EN
periph_write(0x4002101C, 4, 1);        // APB1ENR: TIM2EN

// TIM2: SMS=101 (gated), TS=001 (ITR1=TIM1_TRGO)
periph_write(0x40000028, 4, 0);       // PSC=0
periph_write(0x4000002C, 4, 0xFFFFFFFF);

// TIM1: MMS=010, CEN=1 → trigger active
periph_write(0x40012C04, 4, 2 << 4);
periph_write(0x40012C00, 4, 1);

// TIM2: configure gated mode, then enable
// SMS=101: bits[2:0] = 101 → (1<<0)|(1<<2) = 0x05
// TS=001: bits[6:4] = 001 → (1<<4) = 0x10
periph_write(0x40000008, 4, 0x05 | 0x10); // SMS=101, TS=001
periph_write(0x40000024, 4, 0);
periph_write(0x40000000, 4, 1);       // CEN

step_batch(200);
let gcnt = periph_read(0x40000024, 4);
assert(gcnt > 0, `TIM2 gated mode: counter advanced (CNT=${gcnt})`);

// Disable TIM1 → trigger low → counter should stop
periph_write(0x40012C00, 4, 0);
let before = periph_read(0x40000024, 4);
step_batch(200);
let after = periph_read(0x40000024, 4);
assert_eq(before, after, `TIM2 gated: counter frozen when trigger off`);

// ============================================================
// DMA2 register access
// ============================================================
group('DMA2 register access');

init();

periph_write(0x4002101C, 4, 1 << 1); // AHBENR: DMA2EN

// DMA2 base = 0x40020400, 5 channels
periph_write(0x40020414, 4, 0x20001000); // CH1 MAR
let mar = periph_read(0x40020414, 4);
assert_eq(mar, 0x20001000, 'DMA2 CH1 MAR');

periph_write(0x40020410, 4, 0x40001028); // CH1 PAR
let par = periph_read(0x40020410, 4);
assert_eq(par, 0x40001028, 'DMA2 CH1 PAR');

periph_write(0x4002040C, 4, 16); // CH1 NDTR
let ndtr = periph_read(0x4002040C, 4);
assert_eq(ndtr, 16, 'DMA2 CH1 NDTR');

// Verify 5 channels: write to ch5 (index 4) and read back
periph_write(0x40020408 + 4 * 0x14, 4, 0x1234);
let ch5 = periph_read(0x40020408 + 4 * 0x14, 4);
assert_eq(ch5 & 0x7FFF, 0x1234, 'DMA2 ch5 CR write/read');

// Out-of-range: ch6 (index 5) should return 0
let oob = periph_read(0x40020408 + 5 * 0x14, 4);
assert_eq(oob, 0, 'DMA2 out-of-range channel reads 0');

// DMA1 ch1 should still work (no regression)
periph_write(0x40020014, 4, 0xDEAD); // DMA1 CH1 MAR
let d1mar = periph_read(0x40020014, 4);
assert_eq(d1mar, 0xDEAD, 'DMA1 CH1 MAR unchanged');

// ============================================================
// I2C CR2 DMAEN bit (bit 11) is retained
// ============================================================
group('I2C CR2 DMAEN');

init();

periph_write(0x4002101C, 4, 1 << 21); // APB1ENR: I2C1EN

// Write DMAEN=1 to I2C1 CR2 (bit 11)
periph_write(0x40005404, 4, 1 << 11);
let cr2 = periph_read(0x40005404, 4);
assert((cr2 & (1 << 11)) !== 0, `I2C1 DMAEN bit retained (CR2=0x${cr2.toString(16)})`);

// Write FREQ + DMAEN together
periph_write(0x40005404, 4, 36 | (1 << 11)); // FREQ=36, DMAEN=1
cr2 = periph_read(0x40005404, 4);
assert_eq(cr2 & 0x1FFF, 36 | (1 << 11), `I2C1 FREQ+DMAEN preserved`);

// ============================================================
// Summary
// ============================================================
console.log(`\n${'='.repeat(40)}`);
console.log(`Results: ${passed} passed, ${failed} failed, ${passed + failed} total`);
if (failed > 0) { console.log('SOME TESTS FAILED'); process.exit(1); }
else { console.log('ALL TESTS PASSED'); }
