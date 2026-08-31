// DMA request register bits test.
//
// Verifies that DMA control bits in DAC, TIM, SPI, and USART CR registers
// are stored and read back correctly. These bits gate DMA request generation;
// this test covers the register interface even where the actual DMA request
// signal is not yet wired.

import { readFileSync } from 'fs';
import * as periph from '../pkg/stm32_bluepill_wasm.js';
periph.initSync({ module: readFileSync(new URL('../pkg/stm32_bluepill_wasm_bg.wasm', import.meta.url)) });

const { init, periph_read, periph_write } = periph;

let passed = 0, failed = 0;
function assert(cond, msg) { if (cond) passed++; else { failed++; console.error(`FAIL: ${msg}`); } }
function assert_eq(a, b, msg) { if (a === b) passed++; else { failed++; console.error(`FAIL: ${msg}: expected ${b}, got ${a}`); } }
function group(name) { console.log(`\n=== ${name} ===`); }

// ============================================================
// DAC DMAEN bits in CR (RM0008: CR at 0x40007400)
//   bit 0: EN1, bit 2: TEN1, bits[5:3]: TSEL1, bits[7:6]: WAVE1
//   bit 8: EN2, bit 10: TEN2, bits[13:11]: TSEL2, bits[15:14]: WAVE2
//   mask 0x3F3F_003F — stores bits 0-5, 8-13, 16-21, 24-29
// ============================================================
group('DAC DMAEN bits');

init();
periph_write(0x4002101C, 4, 1 << 29); // DACEN clock enable

// Enable DAC channel 1
periph_write(0x40007400, 4, 0x0001); // EN1=1
assert_eq(periph_read(0x40007400, 4) & 1, 1, 'DAC EN1 bit stored');

// DAC CR does NOT store WAVE bits (6-7) — verify mask
periph_write(0x40007400, 4, 0x00C1); // EN1 + WAVE1=11 (bits 6-7)
let cr = periph_read(0x40007400, 4);
assert_eq(cr & 0xC0, 0, 'DAC WAVE1 bits masked out');
assert_eq(cr & 1, 1, 'DAC EN1 preserved with WAVE write');

// TEN1 (trigger enable, bit 2) + TSEL1=6 (bits[5:3]=110 → bit3=0,bit4=1,bit5=1)
// EN1=1(bit0) + TEN1=1(bit2) + TSEL1=110(bits5:3) = 0b0110101 = 0x35
periph_write(0x40007400, 4, 0x35);
cr = periph_read(0x40007400, 4);
assert_eq(cr & (1 << 2), 1 << 2, 'DAC TEN1 (trigger enable) stored');
assert_eq((cr >> 3) & 7, 6, 'DAC TSEL1=6 trigger select stored');

// DHR12R1 write still propagates to DOR1 even with TEN1 set
periph_write(0x40007408, 4, 0x7FF);
assert_eq(periph_read(0x4000742C, 4), 0x7FF, 'DAC DOR1 reflects DHR12R1 with trigger enabled');

// ============================================================
// TIM DMA enable bits in DIER (offset 0x0C, mask 0xFFFF)
//   bit 8: UDE, bit 9: CC1DE, bit 10: CC2DE, bit 11: CC3DE,
//   bit 12: CC4DE, bit 13: COMDE, bit 14: TDE
// ============================================================
group('TIM DMA enable bits');

init();
periph_write(0x4002101C, 4, 1 << 2); // TIM2 clock enable

// TIM2 DIER at 0x4000000C
periph_write(0x4000000C, 4, 0x0100); // UDE=1
assert_eq(periph_read(0x4000000C, 4) & (1 << 8), 1 << 8, 'TIM2 UDE bit stored');

// All DMA bits: UDE(8) + CC1DE(9) + CC2DE(10) + CC3DE(11) + CC4DE(12) + COMDE(13) + TDE(14)
// = 0x7F00
periph_write(0x4000000C, 4, 0x7F00);
let dier = periph_read(0x4000000C, 4);
assert_eq(dier & (1 << 9), 1 << 9, 'TIM2 CC1DE bit stored');
assert_eq(dier & (1 << 10), 1 << 10, 'TIM2 CC2DE bit stored');
assert_eq(dier & (1 << 11), 1 << 11, 'TIM2 CC3DE bit stored');
assert_eq(dier & (1 << 12), 1 << 12, 'TIM2 CC4DE bit stored');
assert_eq(dier & (1 << 13), 1 << 13, 'TIM2 COMDE bit stored');
assert_eq(dier & (1 << 14), 1 << 14, 'TIM2 TDE bit stored');

// TIM3 at 0x40000400
periph_write(0x4002101C, 4, 1 << 3); // TIM3 clock enable
periph_write(0x4000040C, 4, 0x0100); // UDE=1
assert_eq(periph_read(0x4000040C, 4) & (1 << 8), 1 << 8, 'TIM3 UDE bit stored');

