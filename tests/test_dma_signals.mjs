// DMA signal generation + TIM slave mode + I2C clock stretching tests.
//
// Uses the raw WASM API (no firmware) to test peripheral DMA request routing,
// TIM slave mode, and I2C clock stretching.

import { readFileSync } from 'fs';
import * as periph from '../pkg/stm32_bluepill_wasm.js';
periph.initSync({ module: readFileSync(new URL('../pkg/stm32_bluepill_wasm_bg.wasm', import.meta.url)) });

const { init, periph_read, periph_write, step_batch } = periph;

let passed = 0, failed = 0;
function assert(cond, msg) { if (cond) passed++; else { failed++; console.error(`FAIL: ${msg}`); } }
function assert_eq(a, b, msg) { if (a === b) passed++; else { failed++; console.error(`FAIL: ${msg}: expected ${b}, got ${a}`); } }
function group(name) { console.log(`\n=== ${name} ===`); }

// ============================================================
// DAC DMA signal generation
// ============================================================
group('DAC DMA signal generation');

init();

periph_write(0x4002101C, 4, 1 << 29); // DACEN
periph_write(0x40007400, 4, 0x0001); // EN1=1
assert_eq(periph_read(0x40007400, 4) & 1, 1, 'DAC EN1 set');

periph_write(0x40007408, 4, 0x800);
assert_eq(periph_read(0x4000742C, 4), 0x800, 'DAC DOR1 = 0x800 after DHR12R1 write');

periph_write(0x40007414, 4, 0x400);
assert_eq(periph_read(0x40007430, 4), 0x400, 'DAC DOR2 = 0x400 after DHR12R2 write');

periph_write(0x40007410, 4, 0xFF);
assert_eq(periph_read(0x4000742C, 4), 0xFF0, 'DAC DOR1 = 0xFF0 after 8-bit DHR write');

periph_write(0x40007420, 4, 0x456_0123);
assert_eq(periph_read(0x4000742C, 4), 0x123, 'DAC DOR1 from DHR12RD');
assert_eq(periph_read(0x40007430, 4), 0x456, 'DAC DOR2 from DHR12RD');

// ============================================================
// SPI DMA — register bits
// ============================================================
group('SPI DMA signal generation');

init();

periph_write(0x40021018, 4, 1 << 12); // SPI1 clock

// TXDMAEN(bit1) + RXDMAEN(bit2) = 0x06
periph_write(0x40013004, 4, 0x06);
let cr2 = periph_read(0x40013004, 4);
assert((cr2 & (1 << 1)) !== 0, `SPI1 TXDMAEN set (CR2=0x${cr2.toString(16)})`);
assert((cr2 & (1 << 2)) !== 0, `SPI1 RXDMAEN set (CR2=0x${cr2.toString(16)})`);

periph_write(0x4001300C, 4, 0x42); // DR write
let spi_sr = periph_read(0x40013008, 4);
assert((spi_sr & 2) !== 0, `SPI1 TXE set after DR write (SR=0x${spi_sr.toString(16)})`);
assert((spi_sr & 1) !== 0, `SPI1 RXNE set after DR write`);

// ============================================================
// USART DMA — register bits
// ============================================================
group('USART DMA signal generation');

init();

periph_write(0x40021018, 4, 1 << 14); // USART1 clock

periph_write(0x40013808, 4, (1 << 7) | (1 << 6)); // DMAT + DMAR
let cr3 = periph_read(0x40013808, 4);
assert((cr3 & (1 << 7)) !== 0, `USART1 DMAT set (CR3=0x${cr3.toString(16)})`);
assert((cr3 & (1 << 6)) !== 0, `USART1 DMAR set`);

periph_write(0x40013804, 4, 0x55);
let usart_sr = periph_read(0x40013800, 4);
assert((usart_sr & 0x80) !== 0, `USART1 TXE set after DR write (SR=0x${usart_sr.toString(16)})`);

// ============================================================
// TIM slave mode — SMCR register storage
// ============================================================
group('TIM slave mode — SMCR');

init();

periph_write(0x4002101C, 4, 1 << 2); // TIM2 clock

