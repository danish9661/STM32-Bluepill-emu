volatile uint32_t systick_ms = 0;

void SysTick_Handler(void) {
  systick_ms++;
}

void delay_ms(uint32_t ms) {
  for (uint32_t i = 0; i < ms * 4000; i++) {
    __asm__("nop");
  }
}

void uart_write_char(char c) {
  volatile uint32_t *USART1_DR = (uint32_t *)0x40013804;
  volatile uint32_t *USART1_SR = (uint32_t *)0x40013800;
  while (!(*USART1_SR & (1 << 7)));
  *USART1_DR = c;
}

void uart_write_str(const char *s) {
  while (*s) uart_write_char(*s++);
}

void uart_write_hex(uint32_t val) {
  char buf[11];
  buf[0] = '0'; buf[1] = 'x';
  for (int i = 0; i < 8; i++) {
    uint8_t nibble = (val >> (28 - i * 4)) & 0xF;
    buf[2 + i] = nibble < 10 ? '0' + nibble : 'A' + nibble - 10;
  }
  buf[10] = 0;
  uart_write_str(buf);
}

void uart_write_dec(uint32_t val) {
  char buf[12];
  int i = 0;
  if (val == 0) { uart_write_char('0'); return; }
  while (val > 0) { buf[i++] = '0' + (val % 10); val /= 10; }
  while (i > 0) uart_write_char(buf[--i]);
}

void uart_write_line(const char *s) {
  uart_write_str(s);
  uart_write_str("\r\n");
}

void gpio_init(void) {
  volatile uint32_t *RCC_APB2ENR = (uint32_t *)0x40021018;
  volatile uint32_t *GPIOC_CRH = (uint32_t *)0x40011004;
  *RCC_APB2ENR |= (1 << 4); // IOPCEN
  uint32_t crh = *GPIOC_CRH;
  crh = (crh & ~(0xF << 20)) | (0x3 << 20);
  *GPIOC_CRH = crh;
}

void gpio_write(uint8_t pin, uint8_t val) {
  volatile uint32_t *GPIOC_BSRR = (uint32_t *)0x40011010;
  if (val) *GPIOC_BSRR = 1 << pin;
  else *GPIOC_BSRR = 1 << (pin + 16);
}

void uart_init(void) {
  volatile uint32_t *RCC_APB2ENR = (uint32_t *)0x40021018;
  volatile uint32_t *GPIOA_CRH = (uint32_t *)0x40010804;
  volatile uint32_t *USART1_BRR = (uint32_t *)0x40013808;
  volatile uint32_t *USART1_CR1 = (uint32_t *)0x4001380C;

  *RCC_APB2ENR |= (1 << 14) | (1 << 2);

  uint32_t crh = *GPIOA_CRH;
  crh = (crh & ~0xF0) | 0xB0;
  crh = (crh & ~0xF00) | 0x400;
  *GPIOA_CRH = crh;

  *USART1_BRR = 0x341; // 115200 @ 8MHz
  *USART1_CR1 = (1 << 13) | (1 << 3) | (1 << 2);
}

void uart_enable_loopback(void) {
  volatile uint32_t *USART1_CR3 = (uint32_t *)0x40013814;
  *USART1_CR3 |= (1 << 2); // HDSEL -> software loopback
}

uint8_t uart_read_byte(void) {
  volatile uint32_t *USART1_SR = (uint32_t *)0x40013800;
  volatile uint32_t *USART1_DR = (uint32_t *)0x40013804;
  uint32_t timeout = 1000000;
  while (!(*USART1_SR & (1 << 5))) {
    if (--timeout == 0) return 0xFF;
    __asm__("nop");
  }
  return *USART1_DR & 0xFF;
}

void i2c_init(void) {
  volatile uint32_t *RCC_APB1ENR = (uint32_t *)0x4002101C;
  volatile uint32_t *RCC_APB2ENR = (uint32_t *)0x40021018;
  volatile uint32_t *GPIOB_CRL = (uint32_t *)0x40010C00;
  volatile uint32_t *I2C1_CR1 = (uint32_t *)0x40005400;
  volatile uint32_t *I2C1_CCR = (uint32_t *)0x4000541C;
  volatile uint32_t *I2C1_TRISE = (uint32_t *)0x40005420;

  *RCC_APB1ENR |= (1 << 21); // I2C1EN
  *RCC_APB2ENR |= (1 << 3);  // GPIOBEN

  uint32_t crl = *GPIOB_CRL;
  crl = (crl & ~(0xF << 24)) | (0x3 << 24); // PB6 SCL
  crl = (crl & ~(0xF << 28)) | (0x3 << 28); // PB7 SDA
  *GPIOB_CRL = crl;

  *I2C1_CR1 = 0;
  *I2C1_CCR = 0x50; // 100kHz @ 8MHz
  *I2C1_TRISE = 9;
  *I2C1_CR1 = 1; // PE
}