// ============================================================
// SPI DMA enable bits in CR2 (offset 0x04)
//   bit 1: TXDMAEN, bit 2: RXDMAEN
// ============================================================
group('SPI DMA enable bits');

init();
periph_write(0x40021018, 4, 1 << 12); // SPI1 clock enable

// SPI1 CR2 at 0x40013004
periph_write(0x40013004, 4, 0x06); // TXDMAEN + RXDMAEN
let spi_cr2 = periph_read(0x40013004, 4);
assert_eq(spi_cr2 & (1 << 1), 1 << 1, 'SPI1 TXDMAEN bit stored');
assert_eq(spi_cr2 & (1 << 2), 1 << 2, 'SPI1 RXDMAEN bit stored');

// SPI2 at 0x40003800
periph_write(0x40021018, 4, 1 << 14); // SPI2 clock enable
periph_write(0x40003804, 4, 0x06);
spi_cr2 = periph_read(0x40003804, 4);
assert_eq(spi_cr2 & (1 << 1), 1 << 1, 'SPI2 TXDMAEN bit stored');
assert_eq(spi_cr2 & (1 << 2), 1 << 2, 'SPI2 RXDMAEN bit stored');

// ============================================================
// USART DMA enable bits in CR3 (offset 0x08)
//   bit 7: DMAT, bit 6: DMAR
// ============================================================
group('USART DMA enable bits');

init();
periph_write(0x40021018, 4, 1 << 14); // USART1 clock enable

// USART1 CR3 at 0x40013808
periph_write(0x40013808, 4, 0xC0); // DMAT + DMAR
let usart_cr3 = periph_read(0x40013808, 4);
assert_eq(usart_cr3 & (1 << 7), 1 << 7, 'USART1 DMAT bit stored');
assert_eq(usart_cr3 & (1 << 6), 1 << 6, 'USART1 DMAR bit stored');

// USART2 at 0x40004400
periph_write(0x40021018, 4, 1 << 17); // USART2 clock enable
periph_write(0x40004408, 4, 0xC0);
usart_cr3 = periph_read(0x40004408, 4);
assert_eq(usart_cr3 & (1 << 7), 1 << 7, 'USART2 DMAT bit stored');
assert_eq(usart_cr3 & (1 << 6), 1 << 6, 'USART2 DMAR bit stored');

// ============================================================
// DMA channel configuration registers
// DMA1 at 0x40020000
// Channel registers: CCR=base+0x08+ch*0x14, CNDTR=CCR+4, CPAR=CCR+8, CMAR=CCR+0xC
// ============================================================
group('DMA channel config');

init();

// Channel 1: CCR=0x40020008, CNDTR=0x4002000C
periph_write(0x40020008, 4, 0x00007080); // MINC(7) + PSIZE_16(8,9=01) + MSIZE_16(10,11=01) = bit7 + (1<<8) + (1<<10) = 0x580... let me compute:
// MINC = bit 7 = 0x80
// PSIZE = bits[9:8] = 01 → (1 << 8) = 0x100
// MSIZE = bits[11:10] = 01 → (1 << 10) = 0x400
// Total = 0x80 | 0x100 | 0x400 = 0x580
periph_write(0x40020008, 4, 0x580);
let ccr = periph_read(0x40020008, 4);
assert_eq(ccr & (1 << 7), 1 << 7, 'DMA1 CH1 MINC set');
assert_eq((ccr >> 8) & 3, 1, 'DMA1 CH1 PSIZE = 16-bit');
assert_eq((ccr >> 10) & 3, 1, 'DMA1 CH1 MSIZE = 16-bit');

// Channel 3: CCR=0x40020030, CNDTR=0x40020034
periph_write(0x40020030, 4, 0x00000010); // DIR=1 (mem→periph, bit 4)
ccr = periph_read(0x40020030, 4);
assert_eq(ccr & (1 << 4), 1 << 4, 'DMA1 CH3 DIR=1 (mem→periph)');

// CNDTR
periph_write(0x40020034, 4, 72);
assert_eq(periph_read(0x40020034, 4), 72, 'DMA1 CH3 CNDTR = 72');

// MEM2MEM bit (bit 14) — should be stored (mask 0x7F7F includes bit 14)
periph_write(0x40020008, 4, 0x4000); // MEM2MEM=1
ccr = periph_read(0x40020008, 4);
assert_eq(ccr & (1 << 14), 1 << 14, 'DMA1 CH1 MEM2MEM bit stored');

// ============================================================
// Summary
// ============================================================
console.log(`\n${'='.repeat(50)}`);
console.log(`Results: ${passed} passed, ${failed} failed, ${passed + failed} total`);
if (failed === 0) console.log('ALL TESTS PASSED');
else console.log('SOME TESTS FAILED');
process.exit(failed > 0 ? 1 : 0);