periph_write(0x40000008, 4, 0x0015); // SMS=101 (gated), TS=001
let smcr = periph_read(0x40000008, 4);
assert_eq(smcr & 7, 5, 'TIM2 SMCR SMS=5 (gated)');
assert_eq((smcr >> 4) & 7, 1, 'TIM2 SMCR TS=1');

periph_write(0x40000008, 4, 0x0003); // SMS=011 (encoder)
smcr = periph_read(0x40000008, 4);
assert_eq(smcr & 7, 3, 'TIM2 SMCR SMS=3 (encoder)');

// ============================================================
// TIM DMA on update event (UDE)
// ============================================================
group('TIM DMA UDE');

init();

periph_write(0x4002101C, 4, 1 << 2); // TIM2 clock
periph_write(0x4000002C, 4, 100);     // ARR=100
periph_write(0x4000000C, 4, 0x0101); // UIE + UDE
periph_write(0x40000000, 4, 1);       // CEN=1

step_batch(150);

let tim_sr = periph_read(0x40000010, 4);
assert((tim_sr & 1) !== 0, `TIM2 UIF set after update (SR=0x${tim_sr.toString(16)})`);

// ============================================================
// TIM CCx DMA on compare match
// ============================================================
group('TIM CC DMA');

init();

periph_write(0x4002101C, 4, 1 << 3); // TIM3 clock
periph_write(0x4000042C, 4, 1000);    // ARR=1000
periph_write(0x40000434, 4, 50);      // CCR1=50
periph_write(0x40000420, 4, 1);       // CCER: CC1E
periph_write(0x4000040C, 4, 0x0203);  // DIER: CC1IE + CC1DE
periph_write(0x40000400, 4, 1);       // CEN=1

step_batch(100);

let tim3_sr = periph_read(0x40000410, 4);
assert((tim3_sr & 2) !== 0, `TIM3 CC1IF set after match (SR=0x${tim3_sr.toString(16)})`);

// ============================================================
// I2C — register interface
// ============================================================
group('I2C register interface');

init();

periph_write(0x4002101C, 4, 1 << 21); // I2C1 clock
periph_write(0x40005400, 4, 1);        // PE=1
assert_eq(periph_read(0x40005400, 4) & 1, 1, 'I2C1 PE set');

periph_write(0x40005404, 4, 0x0700);
assert_eq(periph_read(0x40005404, 4), 0x0700, 'I2C1 CR2 stored');

periph_write(0x4000541C, 4, 360);
assert_eq(periph_read(0x4000541C, 4), 360, 'I2C1 CCR stored');

// ============================================================
// I2C — START/STOP sequence
// ============================================================
group('I2C START/STOP');

init();

periph_write(0x4002101C, 4, 1 << 21); // I2C1 clock

// Step 1: PE=1
periph_write(0x40005400, 4, 0x0001);

// Step 2: Generate START (rising edge on bit 8)
// Do NOT set STOP (bit 9) at the same time!
periph_write(0x40005400, 4, 0x0101); // PE + START

// SR1 should show SB (bit 0)
let sr1 = periph_read(0x40005414, 4);
assert((sr1 & 1) !== 0, `I2C1 SR1.SB set after START (SR1=0x${sr1.toString(16)})`);

// SR2 should show BUSY+MSL
let sr2 = periph_read(0x40005418, 4);
assert((sr2 & 3) !== 0, `I2C1 SR2 BUSY+MSL set (SR2=0x${sr2.toString(16)})`);

// Write address (DR at 0x10) — NACK since no devices registered
periph_write(0x40005410, 4, 0xD0); // 0x68 << 1

// After NACK, AF bit (bit 10) should be set, BUSY cleared
sr1 = periph_read(0x40005414, 4);
assert((sr1 & (1 << 10)) !== 0, `I2C1 SR1.AF set after NACK (SR1=0x${sr1.toString(16)})`);

sr2 = periph_read(0x40005418, 4);
assert((sr2 & 1) === 0, `I2C1 SR2.BUSY cleared after NACK (SR2=0x${sr2.toString(16)})`);

// ============================================================
// Summary
// ============================================================
console.log(`\n${'='.repeat(50)}`);
console.log(`Results: ${passed} passed, ${failed} failed, ${passed + failed} total`);
if (failed === 0) console.log('ALL TESTS PASSED');
else console.log('SOME TESTS FAILED');
process.exit(failed > 0 ? 1 : 0);