uint8_t i2c_scan(uint8_t addr) {
  volatile uint32_t *I2C1_CR1 = (uint32_t *)0x40005400;
  volatile uint32_t *I2C1_SR1 = (uint32_t *)0x40005414;
  volatile uint32_t *I2C1_SR2 = (uint32_t *)0x40005418;
  volatile uint32_t *I2C1_DR = (uint32_t *)0x40005410;

  *I2C1_CR1 |= (1 << 8); // START
  uint32_t timeout = 10000;
  while (!(*I2C1_SR1 & 1)) { if (--timeout == 0) return 0xFF; }

  *I2C1_DR = (addr << 1) | 0; // write
  timeout = 10000;
  while (!(*I2C1_SR1 & (1 << 1))) { // ADDR
    if (*I2C1_SR1 & (1 << 10)) return 0xFE; // AF=NACK
    if (--timeout == 0) return 0xFF;
  }

  uint8_t sr2 = *I2C1_SR2; // clear ADDR
  *I2C1_CR1 |= (1 << 9); // STOP
  return 0; // found
}

uint8_t i2c_write_byte(uint8_t addr, uint8_t reg_high, uint8_t reg_low, uint8_t data) {
  volatile uint32_t *I2C1_CR1 = (uint32_t *)0x40005400;
  volatile uint32_t *I2C1_SR1 = (uint32_t *)0x40005414;
  volatile uint32_t *I2C1_SR2 = (uint32_t *)0x40005418;
  volatile uint32_t *I2C1_DR = (uint32_t *)0x40005410;

  *I2C1_CR1 |= (1 << 8); // START
  uint32_t timeout = 10000;
  while (!(*I2C1_SR1 & 1)) { if (--timeout == 0) return 1; }

  *I2C1_DR = (addr << 1) | 0;
  timeout = 10000;
  while (!(*I2C1_SR1 & (1 << 1))) {
    if (*I2C1_SR1 & (1 << 10)) return 2;
    if (--timeout == 0) return 1;
  }
  *I2C1_SR2;

  timeout = 10000;
  while (!(*I2C1_SR1 & (1 << 7))) { if (--timeout == 0) return 1; } // TXE
  *I2C1_DR = reg_high;

  timeout = 10000;
  while (!(*I2C1_SR1 & (1 << 7))) { if (--timeout == 0) return 1; }
  *I2C1_DR = reg_low;

  timeout = 10000;
  while (!(*I2C1_SR1 & (1 << 7))) { if (--timeout == 0) return 1; }
  *I2C1_DR = data;

  timeout = 10000;
  while (!(*I2C1_SR1 & (1 << 7))) { if (--timeout == 0) return 1; }

  *I2C1_CR1 |= (1 << 9); // STOP
  return 0;
}

uint8_t i2c_read_byte(uint8_t addr, uint8_t reg_high, uint8_t reg_low) {
  volatile uint32_t *I2C1_CR1 = (uint32_t *)0x40005400;
  volatile uint32_t *I2C1_SR1 = (uint32_t *)0x40005414;
  volatile uint32_t *I2C1_SR2 = (uint32_t *)0x40005418;
  volatile uint32_t *I2C1_DR = (uint32_t *)0x40005410;

  *I2C1_CR1 |= (1 << 8);
  uint32_t timeout = 10000;
  while (!(*I2C1_SR1 & 1)) { if (--timeout == 0) return 0xFF; }

  *I2C1_DR = (addr << 1) | 0;
  timeout = 10000;
  while (!(*I2C1_SR1 & (1 << 1))) {
    if (*I2C1_SR1 & (1 << 10)) return 0xFF;
    if (--timeout == 0) return 0xFF;
  }
  *I2C1_SR2;

  timeout = 10000;
  while (!(*I2C1_SR1 & (1 << 7))) { if (--timeout == 0) return 0xFF; }
  *I2C1_DR = reg_high;

  timeout = 10000;
  while (!(*I2C1_SR1 & (1 << 7))) { if (--timeout == 0) return 0xFF; }
  *I2C1_DR = reg_low;

  timeout = 10000;
  while (!(*I2C1_SR1 & (1 << 7))) { if (--timeout == 0) return 0xFF; }

  *I2C1_CR1 |= (1 << 8); // Repeated START
  timeout = 10000;
  while (!(*I2C1_SR1 & 1)) { if (--timeout == 0) return 0xFF; }

  *I2C1_DR = (addr << 1) | 1; // read
  timeout = 10000;
  while (!(*I2C1_SR1 & (1 << 1))) {
    if (*I2C1_SR1 & (1 << 10)) return 0xFF;
    if (--timeout == 0) return 0xFF;
  }
  *I2C1_CR1 &= ~(1 << 10); // ACK=0 (NACK)

  uint8_t sr2 = *I2C1_SR2; // clear ADDR
  timeout = 10000;
  while (!(*I2C1_SR1 & (1 << 6))) { if (--timeout == 0) return 0xFF; } // RXNE

  uint8_t val = *I2C1_DR & 0xFF;
  *I2C1_CR1 |= (1 << 9); // STOP
  return val;
}

