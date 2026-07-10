import * as periph from './pkg/stm32_bluepill_wasm.js';

const { init, periph_read, periph_write, tick, has_pending_interrupt,
        get_next_pending_interrupt, gpio_read_output, gpio_set_input,
        gpio_read_input, get_uart_output, uart_rx_byte, adc_set_sim_value,
        is_watchdog_reset_requested } = periph;

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

// Check EOC in SR bit 1
let sradc = periph_read(ADC1 + 0x00, 4);
assert_eq(sradc & (1 << 1), 1 << 1, 'ADC SR EOC after SWSTART');

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
assert_eq(sradc & (1 << 1), 1 << 1, 'ADC SR EOC after second SWSTART');
dr_val = periph_read(ADC1 + 0x4C, 4) & 0xFFF;
assert_eq(dr_val, 0x155, 'ADC DR second value');

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
assert_eq(periph_read(SPI1 + 0x0C, 4), 0xFF, 'SPI1 DR xfer returns 0xFF (no device)');

// CRCPR at offset 0x10 stores value directly
periph_write(SPI1 + 0x10, 4, 0x07);
assert_eq(periph_read(SPI1 + 0x10, 4), 0x07, 'SPI1 CRCPR');

// Read SR
let spi_sr = periph_read(SPI1 + 0x08, 4);
assert_eq(spi_sr & (1 << 1), 1 << 1, 'SPI1 SR TXE');
assert_eq(spi_sr & (1 << 0), 1 << 0, 'SPI1 SR RXNE');

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

// Write to RTC CNT (counter)
periph_write(RTC + 0x08, 4, 0x1234);
assert_eq(periph_read(RTC + 0x08, 4), 0x1234, 'RTC CNT');

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
const DMA1 = 0x40006000;

// Enable DMA1 clock
periph_write(0x4002101C, 4, 1 << 0); // DMA1EN on APB1

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
periph_write(0x4002101C, 4, 1 << 0);
periph_write(DMA1 + 0x08 + 0*0x14, 4, 0); // disable first
periph_write(DMA1 + 0x08 + 2*4, 4, 0x20000000); // CPAR
periph_write(DMA1 + 0x08 + 3*4, 4, 0x20001000); // CMAR
periph_write(DMA1 + 0x08 + 1*4, 4, 8); // CNDTR = 8
// Enable with EN bit and DIR=memory-to-memory, MINC, PINC
periph_write(DMA1 + 0x08 + 0*0x14, 4, (1 << 14) | (1 << 7) | (1 << 6) | 1);
assert_eq(periph_read(DMA1 + 0x08 + 1*4, 4), 0, 'DMA1 CNDTR decremented to 0 on enable');
let dma_pending = periph.dma_get_pending_count();
assert_eq(dma_pending >= 1, true, 'DMA has pending transfers');

// ============================================================
// SCB
// ============================================================
group('SCB');

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
// Summary
// ============================================================
console.log(`\n${'='.repeat(50)}`);
console.log(`Results: ${passed} passed, ${failed} failed, ${passed+failed} total`);
if (failed === 0) console.log('ALL TESTS PASSED');
else console.log('SOME TESTS FAILED');
process.exit(failed > 0 ? 1 : 0);