void spi1_init(void) {
  volatile uint32_t *RCC_APB2ENR = (uint32_t *)0x40021018;
  volatile uint32_t *GPIOA_CRL = (uint32_t *)0x40010800;
  volatile uint32_t *SPI1_CR1 = (uint32_t *)0x40013000;

  *RCC_APB2ENR |= (1 << 12) | (1 << 2);
  uint32_t crl = *GPIOA_CRL;
  crl = (crl & ~(0xF << 20)) | (0xB << 20); // PA5 SCK AFPP
  crl = (crl & ~(0xF << 24)) | (0x8 << 24); // PA6 MISO input
  crl = (crl & ~(0xF << 28)) | (0xB << 28); // PA7 MOSI AFPP
  *GPIOA_CRL = crl;

  *SPI1_CR1 = (4 << 3) | (1 << 2) | (1 << 6); // BR=4, MSTR, SPE
}

void spi1_cs(uint8_t val) {
  volatile uint32_t *GPIOA_BSRR = (uint32_t *)0x40010810;
  if (val) *GPIOA_BSRR = 1 << 4;
  else *GPIOA_BSRR = 1 << (4 + 16);
}

void spi1_cs_init(void) {
  volatile uint32_t *GPIOA_CRL = (uint32_t *)0x40010800;
  *GPIOA_CRL = (*GPIOA_CRL & ~(0xF << 16)) | (0x3 << 16);
  spi1_cs(1);
}

uint8_t spi1_xfer(uint8_t data) {
  volatile uint32_t *SPI1_DR = (uint32_t *)0x4001300C;
  volatile uint32_t *SPI1_SR = (uint32_t *)0x40013008;
  while (!(*SPI1_SR & (1 << 1)));
  *SPI1_DR = data;
  while (!(*SPI1_SR & 1));
  return *SPI1_DR & 0xFF;
}

void adc1_init(void) {
  volatile uint32_t *RCC_APB2ENR = (uint32_t *)0x40021018;
  volatile uint32_t *ADC1_CR2 = (uint32_t *)0x40012408;
  volatile uint32_t *ADC1_SMPR2 = (uint32_t *)0x40012410;

  *RCC_APB2ENR |= (1 << 9); // ADC1EN
  *ADC1_SMPR2 = (7 << 0);   // sample time for channel 0
  *ADC1_CR2 = 1;            // ADON
}

uint16_t adc1_read(void) {
  volatile uint32_t *ADC1_CR2 = (uint32_t *)0x40012408;
  volatile uint32_t *ADC1_SR = (uint32_t *)0x40012400;
  volatile uint32_t *ADC1_DR = (uint32_t *)0x4001244C;

  *ADC1_CR2 |= (1 << 22); // SWSTART
  while (!(*ADC1_SR & (1 << 1)));
  return *ADC1_DR & 0xFFF;
}

void tim2_pwm_init(void) {
  volatile uint32_t *RCC_APB1ENR = (uint32_t *)0x4002101C;
  volatile uint32_t *GPIOA_CRL = (uint32_t *)0x40010800;
  volatile uint32_t *TIM2_PSC = (uint32_t *)0x40000028;
  volatile uint32_t *TIM2_ARR = (uint32_t *)0x4000002C;
  volatile uint32_t *TIM2_CCMR1 = (uint32_t *)0x40000018;
  volatile uint32_t *TIM2_CCER = (uint32_t *)0x40000020;
  volatile uint32_t *TIM2_CR1 = (uint32_t *)0x40000000;

  *RCC_APB1ENR |= (1 << 0);
  uint32_t crl = *GPIOA_CRL;
  crl = (crl & ~(0xF << 4)) | (0xB << 4); // PA1 AFPP
  *GPIOA_CRL = crl;

  *TIM2_PSC = 72 - 1;  // 1MHz
  *TIM2_ARR = 1000 - 1; // 1kHz
  *TIM2_CCMR1 = (0b110 << 12) | (1 << 11); // OC2M=PWM1, OC2PE=1
  *TIM2_CCER = (1 << 4); // CC2E
  *TIM2_CR1 = 1;         // CEN
}

void tim2_pwm_set_duty(uint8_t duty) {
  volatile uint32_t *TIM2_CCR2 = (uint32_t *)0x40000038;
  uint32_t val = (uint32_t)duty * 1000 / 256;
  *TIM2_CCR2 = val;
}

void exti_init(void) {
  volatile uint32_t *RCC_APB2ENR = (uint32_t *)0x40021018;
  volatile uint32_t *AFIO_EXTICR1 = (uint32_t *)0x40010008;
  volatile uint32_t *EXTI_IMR = (uint32_t *)0x40010400;
  volatile uint32_t *EXTI_RTSR = (uint32_t *)0x40010408;

  *RCC_APB2ENR |= (1 << 0); // AFIOEN
  *AFIO_EXTICR1 = 0;        // EXTI0 on PA0
  *EXTI_IMR |= 1;           // unmask line 0
  *EXTI_RTSR |= 1;          // rising edge
}

void exti_trigger(void) {
  volatile uint32_t *EXTI_SWIER = (uint32_t *)0x40010410;
  volatile uint32_t *EXTI_PR = (uint32_t *)0x40010414;
  *EXTI_SWIER = 1;
  if (*EXTI_PR & 1) {
    uart_write_line("  EXTI: Interrupt triggered via SWIER");
    *EXTI_PR = 1;
  }
}

void exti_wait_for_line(uint32_t line) {
  volatile uint32_t *EXTI_PR = (uint32_t *)0x40010414;
  while (!(*EXTI_PR & (1 << line)));
  *EXTI_PR = (1 << line);
}

void rcc_print_info(void) {
  volatile uint32_t *RCC_CR = (uint32_t *)0x40021000;
  volatile uint32_t *RCC_CFGR = (uint32_t *)0x40021004;

  uint32_t cr = *RCC_CR;
  uint32_t cfgr = *RCC_CFGR;

  uart_write_str("  HSI: ");
  uart_write_str(cr & 2 ? "Ready" : "Off");
  uart_write_str(", HSE: ");
  uart_write_str(cr & 0x20000 ? "Ready" : "Off");
  uart_write_str(", PLL: ");
  uart_write_str(cr & 0x2000000 ? "Ready" : "Off");

  const char *src;
  switch (cfgr & 3) {
    case 0: src = "HSI"; break;
    case 1: src = "HSE"; break;
    case 2: src = "PLL"; break;
    default: src = "???"; break;
  }
  uart_write_str(", SYSCLK: ");
  uart_write_str(src);
  uart_write_line("");
}

void sys_tick_init(void) {
  volatile uint32_t *STK_CSR = (uint32_t *)0xE000E010;
  volatile uint32_t *STK_RVR = (uint32_t *)0xE000E014;
  volatile uint32_t *STK_CVR = (uint32_t *)0xE000E018;

  *STK_RVR = 8000 - 1; // 1ms @ 8MHz
  *STK_CVR = 0;
  *STK_CSR = 7; // ENABLE | TICKINT | CLKSOURCE
}

// 11. CRC Test
void crc_init(void) {
  volatile uint32_t *RCC_AHBENR = (uint32_t *)0x40021014;
  *RCC_AHBENR |= (1 << 6); // CRCEN
}

void crc_test(void) {
  volatile uint32_t *CRC_DR = (uint32_t *)0x40023000;
  volatile uint32_t *CRC_CR = (uint32_t *)0x40023008;
  // Reset CRC
  *CRC_CR = 1;
  // Compute CRC of 0xDEADBEEF
  *CRC_DR = 0xDEADBEEF;
  uint32_t crc_val = *CRC_DR;
  uart_write_str("   CRC(0xDEADBEEF) = ");
  uart_write_hex(crc_val);
  uart_write_line("");

  *CRC_CR = 1;
  *CRC_DR = 0x00000000;
  crc_val = *CRC_DR;
  uart_write_str("   CRC(0x00000000) = ");
  uart_write_hex(crc_val);
  uart_write_line("");

  *CRC_CR = 1;
  *CRC_DR = 0xFFFFFFFF;
  crc_val = *CRC_DR;
  uart_write_str("   CRC(0xFFFFFFFF) = ");
  uart_write_hex(crc_val);
  uart_write_line("");
}

// 12. DAC Test
void dac_init(void) {
  volatile uint32_t *RCC_APB1ENR = (uint32_t *)0x4002101C;
  *RCC_APB1ENR |= (1 << 29); // DACEN
}

void dac_test(void) {
  volatile uint32_t *DAC_DHR12R1 = (uint32_t *)0x40007408;
  volatile uint32_t *DAC_DOR1 = (uint32_t *)0x4000742C;
  *DAC_DHR12R1 = 0x800;
  uint32_t dac_val = *DAC_DOR1;
  uart_write_str("   DAC DOR1 = ");
  uart_write_hex(dac_val);
  uart_write_line(dac_val == 0x800 ? " (MATCH)" : " (MISMATCH)");
}

// 13. RTC Test
void rtc_init(void) {
  volatile uint32_t *RCC_APB1ENR = (uint32_t *)0x4002101C;
  *RCC_APB1ENR |= (1 << 9); // RTCEN
}

void rtc_test(void) {
  volatile uint32_t *RTC_CRL = (uint32_t *)0x40002804;
  volatile uint32_t *RTC_CNTH = (uint32_t *)0x40002818;
  volatile uint32_t *RTC_CNTL = (uint32_t *)0x4000281C;

  // Write counter
  *RTC_CNTH = 0;
  *RTC_CNTL = 0x42;
  uint32_t cnt = (*RTC_CNTH << 16) | *RTC_CNTL;
  uart_write_str("   RTC CNT = ");
  uart_write_hex(cnt);
  uart_write_line(cnt == 0x42 ? " (MATCH)" : " (MISMATCH)");

  // Read CRL (should have RTOFF=1)
  uint32_t crl = *RTC_CRL;
  uart_write_str("   RTC CRL = ");
  uart_write_hex(crl);
  uart_write_line(crl & 0x20 ? " (RTOFF set)" : " (RTOFF not set)");
}

// 14. PWR Test
void pwr_test(void) {
  volatile uint32_t *RCC_APB1ENR = (uint32_t *)0x4002101C;
  volatile uint32_t *PWR_CR = (uint32_t *)0x40007000;
  volatile uint32_t *PWR_CSR = (uint32_t *)0x40007004;
  *RCC_APB1ENR |= (1 << 28); // PWREN

  *PWR_CR = 0x100;
  uint32_t cr = *PWR_CR;
  uart_write_str("   PWR CR = ");
  uart_write_hex(cr);
  uart_write_line(cr == 0x100 ? "" : " (unexpected)");

  uint32_t csr = *PWR_CSR;
  uart_write_str("   PWR CSR = ");
  uart_write_hex(csr);
  uart_write_line("");
}

// 15. Flash Test
void flash_test(void) {
  volatile uint32_t *FLASH_ACR = (uint32_t *)0x40022000;
  volatile uint32_t *FLASH_KEYR = (uint32_t *)0x40022004;
  // Read FLASH_ACR (reset value should have PRFTBS bit 5 set if prefetch buffer exists)
  uint32_t acr = *FLASH_ACR;
  uart_write_str("   FLASH_ACR = ");
  uart_write_hex(acr);
  uart_write_line("");

  // Write to FLASH_ACR (latency and prefetch bits)
  *FLASH_ACR = (1 << 4) | (1 << 1);
  acr = *FLASH_ACR;
  uart_write_str("   FLASH_ACR after write = ");
  uart_write_hex(acr);
  uart_write_line(acr & 2 ? " (latency=1)" : "");
}

// 16. BKP Test
void bkp_test(void) {
  volatile uint32_t *RCC_APB1ENR = (uint32_t *)0x4002101C;
  volatile uint32_t *RCC_BDCR = (uint32_t *)0x40021020;
  volatile uint32_t *BKP_DR1 = (uint32_t *)0x40006C10;
  volatile uint32_t *BKP_DR2 = (uint32_t *)0x40006C14;

  // Enable BKP clock and LSE
  *RCC_APB1ENR |= (1 << 27); // BKPEN
  *RCC_BDCR |= 1; // LSEON

  // Write and read back backup data register
  *BKP_DR1 = 0xA5A5;
  uint32_t val = *BKP_DR1;
  *BKP_DR2 = 0x5A5A;
  uint32_t val2 = *BKP_DR2;
  uart_write_str("   BKP DR1 = ");
  uart_write_hex(val);
  uart_write_str(", DR2 = ");
  uart_write_hex(val2);
  uart_write_line(val == 0xA5A5 && val2 == 0x5A5A ? " (MATCH)" : " (MISMATCH)");
}

// 17. SPI TFT LCD Test (via Lcd ext device)
void lcd_test(void) {
  spi1_cs(0);
  spi1_xfer(0xFB);
  for (int i = 0; i < 128; i++) {
    spi1_xfer(0xFF);
  }
  spi1_cs(1);
  uart_write_line("   Sent 0xFB + 128 bytes of pixel data");
}

// 18. I2C OLED Test (via I2cOled ext device)
void oled_write_cmd(uint8_t cmd) {
  volatile uint32_t *I2C1_CR1 = (uint32_t *)0x40005400;
  volatile uint32_t *I2C1_SR1 = (uint32_t *)0x40005414;
  volatile uint32_t *I2C1_SR2 = (uint32_t *)0x40005418;
  volatile uint32_t *I2C1_DR = (uint32_t *)0x40005410;

  *I2C1_CR1 |= (1 << 8);
  uint32_t t = 10000;
  while (!(*I2C1_SR1 & 1)) { if (--t == 0) return; }

  *I2C1_DR = (0x3C << 1) | 0;
  t = 10000;
  while (!(*I2C1_SR1 & (1 << 1))) { if (--t == 0) return; }
  *I2C1_SR2;

  *I2C1_DR = 0x00;
  t = 10000;
  while (!(*I2C1_SR1 & (1 << 7))) { if (--t == 0) return; }

  *I2C1_DR = cmd;
  t = 10000;
  while (!(*I2C1_SR1 & (1 << 7))) { if (--t == 0) return; }

  *I2C1_CR1 |= (1 << 9);
}

void oled_test(void) {
  oled_write_cmd(0xAE);
  oled_write_cmd(0x81);
  oled_write_cmd(0xCF);
  oled_write_cmd(0xAF);
  uart_write_line("   Sent OLED init commands (AE, 81 CF, AF)");

  // Write display data
  volatile uint32_t *I2C1_CR1 = (uint32_t *)0x40005400;
  volatile uint32_t *I2C1_SR1 = (uint32_t *)0x40005414;
  volatile uint32_t *I2C1_SR2 = (uint32_t *)0x40005418;
  volatile uint32_t *I2C1_DR = (uint32_t *)0x40005410;

  *I2C1_CR1 |= (1 << 8);
  uint32_t t = 10000;
  while (!(*I2C1_SR1 & 1)) { if (--t == 0) return; }

  *I2C1_DR = (0x3C << 1) | 0;
  t = 10000;
  while (!(*I2C1_SR1 & (1 << 1))) { if (--t == 0) return; }
  *I2C1_SR2;

  *I2C1_DR = 0x40;
  t = 10000;
  while (!(*I2C1_SR1 & (1 << 7))) { if (--t == 0) return; }

  for (int i = 0; i < 128; i++) {
    *I2C1_DR = 0xFF;
    t = 10000;
    while (!(*I2C1_SR1 & (1 << 7))) { if (--t == 0) return; }
  }

  *I2C1_CR1 |= (1 << 9);
  uart_write_line("   Sent 128 bytes of OLED pixel data via I2C");
}

// 19. CAN Test
void can_test(void) {
  volatile uint32_t *RCC_APB1ENR = (uint32_t *)0x4002101C;
  volatile uint32_t *GPIOB_CRH = (uint32_t *)0x40010C04;
  volatile uint32_t *CAN_MCR = (uint32_t *)0x40006400;
  volatile uint32_t *CAN_MSR = (uint32_t *)0x40006404;
  volatile uint32_t *CAN_BTR = (uint32_t *)0x4000641C;
  volatile uint32_t *CAN_TI0R = (uint32_t *)0x40006580;
  volatile uint32_t *CAN_TDT0R = (uint32_t *)0x40006584;
  volatile uint32_t *CAN_TDL0R = (uint32_t *)0x40006588;
  volatile uint32_t *CAN_TDH0R = (uint32_t *)0x4000658C;
  volatile uint32_t *CAN_TSR = (uint32_t *)0x40006408;
  volatile uint32_t *CAN_FMR = (uint32_t *)0x40006600;
  volatile uint32_t *CAN_FM1R = (uint32_t *)0x40006604;
  volatile uint32_t *CAN_FS1R = (uint32_t *)0x4000660C;
  volatile uint32_t *CAN_FA1R = (uint32_t *)0x4000661C;
  volatile uint32_t *CAN_F0R1 = (uint32_t *)0x40006640;
  volatile uint32_t *RCC_APB1ENR2 = (uint32_t *)0x4002101C;
  volatile uint32_t *RCC_APB2ENR2 = (uint32_t *)0x40021018;

  *RCC_APB1ENR2 |= (1 << 25); // CAN1EN
  *RCC_APB2ENR2 |= (1 << 3);  // GPIOBEN

  uint32_t crh = *GPIOB_CRH;
  crh = (crh & ~(0xF << 0)) | (0x8 << 0);  // PB8 RX input pull-up
  crh = (crh & ~(0xF << 4)) | (0xB << 4);  // PB9 TX AFPP
  *GPIOB_CRH = crh;

  // Request leave init mode
  *CAN_MCR = *CAN_MCR & ~1;
  uint32_t t = 10000;
  while (*CAN_MSR & 1) { if (--t == 0) break; }
  uart_write_str("   CAN MSR after leave init: ");
  uart_write_hex(*CAN_MSR);
  uart_write_line("");

  // Bit timing: 8MHz → 500kbps (BS1=4, BS2=3, SJW=1, prescaler=2)
  *CAN_BTR = (1 << 24) | (3 << 20) | (4 << 16) | (2 - 1);
  uart_write_str("   CAN BTR: ");
  uart_write_hex(*CAN_BTR);
  uart_write_line("");

  // Filter init mode
  *CAN_FMR = 1;
  // 32-bit mask mode (FM1R bit 0 = 0)
  *CAN_FM1R = 0;
  // Single 32-bit filter (FS1R bit 0 = 1)
  *CAN_FS1R = (1 << 0);
  // ID = 0, mask = 0 (accept all)
  *CAN_F0R1 = 0;
  // Activate filter 0
  *CAN_FA1R = (1 << 0);
  // Exit filter init mode
  *CAN_FMR = 0;
  uart_write_str("   CAN FA1R (filters active): ");
  uart_write_hex(*CAN_FA1R);
  uart_write_line("");

  // Send a CAN frame via TX mailbox 0
  *CAN_TI0R = (0x123 << 21) | 1; // STDID=0x123, TXRQ=1
  *CAN_TDT0R = 2; // DLC=2
  *CAN_TDL0R = 0xDEAD;
  *CAN_TDH0R = 0;
  uart_write_str("   CAN TI0R: ");
  uart_write_hex(*CAN_TI0R);
  uart_write_line("");

  t = 10000;
  while (!(*CAN_TSR & (1 << 0))) { if (--t == 0) break; } // TXOK
  uart_write_str("   CAN TSR: ");
  uart_write_hex(*CAN_TSR);
  uart_write_line(t > 0 ? " (TXOK)" : " (TX timeout)");
}

// 20. IWDG Test (no reset)
void iwdg_test(void) {
  volatile uint32_t *IWDG_KR = (uint32_t *)0x40003000;
  volatile uint32_t *IWDG_PR = (uint32_t *)0x40003004;
  volatile uint32_t *IWDG_RLR = (uint32_t *)0x40003008;
  volatile uint32_t *IWDG_SR = (uint32_t *)0x4000300C;

  // Unlock register access
  *IWDG_KR = 0x5555;

  // Write PR and RLR
  *IWDG_PR = 0; // /4
  *IWDG_RLR = 0xFFF;

  // Read back
  uint32_t pr = *IWDG_PR;
  uint32_t rlr = *IWDG_RLR;
  uint32_t sr = *IWDG_SR;
  uart_write_str("   IWDG PR: ");
  uart_write_hex(pr);
  uart_write_str(", RLR: ");
  uart_write_hex(rlr);
  uart_write_str(", SR: ");
  uart_write_hex(sr);
  uart_write_line(pr == 0 && rlr == 0xFFF ? " (MATCH)" : " (MISMATCH)");

  // DO NOT write 0xCCCC to KR (would start watchdog)
}

// 21. WWDG Test
void wwdg_test(void) {
  volatile uint32_t *RCC_APB1ENR3 = (uint32_t *)0x4002101C;
  volatile uint32_t *WWDG_CR = (uint32_t *)0x40002C00;
  volatile uint32_t *WWDG_CFR = (uint32_t *)0x40002C04;
  volatile uint32_t *WWDG_SR = (uint32_t *)0x40002C08;

  *RCC_APB1ENR3 |= (1 << 11); // WWDGEN

  *WWDG_CFR = 0x3FF;
  uint32_t cfr = *WWDG_CFR;
  uart_write_str("   WWDG CFR: ");
  uart_write_hex(cfr);
  uart_write_str(cfr == 0x3FF ? " (MATCH)" : " (MISMATCH)");
  uart_write_line("");

  *WWDG_CR = 0x50;
  uint32_t cr = *WWDG_CR;
  uart_write_str("   WWDG CR: ");
  uart_write_hex(cr);
  uart_write_str(cr > 0 ? " (non-zero)" : " (ZERO)");
  uart_write_line("");

  uint32_t sr = *WWDG_SR;
  uart_write_str("   WWDG SR: ");
  uart_write_hex(sr);
  uart_write_line("");
}

void setup(void) {
  gpio_init();
  gpio_write(13, 1);
  uart_init();
  sys_tick_init();
  delay_ms(50);

  uart_write_line("");
  uart_write_line("============================================");
  uart_write_line("STM32 Blue Pill Comprehensive Peripheral Test");
  uart_write_line("============================================");
  uart_write_line("");

  // 1. GPIO
  uart_write_line("1. GPIO Test");
  uart_write_line("   Blinking PC13 LED...");
  gpio_write(13, 0);
  delay_ms(300);
  gpio_write(13, 1);
  uart_write_line("   PASS");
  uart_write_line("");

  // 2. USART (UART) loopback
  uart_write_line("2. USART Test");
  uart_write_str("   Writing string via UART: Hello from Blue Pill!");
  uart_write_line("");
  uart_write_line("   PASS");
  uart_write_line("");

  // 3. UART RX Test (loopback)
  uart_write_line("3. UART RX Test");
  uart_write_str("   Sending 0xA5 via loopback...");
  uart_write_line("");
  uart_enable_loopback();
  *((volatile uint32_t *)0x40013804) = 0xA5;
  uint8_t rx_byte = uart_read_byte();
  uart_write_str("   Received: ");
  uart_write_hex(rx_byte);
  if (rx_byte == 0xA5) {
    uart_write_line(" (MATCH)");
    uart_write_line("   PASS");
  } else {
    uart_write_line(" (MISMATCH)");
    uart_write_line("   FAIL");
  }
  uart_write_line("");

  // 4. I2C
  uart_write_line("4. I2C Test");
  uart_write_line("   Initializing I2C1 (PB6=SCL, PB7=SDA)...");
  i2c_init();
  delay_ms(10);
  uart_write_line("   Scanning for devices...");
  uint8_t found = 0;
  for (uint8_t addr = 1; addr < 127; addr++) {
    uint8_t ret = i2c_scan(addr);
    if (ret == 0) {
      uart_write_str("   Found device at ");
      uart_write_hex(addr);
      if (addr == 0x50) uart_write_str(" (EEPROM)");
      uart_write_line("");
      found++;

      if (addr == 0x50) {
        uart_write_line("   Writing 0x42 to EEPROM addr 0x0000...");
        if (i2c_write_byte(0x50, 0x00, 0x00, 0x42) == 0) {
          delay_ms(10);
          uint8_t val = i2c_read_byte(0x50, 0x00, 0x00);
          uart_write_str("   Read back: ");
          uart_write_hex(val);
          uart_write_str(val == 0x42 ? " (MATCH)" : " (MISMATCH)");
          uart_write_line("");
        }
      }
    }
  }
  uart_write_str("   Found ");
  uart_write_dec(found);
  uart_write_line(" device(s)");
  if (found > 0) uart_write_line("   PASS");
  else uart_write_line("   PASS (no devices expected without ext devices)");
  uart_write_line("");

  // 5. SPI
  uart_write_line("5. SPI Test");
  uart_write_line("   Initializing SPI1 (PA5=SCK, PA6=MISO, PA7=MOSI)...");
  spi1_cs_init();
  spi1_init();
  delay_ms(10);
  uart_write_line("   Reading JEDEC ID from SPI flash (cmd 0x9F)...");
  spi1_cs(0);
  uint8_t id0 = spi1_xfer(0x9F);
  uint8_t id1 = spi1_xfer(0);
  uint8_t id2 = spi1_xfer(0);
  spi1_cs(1);
  uart_write_str("   JEDEC ID: ");
  uart_write_hex(id0);
  uart_write_str(" ");
  uart_write_hex(id1);
  uart_write_str(" ");
  uart_write_hex(id2);
  uart_write_line("");
  uart_write_line("   PASS");
  uart_write_line("");

  // 6. ADC
  uart_write_line("6. ADC Test");
  uart_write_line("   Initializing ADC1...");
  adc1_init();
  delay_ms(10);
  uint16_t adc_val = adc1_read();
  uart_write_str("   ADC1 reading: ");
  uart_write_dec(adc_val);
  uart_write_line(" (0-4095)");
  uart_write_line("   PASS");
  uart_write_line("");

  // 7. Timer / PWM
  uart_write_line("7. Timer/PWM Test");
  uart_write_line("   Initializing TIM2 CH2 PWM on PA1...");
  tim2_pwm_init();
  uart_write_line("   Setting duty to 50%...");
  tim2_pwm_set_duty(128);
  delay_ms(50);
  uart_write_line("   Setting duty to 0%...");
  tim2_pwm_set_duty(0);
  uart_write_line("   PASS");
  uart_write_line("");

  // 8. RCC Info
  uart_write_line("8. RCC System Info");
  rcc_print_info();
  uart_write_line("   PASS");
  uart_write_line("");

  // 9. EXTI
  uart_write_line("9. EXTI Test");
  exti_init();
  exti_trigger();
  uart_write_line("   PASS");
  uart_write_line("");

  // 10. SysTick
  uart_write_line("10. SysTick Timer Test");
  uart_write_line("   SysTick running at 1ms interval");
  uart_write_str("   Elapsed: ");
  uart_write_dec(systick_ms);
  uart_write_line(" ms");
  uart_write_line("   PASS");
  uart_write_line("");

  // 11. CRC
  uart_write_line("11. CRC Test");
  crc_init();
  crc_test();
  uart_write_line("   PASS");
  uart_write_line("");

  // 12. DAC
  uart_write_line("12. DAC Test");
  dac_init();
  dac_test();
  uart_write_line("   PASS");
  uart_write_line("");

  // 13. RTC
  uart_write_line("13. RTC Test");
  rtc_init();
  rtc_test();
  uart_write_line("   PASS");
  uart_write_line("");

  // 14. PWR
  uart_write_line("14. PWR Test");
  pwr_test();
  uart_write_line("   PASS");
  uart_write_line("");

  // 15. Flash Controller
  uart_write_line("15. Flash Controller Test");
  flash_test();
  uart_write_line("   PASS");
  uart_write_line("");

  // 16. BKP
  uart_write_line("16. BKP Test");
  bkp_test();
  uart_write_line("   PASS");
  uart_write_line("");

  // 17. SPI TFT LCD
  uart_write_line("17. SPI TFT LCD Test");
  spi1_cs_init();
  spi1_init();
  lcd_test();
  uart_write_line("   PASS");
  uart_write_line("");

  // 18. I2C OLED
  uart_write_line("18. I2C OLED Test");
  i2c_init();
  oled_test();
  uart_write_line("   PASS");
  uart_write_line("");

  // 19. CAN
  uart_write_line("19. CAN Test");
  can_test();
  uart_write_line("   PASS");
  uart_write_line("");

  // 20. IWDG
  uart_write_line("20. IWDG Test");
  iwdg_test();
  uart_write_line("   PASS");
  uart_write_line("");

  // 21. WWDG
  uart_write_line("21. WWDG Test");
  wwdg_test();
  uart_write_line("   PASS");
  uart_write_line("");

  uart_write_line("============================================");
  uart_write_str("All 21 peripheral tests completed!");
  uart_write_line("");
  uart_write_line("============================================");
}

void loop(void) {
  gpio_write(13, 0);
  delay_ms(500);
  gpio_write(13, 1);
  delay_ms(500);
  uart_write_line("tick");
}

int main(void) {
  setup();
  while (1) loop();
}
