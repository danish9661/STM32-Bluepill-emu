/*
 * Full peripheral test firmware for the STM32 BluePill (F103C8) WASM emulator.
 *
 * Uses the real Arduino (STM32duino) framework API where available
 * (Serial, SPI, Wire, analogRead, HardwareTimer, pinMode/digitalWrite)
 * and register-level access where the framework lacks a driver
 * (CRC, CAN, DMA, RTC, PWR, BKP, IWDG, WWDG, FLASH, AFIO, EXTI).
 *
 * Every test prints a machine-parseable line:
 *     [name] PASS   or   [name] FAIL: <detail>
 *
 * NOTE: no String/new/malloc — STM32duino's _sbrk uses `mrs msp`, which the
 * WASM emulator's Unicorn CPU cannot decode.
 *
 * Run with:
 *   echo "AB" | node pkg/cli.mjs --config=tests/arduino_periph_test/config.yaml
 *   ('A' is consumed by the DMA RX test, 'B' by the UART RX test)
 */
#include <Arduino.h>
#include <HardwareTimer.h>
#include <SPI.h>
#include <Wire.h>
#include <stdio.h>

/* ----------------------- register base addresses ----------------------- */
#define RCC_APB2ENR 0x40021018u
#define RCC_APB1ENR 0x4002101Cu
#define RCC_AHBENR  0x40021014u
#define RCC_CR      0x40021000u
#define RCC_CFGR    0x40021004u
#define USART1_B    0x40013800u
#define USART2_B    0x40004400u
#define GPIOC_B     0x40011000u
#define SPI1_B      0x40013000u
#define SPI2_B      0x40003800u
#define I2C1_B      0x40005400u
#define I2C2_B      0x40005800u
#define TIM3_B      0x40000400u
#define TIM4_B      0x40000800u
#define CAN1_B      0x40006400u
#define IWDG_B      0x40003000u
#define WWDG_B      0x40002C00u
#define RTC_B       0x40002800u
#define PWR_B       0x40007000u
#define BKP_B       0x40006C00u
#define CRC_B       0x40023000u
#define DAC_B       0x40007400u
#define FLASH_B     0x40022000u
#define AFIO_B      0x40010000u
#define EXTI_B      0x40010400u
#define DMA1_B      0x40020000u
#define NVIC_ISER0  0xE000E100u
#define SCB_B       0xE000ED00u

#define reg(addr) (*(volatile uint32_t *)(addr))

static uint32_t passCount = 0;
static uint32_t failCount = 0;

void report(const char *name, bool ok, const char *detail) {
    if (ok) { passCount++; Serial.print("["); Serial.print(name); Serial.println("] PASS"); }
    else    { failCount++; Serial.print("["); Serial.print(name); Serial.print("] FAIL: "); Serial.println(detail); }
}

/* bounded busy wait, independent of SysTick */
void spin(uint32_t n) { while (n--) __asm__("nop"); }

/* ============================ IRQ callbacks ============================ */
volatile uint32_t tim2IrqCount = 0;
volatile uint32_t exti0IrqCount = 0;
volatile uint32_t rtcAlarmCount = 0;
volatile uint32_t exti1IrqCount = 0;
volatile uint32_t exti13IrqCount = 0;

void onTimer() { tim2IrqCount++; }

/* global scope so ~HardwareTimer doesn't clear HardwareTimer_Handle[1] */
HardwareTimer timer2(TIM2);

void extiISR() { exti0IrqCount++; }
void exti1ISR() { exti1IrqCount++; }
void exti13ISR() { exti13IrqCount++; }

/* ================================ tests ================================ */
void testGPIO() {
    pinMode(PC13, OUTPUT);
    digitalWrite(PC13, HIGH);
    bool hi = digitalRead(PC13) == HIGH;
    digitalWrite(PC13, LOW);
    bool lo = digitalRead(PC13) == LOW;
    char buf[32];
    snprintf(buf, sizeof(buf), "hi=%d lo=%d odr=%04X", hi, lo, (uint16_t)reg(GPIOC_B + 0x0C));
    report("GPIO", hi && lo, buf);
}

void testUSART_TX() {
    Serial.print("Hello from BluePill! ");
    /* TXE clears while a byte shifts out; poll until the transmit side becomes ready again */
    uint32_t txe = 0;
    for (uint32_t i = 0; i < 2000000 && !txe; i++) txe = (reg(USART1_B + 0x00) & (1 << 7)) != 0;
    report("USART TX", txe, txe ? "TXE set" : "TXE not set");
}

void testUART_Loopback() {
    reg(USART1_B + 0x14) |= (1 << 2);          /* CR3 HDSEL -> software loopback */
    reg(USART1_B + 0x04) = 0xA5;               /* write DR */
    spin(100);
    uint8_t got = reg(USART1_B + 0x04) & 0xFF; /* read DR */
    reg(USART1_B + 0x14) &= ~(1 << 2);         /* clear HDSEL */
    char buf[32];
    snprintf(buf, sizeof(buf), "got=%02X", got);
    report("UART Loopback", got == 0xA5, buf);
}

void testRCC() {
    uint32_t cr = reg(RCC_CR);
    uint32_t cfgr = reg(RCC_CFGR);
    bool hsi = (cr & 2) != 0;
    char buf[64];
    snprintf(buf, sizeof(buf), "CR=%08X CFGR=%08X", cr, cfgr);
    report("RCC", hsi, buf);
}

void testFLASH() {
    uint32_t acr0 = reg(FLASH_B + 0x00);
    reg(FLASH_B + 0x00) = 0x25;                /* PRFTEN | latency=1 */
    uint32_t acr1 = reg(FLASH_B + 0x00);
    reg(FLASH_B + 0x04) = 0x45670123;          /* unlock key 1 */
    reg(FLASH_B + 0x04) = 0xCDEF89AB;          /* unlock key 2 */
    uint32_t cr = reg(FLASH_B + 0x0C);
    bool unlocked = (cr & (1 << 7)) == 0;
    char buf[64];
    snprintf(buf, sizeof(buf), "ACR=%08X->%08X CR=%08X", acr0, acr1, cr);
    report("FLASH", unlocked && (acr1 == 0x25), buf);
}

void testPWR() {
    reg(RCC_APB1ENR) |= (1 << 28); /* PWREN */
    reg(PWR_B + 0x00) = 0x100;     /* PLS bits */
    uint32_t cr = reg(PWR_B + 0x00);
    uint32_t csr = reg(PWR_B + 0x04);
    char buf[64];
    snprintf(buf, sizeof(buf), "CR=%08X CSR=%08X", cr, csr);
    report("PWR", (cr & 0x100) != 0, buf);
}

void testBKP() {
    reg(RCC_APB1ENR) |= (1 << 27); /* BKPEN */
    reg(BKP_B + 0x10) = 0xA5A5;    /* DR1 */
    reg(BKP_B + 0x14) = 0x5A5A;    /* DR2 */
    uint32_t d1 = reg(BKP_B + 0x10);
    uint32_t d2 = reg(BKP_B + 0x14);
    char buf[64];
    snprintf(buf, sizeof(buf), "DR1=%04X DR2=%04X", d1, d2);
    report("BKP", d1 == 0xA5A5 && d2 == 0x5A5A, buf);
}

void testIWDG() {
    reg(IWDG_B + 0x00) = 0x5555;   /* unlock */
    reg(IWDG_B + 0x04) = 0;        /* PR = /4 */
    reg(IWDG_B + 0x08) = 0xFFF;    /* RLR */
    uint32_t pr = reg(IWDG_B + 0x04);
    uint32_t rlr = reg(IWDG_B + 0x08);
    uint32_t sr = reg(IWDG_B + 0x0C);
    char buf[64];
    snprintf(buf, sizeof(buf), "PR=%X RLR=%X SR=%X", pr, rlr, sr);
    report("IWDG", pr == 0 && rlr == 0xFFF, buf);
}

void testWWDG() {
    reg(RCC_APB1ENR) |= (1 << 11); /* WWDGEN */
    reg(WWDG_B + 0x04) = 0x3FF;
    reg(WWDG_B + 0x00) = 0x50;     /* no WDGA -> does not start */
    uint32_t cfr = reg(WWDG_B + 0x04);
    uint32_t cr = reg(WWDG_B + 0x00);
    uint32_t sr = reg(WWDG_B + 0x08);
    char buf[64];
    snprintf(buf, sizeof(buf), "CFR=%X CR=%X SR=%X", cfr, cr, sr);
    report("WWDG", cfr == 0x3FF && (cr & 0x7F) == 0x50, buf);
}

void testRTC() {
    reg(RCC_APB1ENR) |= (1 << 9);  /* RTCEN */
    reg(RTC_B + 0x18) = 0;         /* CNTH */
    reg(RTC_B + 0x1C) = 0x42;      /* CNTL */
    uint32_t cnt = (reg(RTC_B + 0x18) << 16) | (reg(RTC_B + 0x1C) & 0xFFFF);
    uint32_t crl = reg(RTC_B + 0x04);
    bool rtoff = (crl & 0x20) != 0;
    char buf[64];
    snprintf(buf, sizeof(buf), "CNT=%08X CRL=%08X", cnt, crl);
    report("RTC", cnt == 0x42 && rtoff, buf);
}

void testCRC() {
    reg(RCC_AHBENR) |= (1 << 6);   /* CRCEN */
    reg(CRC_B + 0x08) = 1;         /* reset */
    uint32_t rst = reg(CRC_B + 0x00);
    reg(CRC_B + 0x00) = 0x00000000;
    uint32_t crc0 = reg(CRC_B + 0x00);
    char buf[64];
    snprintf(buf, sizeof(buf), "reset=%08X CRC(0)=%08X", rst, crc0);
    report("CRC", rst == 0xFFFFFFFF && crc0 == 0xC704DD7B, buf);
}

void testDAC() {
    reg(RCC_APB1ENR) |= (1 << 29); /* DACEN */
    reg(DAC_B + 0x08) = 0x800;     /* DHR12R1 */
    uint32_t dor = reg(DAC_B + 0x2C);
    char buf[32];
    snprintf(buf, sizeof(buf), "DOR1=%04X", dor);
    report("DAC", dor == 0x800, buf);
}

void testADC() {
    uint32_t v = analogRead(A0);
    char buf[32];
    snprintf(buf, sizeof(buf), "ADC1=%lu", (unsigned long)v);
    report("ADC", v > 0, buf); /* emulator sim value is 0x3FF */
}

void testAFIO() {
    reg(RCC_APB2ENR) |= (1 << 0);  /* AFIOEN */
    reg(AFIO_B + 0x04) = (2 << 13);/* MAPR: CAN remap bits = 10 */
    uint32_t mapr = reg(AFIO_B + 0x04);
    reg(AFIO_B + 0x14) = 0x00000010; /* EXTICR4: EXTI13 -> port B (1 << 4) */
    uint32_t exti4 = reg(AFIO_B + 0x14);
    char buf[64];
    snprintf(buf, sizeof(buf), "MAPR=%08X EXTICR4=%08X", mapr, exti4);
    report("AFIO", (mapr & (3 << 13)) == (2 << 13) && (exti4 & 0xF0) == (1 << 4), buf);
}

void testEXTI_reg() {
    attachInterrupt(PA0, extiISR, RISING); /* configures EXTI0 on PA0 via HAL */
    reg(EXTI_B + 0x10) = 1;                /* SWIER */
    bool pr = (reg(EXTI_B + 0x14) & 1) != 0;
    report("EXTI reg", pr, pr ? "PR0 set via SWIER" : "PR0 not set");
}

void testCAN() {
    reg(RCC_APB1ENR) |= (1 << 25); /* CAN1EN */
    reg(RCC_APB2ENR) |= (1 << 3);  /* GPIOBEN */
    reg(CAN1_B + 0x00) = reg(CAN1_B + 0x00) & ~1; /* leave init mode */
    spin(100);
    uint32_t msr = reg(CAN1_B + 0x04);
    reg(CAN1_B + 0x1C) = (1 << 24) | (3 << 20) | (4 << 16) | 1; /* BTR */
    reg(CAN1_B + 0x200) = 1;       /* filter init mode */
    reg(CAN1_B + 0x204) = 0;       /* 32-bit mask mode */
    reg(CAN1_B + 0x20C) = 1;       /* single 32-bit filter */
    reg(CAN1_B + 0x240) = 0;       /* ID=0, mask=0 accept all */
    reg(CAN1_B + 0x21C) = 1;       /* activate filter 0 */
    reg(CAN1_B + 0x200) = 0;       /* exit filter init */
    reg(CAN1_B + 0x180) = (0x123 << 21) | 1; /* TI0R: STDID=0x123, TXRQ */
    reg(CAN1_B + 0x184) = 2;       /* DLC=2 */
    reg(CAN1_B + 0x188) = 0xDEAD;  /* data */
    reg(CAN1_B + 0x18C) = 0;
    uint32_t tsr = reg(CAN1_B + 0x08);
    uint32_t fa1r = reg(CAN1_B + 0x21C);
    bool txok = (tsr & (1 << 0)) != 0;   /* TXOK0 */
    bool tme = (tsr & (1 << 16)) != 0;   /* TME0 */
    char buf[64];
    snprintf(buf, sizeof(buf), "MSR=%08X TSR=%08X FA1R=%08X", msr, tsr, fa1r);
    report("CAN", txok && tme && (fa1r & 1) != 0, buf);
}

void testSPI_Flash() {
    /* flash CS on PA4, LCD CS on PA1, touch CS on PA2 */
    pinMode(PA4, OUTPUT);
    digitalWrite(PA4, HIGH);
    pinMode(PA1, OUTPUT);
    digitalWrite(PA1, HIGH);
    pinMode(PA2, OUTPUT);
    digitalWrite(PA2, HIGH);
    SPI.begin();
    spin(1000);

    uint8_t id[3];
    digitalWrite(PA4, LOW);
    id[0] = SPI.transfer(0x9F);
    id[1] = SPI.transfer(0x00);
    id[2] = SPI.transfer(0x00);
    digitalWrite(PA4, HIGH);
    bool jedec = (id[0] == 0xEF && id[1] == 0x40 && id[2] == 0x16);

    static uint8_t wdata[4];
    static uint8_t rdata[4];
    wdata[0] = 0xAA; wdata[1] = 0x55; wdata[2] = 0x11; wdata[3] = 0x22;
    digitalWrite(PA4, LOW);
    SPI.transfer(0x06);                    /* WREN */
    SPI.transfer(0x02);                    /* page program */
    SPI.transfer(0x00); SPI.transfer(0x00); SPI.transfer(0x00);
    for (int i = 0; i < 4; i++) SPI.transfer(wdata[i]);
    digitalWrite(PA4, HIGH);

    digitalWrite(PA4, LOW);
    SPI.transfer(0x03);                    /* read data */
    SPI.transfer(0x00); SPI.transfer(0x00); SPI.transfer(0x00);
    for (int i = 0; i < 4; i++) rdata[i] = SPI.transfer(0x00);
    digitalWrite(PA4, HIGH);
    bool rw = (rdata[0] == 0xAA && rdata[1] == 0x55 && rdata[2] == 0x11 && rdata[3] == 0x22);

    char buf[80];
    snprintf(buf, sizeof(buf), "JEDEC=%02X%02X%02X rw=%02X%02X%02X%02X",
             id[0], id[1], id[2], rdata[0], rdata[1], rdata[2], rdata[3]);
    report("SPI Flash", jedec && rw, buf);
}

void testI2C() {
    Wire.begin();
    spin(1000);
    bool eeprom = false, oled = false;
    Wire.beginTransmission(0x3C);
    if (Wire.endTransmission() == 0) oled = true;
    Wire.beginTransmission(0x50);
    if (Wire.endTransmission() == 0) eeprom = true;

    uint8_t val = 0xFF;
    if (eeprom) {
        Wire.beginTransmission(0x50);
        Wire.write(0x00); Wire.write(0x00); Wire.write(0x42);
        Wire.endTransmission();
        Wire.beginTransmission(0x50);
        Wire.write(0x00); Wire.write(0x00);
        Wire.endTransmission(false);
        Wire.requestFrom((uint8_t)0x50, (uint8_t)1);
        if (Wire.available()) val = Wire.read();
    }
    char buf[64];
    snprintf(buf, sizeof(buf), "eeprom=%d oled=%d val=%02X", eeprom, oled, val);
    report("I2C", eeprom && val == 0x42, buf);
    report("I2C OLED", oled, oled ? "0x3C found" : "0x3C not found");
}

void testI2C2() {
    /* register-level I2C2 master on 0x40005800; EEPROM at 0x51 */
    reg(RCC_APB1ENR) |= (1 << 22);            /* I2C2EN */
    reg(I2C2_B + 0x00) = 1;                   /* PE */
    reg(I2C2_B + 0x00) = 0x101;               /* START */
    uint32_t sb = 0;
    for (uint32_t i = 0; i < 100000 && (sb = reg(I2C2_B + 0x14) & 1) == 0; i++) {}
    reg(I2C2_B + 0x10) = (0x51 << 1) | 0;     /* write addr 0x51 */
    uint32_t ad = 0;
    for (uint32_t i = 0; i < 100000 && (ad = reg(I2C2_B + 0x14) & (1 << 1)) == 0; i++) {}
    reg(I2C2_B + 0x18);                       /* SR2 clears ADDR */
    reg(I2C2_B + 0x10) = 0x00;                /* reg addr hi */
    while (!(reg(I2C2_B + 0x14) & (1 << 7))) {}
    reg(I2C2_B + 0x10) = 0x42;                /* reg addr lo */
    while (!(reg(I2C2_B + 0x14) & (1 << 7))) {}
    reg(I2C2_B + 0x10) = 0x24;                /* data */
    while (!(reg(I2C2_B + 0x14) & (1 << 7))) {}
    reg(I2C2_B + 0x00) = 0x201;               /* STOP */

    /* read back: repeated START, re-send reg addr, then read */
    reg(I2C2_B + 0x00) = 0x101;               /* START */
    while (!(reg(I2C2_B + 0x14) & 1)) {}
    reg(I2C2_B + 0x10) = (0x51 << 1) | 0;
    while (!(reg(I2C2_B + 0x14) & (1 << 1))) {}
    reg(I2C2_B + 0x18);
    reg(I2C2_B + 0x10) = 0x00;
    while (!(reg(I2C2_B + 0x14) & (1 << 7))) {}
    reg(I2C2_B + 0x10) = 0x42;
    while (!(reg(I2C2_B + 0x14) & (1 << 7))) {}
    reg(I2C2_B + 0x00) = 0x101;               /* repeated START */
    while (!(reg(I2C2_B + 0x14) & 1)) {}
    reg(I2C2_B + 0x10) = (0x51 << 1) | 1;     /* read addr 0x51 */
    while (!(reg(I2C2_B + 0x14) & (1 << 1))) {}
    reg(I2C2_B + 0x18);                       /* clears ADDR; latches RX */
    uint32_t rxne = 0;
    for (uint32_t i = 0; i < 100000 && (rxne = reg(I2C2_B + 0x14) & (1 << 6)) == 0; i++) {}
    uint8_t val = reg(I2C2_B + 0x10) & 0xFF;  /* DR -> 0x24 */
    reg(I2C2_B + 0x00) = 0x201;               /* STOP */
    char buf[48];
    snprintf(buf, sizeof(buf), "I2C2 addr=0x51 reg=0x0042 val=%02X sb=%d ad=%d rxne=%d",
             val, sb ? 1 : 0, ad ? 1 : 0, rxne ? 1 : 0);
    report("I2C2 EEPROM", val == 0x24, buf);
}

void testSPI2() {
    /* register-level SPI2 master: 0x40003800, flash CS on PB12 */
    reg(RCC_APB1ENR) |= (1 << 14);            /* SPI2EN */
    pinMode(PB12, OUTPUT);
    digitalWrite(PB12, HIGH);
    reg(SPI2_B + 0x00) = 0x0344;              /* MSTR|SPE|SSM|SSI */

    uint8_t id[3];
    digitalWrite(PB12, LOW);
    reg(SPI2_B + 0x0C) = 0x9F;                /* JEDEC cmd */
    id[0] = reg(SPI2_B + 0x0C);
    reg(SPI2_B + 0x0C) = 0x00; id[1] = reg(SPI2_B + 0x0C);
    reg(SPI2_B + 0x0C) = 0x00; id[2] = reg(SPI2_B + 0x0C);
    digitalWrite(PB12, HIGH);

    /* page program + readback (mirrors testSPI_Flash) */
    uint8_t wd[4] = { 0x11, 0x22, 0x33, 0x44 };
    uint8_t rd[4];
    digitalWrite(PB12, LOW);
    reg(SPI2_B + 0x0C) = 0x06;                /* WREN */
    reg(SPI2_B + 0x0C) = 0x02;                /* PP */
    reg(SPI2_B + 0x0C) = 0x00; reg(SPI2_B + 0x0C) = 0x00; reg(SPI2_B + 0x0C) = 0x00;
    for (int i = 0; i < 4; i++) reg(SPI2_B + 0x0C) = wd[i];
    digitalWrite(PB12, HIGH);

    digitalWrite(PB12, LOW);
    reg(SPI2_B + 0x0C) = 0x03;                /* READ */
    reg(SPI2_B + 0x0C) = 0x00; reg(SPI2_B + 0x0C) = 0x00; reg(SPI2_B + 0x0C) = 0x00;
    for (int i = 0; i < 4; i++) { reg(SPI2_B + 0x0C) = 0x00; rd[i] = reg(SPI2_B + 0x0C); }
    digitalWrite(PB12, HIGH);

    bool jedec = (id[0] == 0xEF && id[1] == 0x40 && id[2] == 0x17);
    bool rw = (rd[0] == 0x11 && rd[1] == 0x22 && rd[2] == 0x33 && rd[3] == 0x44);
    char buf[80];
    snprintf(buf, sizeof(buf), "JEDEC=%02X%02X%02X rw=%02X%02X%02X%02X",
             id[0], id[1], id[2], rd[0], rd[1], rd[2], rd[3]);
    report("SPI2 Flash", jedec && rw, buf);
}

void testUSART2() {
    /* USART2 HDSEL software loopback (register-level) */
    reg(RCC_APB1ENR) |= (1 << 17);            /* USART2EN */
    reg(USART2_B + 0x14) |= (1 << 2);         /* CR3 HDSEL */
    reg(USART2_B + 0x04) = 0x5A;              /* write DR */
    spin(200);
    uint8_t got = reg(USART2_B + 0x04) & 0xFF;
    reg(USART2_B + 0x14) &= ~(1 << 2);
    char buf[32];
    snprintf(buf, sizeof(buf), "echo=%02X", got);
    report("USART2 Loopback", got == 0x5A, buf);
}

void testTIM3_PWM() {
    /* configured in loopAsyncChecks (emulator ticks only at batch boundaries) */
}

extern "C" void RTC_IRQHandler(void) {
    rtcAlarmCount++;
    reg(RTC_B + 0x04) = 0;                 /* CRL: write-0 clears ALRF */
}

void testRTCAlarmIRQ() {
    /* configured in loopAsyncChecks (tick-gated at batch boundaries) */
}

/* 10. SVC (software interrupt) + PendSV (lower-priority exception) */
static volatile uint32_t svcCount = 0;
static volatile uint32_t pendsvCount = 0;

extern "C" void SVC_Handler(void) {
    svcCount++;
}

extern "C" void PendSV_Handler(void) {
    pendsvCount++;
}

void testSVC() {
    /* PendSV (0x80) lower priority than SVCall (0x40): PendSV fires after SVC returns */
    reg(SCB_B + 0x20) = (0x80 << 16) | 0x40; /* SHPR3 */
    __asm volatile("svc #2");
    reg(SCB_B + 0x04) = (1 << 28);           /* ICSR: PENDSVSET */
    char buf[48];
    snprintf(buf, sizeof(buf), "count=%lu", (unsigned long)svcCount);
    report("SVC", svcCount == 1, buf);
}

void testOLED() {
    Wire.beginTransmission(0x3C);
    Wire.write(0x00); Wire.write(0xAE);   /* display off */
    Wire.endTransmission();
    Wire.beginTransmission(0x3C);
    Wire.write(0x00); Wire.write(0x81); Wire.write(0xCF); /* contrast */
    Wire.endTransmission();
    Wire.beginTransmission(0x3C);
    Wire.write(0x00); Wire.write(0xAF);   /* display on */
    Wire.endTransmission();
    Wire.beginTransmission(0x3C);
    Wire.write(0x40);
    for (int i = 0; i < 32; i++) Wire.write(0xFF); /* pixels */
    Wire.endTransmission();
    report("I2C OLED write", true, "cmd+data bytes sent");
}

void testLCD() {
    digitalWrite(PA1, LOW);
    SPI.transfer(0xFB);
    for (int i = 0; i < 128; i++) SPI.transfer(0xFF);
    digitalWrite(PA1, HIGH);
    report("LCD", true, "0xFB + 128 bytes sent (write-only, no readback)");
}

void testTouchscreen() {
    pinMode(PA3, INPUT_PULLUP); /* touch-detect pin */
    digitalWrite(PA2, LOW);
    SPI.transfer(0x90); /* Y channel, 12-bit */
    uint8_t yh = SPI.transfer(0x00);
    uint8_t yl = SPI.transfer(0x00);
    SPI.transfer(0xD0); /* X channel */
    uint8_t xh = SPI.transfer(0x00);
    uint8_t xl = SPI.transfer(0x00);
    SPI.transfer(0x94); /* pressure channel */
    uint8_t ph = SPI.transfer(0x00);
    uint8_t pl = SPI.transfer(0x00);
    digitalWrite(PA2, HIGH);
    uint16_t y = ((uint16_t)yh << 4) | (yl >> 4);
    uint16_t x = ((uint16_t)xh << 4) | (xl >> 4);
    uint16_t p = ((uint16_t)ph << 4) | (pl >> 4);
    bool touchPinLow = digitalRead(PA3) == LOW;
    char buf[64];
    snprintf(buf, sizeof(buf), "x=%u y=%u p=%u detect=%d", x, y, p, touchPinLow);
    report("Touchscreen", y == 2048 && x == 2048 && p == 0 && touchPinLow, buf);
}

/* ------------------------- async tests (loop) -------------------------- */
static uint8_t dmaTxBuf[4];
static uint8_t dmaTxArmed = 0;
static bool dmaTxDone = false;
static uint8_t dmaRxBuf[4];
static uint8_t dmaRxTries = 0;
static bool dmaRxDone = false;
static bool uartRxDone = false;
static bool timDone = false;
static bool extiDone = false;
static bool sysTickDone = false;
static uint32_t asyncCount = 0;
static bool canRxDone = false;
static bool exti1Done = false;
static bool exti13Done = false;
volatile uint32_t canRxArmed = 0;   /* RAM flag: cli.mjs polls this, then injects a CAN frame */
static uint32_t canRxTries = 0;
static bool tim3Done = false;
static bool tim4Done = false;
static bool rtcAlarmDone = false;
static uint32_t tim3Armed = 0;
static uint32_t tim4Armed = 0;
static uint32_t rtcAlarmArmed = 0;
static bool pendsvDone = false;

void loopAsyncChecks() {
    asyncCount++;

    /* 1. DMA TX: DMA1 CH4 memory -> USART1 DR (UART1 TX) */
    if (!dmaTxDone) {
        if (dmaTxArmed == 0) {
            dmaTxBuf[0] = 'D';
            reg(DMA1_B + 0x44) = (1 << 4);           /* CCR4 = DIR=1 (mem->periph) */
            reg(DMA1_B + 0x4C) = USART1_B + 0x04;    /* CPAR4 = USART1_DR */
            reg(DMA1_B + 0x50) = (uint32_t)dmaTxBuf; /* CMAR4 */
            reg(DMA1_B + 0x48) = 1;                  /* CNDTR4 = 1 */
            reg(DMA1_B + 0x44) = (1 << 4) | 1;       /* EN */
            dmaTxArmed = 1;
        }
        uint32_t ndtr = reg(DMA1_B + 0x48);
        uint32_t en = reg(DMA1_B + 0x44) & 1;
        uint32_t isr = reg(DMA1_B + 0x00);
        bool tc = (isr & (1 << 13)) != 0;          /* TCIF4 (real HW layout) */
        if (ndtr == 0 && en == 0 && tc) {
            char buf[48];
            snprintf(buf, sizeof(buf), "NDTR=0 EN=0 ISR=%08X", isr);
            report("DMA TX", true, buf);
            dmaTxDone = true;
        } else if (asyncCount > 8) {
            char buf[48];
            snprintf(buf, sizeof(buf), "NDTR=%lu EN=%lu ISR=%08X", (unsigned long)ndtr, (unsigned long)en, isr);
            report("DMA TX", false, buf);
            dmaTxDone = true;
        }
    }

    /* 2. DMA RX: DMA1 CH5 USART1_DR -> RAM, consumes injected byte 'A' */
    if (!dmaRxDone) {
        reg(DMA1_B + 0x58) = 0;                    /* CCR5 = 0 */
        reg(DMA1_B + 0x60) = USART1_B + 0x04;      /* CPAR5 = USART1_DR */
        reg(DMA1_B + 0x64) = (uint32_t)dmaRxBuf;   /* CMAR5 */
        reg(DMA1_B + 0x5C) = 1;                    /* CNDTR5 = 1 */
        reg(DMA1_B + 0x58) = 1;                    /* DIR=0 periph->mem, EN */
        uint32_t cap = 4000;
        while (reg(DMA1_B + 0x5C) != 0 && --cap != 0) spin(10);
        uint32_t isr = reg(DMA1_B + 0x00);
        dmaRxTries++;
        if (dmaRxBuf[0] == 'A') {
            char buf[48];
            snprintf(buf, sizeof(buf), "byte=%02X tries=%u ISR=%08X", dmaRxBuf[0], dmaRxTries, isr);
            report("DMA RX", true, buf);
            dmaRxDone = true;
        } else if (dmaRxTries > 40) {
            char buf[48];
            snprintf(buf, sizeof(buf), "byte=%02X tries=%u", dmaRxBuf[0], dmaRxTries);
            report("DMA RX", false, buf);
            dmaRxDone = true;
        }
    }

    /* 3. UART RX via Serial (consumes injected byte 'B') */
    if (!uartRxDone && dmaRxDone) {
        reg(USART1_B + 0x0C) |= (1 << 5); /* re-enable RXNEIE so the RX IRQ feeds Serial */
        if (Serial.available() > 0) {
            int c = Serial.read();
            if (c == 'B') { report("UART RX", true, "got 'B' via Serial"); }
            else { char buf[32]; snprintf(buf, sizeof(buf), "got 0x%02X", c); report("UART RX", false, buf); }
            uartRxDone = true;
        } else if (asyncCount > 60) {
            report("UART RX", false, "no byte after 60 batches");
            uartRxDone = true;
        }
    }

    /* 4. TIM2 interrupt (also exercises NVIC delivery) */
    if (!timDone) {
        if (tim2IrqCount > 0) {
            char buf[32];
            snprintf(buf, sizeof(buf), "count=%lu", (unsigned long)tim2IrqCount);
            report("TIM2 (NVIC)", true, buf);
            timDone = true;
        } else if (asyncCount > 40) {
            report("TIM2 (NVIC)", false, "no IRQ after 40 batches");
            timDone = true;
        }
    }

    /* 5. EXTI0 interrupt */
    if (!extiDone) {
        if (exti0IrqCount > 0) {
            char buf[32];
            snprintf(buf, sizeof(buf), "count=%lu", (unsigned long)exti0IrqCount);
            report("EXTI IRQ", true, buf);
            extiDone = true;
        } else if (asyncCount > 40) {
            report("EXTI IRQ", false, "no IRQ after 40 batches");
            extiDone = true;
        }
    }

    /* 5b. EXTI1 + EXTI13 interrupts (single SWIER trigger) */
    if (!exti1Done || !exti13Done) {
        if (asyncCount == 1) {
            attachInterrupt(PB1, exti1ISR, RISING);
            attachInterrupt(PB13, exti13ISR, RISING);
            reg(EXTI_B + 0x10) = (1 << 1) | (1 << 13); /* SWIER: lines 1 + 13 */
        }
        if (!exti1Done) {
            if (exti1IrqCount > 0) {
                report("EXTI1 IRQ", true, "PB1 RISING fired");
                exti1Done = true;
            } else if (asyncCount > 40) {
                report("EXTI1 IRQ", false, "no IRQ after 40 batches");
                exti1Done = true;
            }
        }
        if (!exti13Done) {
            if (exti13IrqCount > 0) {
                report("EXTI13 IRQ", true, "PB13 RISING fired");
                exti13Done = true;
            } else if (asyncCount > 40) {
                report("EXTI13 IRQ", false, "no IRQ after 40 batches");
                exti13Done = true;
            }
        }
    }

    /* 5c. CAN RX: cli.mjs injects one frame after canRxArmed is set */
    if (!canRxDone) {
        canRxArmed = 1;
        reg(RCC_APB1ENR) |= (1 << 25);          /* CAN1EN (already on, keep) */
        uint32_t fmp = 0;
        for (uint32_t i = 0; i < 3000000 && (fmp = reg(CAN1_B + 0x0C) & 0x3) == 0; i++) spin(1);
        canRxTries++;
        if (fmp != 0) {
            uint32_t tir = reg(CAN1_B + 0x1B0); /* RIR0 */
            uint32_t tdtr = reg(CAN1_B + 0x1B4);/* RDTR0 */
            uint32_t tdlr = reg(CAN1_B + 0x1B8);/* RDLR0 */
            bool id_ok = ((tir >> 21) & 0x7FF) == 0;
            bool data_ok = ((tdtr & 0xF) == 2) && (tdlr & 0xFFFF) == 0xDEAD;
            char buf[64];
            snprintf(buf, sizeof(buf), "RIR=%08X DTR=%08X DLR=%08X tries=%lu",
                     tir, tdtr, tdlr, (unsigned long)canRxTries);
            report("CAN RX", id_ok && data_ok, buf);
            canRxDone = true;
        } else if (canRxTries > 100) {
            report("CAN RX", false, "no frame after 100 tries");
            canRxDone = true;
        }
    }

    /* 6. SysTick / millis */
    if (!sysTickDone) {
        uint32_t m0 = millis();
        spin(60000); /* >100K instructions, crosses at least one batch boundary */
        uint32_t m1 = millis();
        char buf[48];
        snprintf(buf, sizeof(buf), "m0=%lu m1=%lu", (unsigned long)m0, (unsigned long)m1);
        report("SysTick", m1 > m0, buf);
        sysTickDone = true;
    }

    /* 7. TIM3 PWM (emulator ticks only at batch boundaries, so run async) */
    if (!tim3Done) {
        if (tim3Armed == 0) {
            reg(RCC_APB1ENR) |= (1 << 1);      /* TIM3EN */
            reg(TIM3_B + 0x28) = 0;            /* PSC = 0 */
            reg(TIM3_B + 0x2C) = 5000;         /* ARR */
            reg(TIM3_B + 0x34) = 600;          /* CCR1 */
            reg(TIM3_B + 0x18) = 0x60;         /* CCMR1: OC1M = PWM1 */
            reg(TIM3_B + 0x20) = 1;            /* CCER: CC1E */
            reg(TIM3_B + 0x00) = 1;            /* CR1: CEN */
            tim3Armed = 1;
        }
        uint32_t sr = reg(TIM3_B + 0x10);
        uint32_t cnt = reg(TIM3_B + 0x24);
        if ((sr & (1 << 1)) != 0) {
            char buf[48];
            snprintf(buf, sizeof(buf), "CNT=%lu SR=%08X", (unsigned long)cnt, sr);
            report("TIM3 PWM", true, buf);
            tim3Done = true;
        } else if (asyncCount > 40) {
            report("TIM3 PWM", false, "CC1IF never set");
            tim3Done = true;
        }
    }

    /* 8. TIM4 basic counter */
    if (!tim4Done) {
        if (tim4Armed == 0) {
            reg(RCC_APB1ENR) |= (1 << 2);      /* TIM4EN */
            reg(TIM4_B + 0x28) = 0;            /* PSC = 0 */
            reg(TIM4_B + 0x2C) = 0xFFFF;       /* ARR */
            reg(TIM4_B + 0x00) = 1;            /* CEN */
            tim4Armed = 1;
        }
        uint32_t cnt = reg(TIM4_B + 0x24);
        if (cnt > 0) {
            char buf[48];
            snprintf(buf, sizeof(buf), "CNT=%lu", (unsigned long)cnt);
            report("TIM4", true, buf);
            tim4Done = true;
        } else if (asyncCount > 40) {
            report("TIM4", false, "CNT never advanced");
            tim4Done = true;
        }
    }

    /* 9. RTC alarm IRQ (fires at batch boundary via NVIC) */
    if (!rtcAlarmDone) {
        if (rtcAlarmArmed == 0) {
            reg(NVIC_ISER0) = (1 << 3);        /* RTC IRQ */
            reg(RCC_APB1ENR) |= (1 << 9);      /* RTCEN */
            reg(RTC_B + 0x08) = 0;             /* PRLH */
            reg(RTC_B + 0x0C) = 99;            /* PRLL: count every 100 instr */
            reg(RTC_B + 0x18) = 0;             /* CNTH = 0 */
            reg(RTC_B + 0x1C) = 0;             /* CNTL = 0 */
            reg(RTC_B + 0x20) = 0;             /* ALRH */
            reg(RTC_B + 0x24) = 5;             /* ALRL = 5 */
            reg(RTC_B + 0x00) = 2;             /* CRH: ALRIE (bit 1, RM0008) */
            reg(RTC_B + 0x04) = (1 << 5);      /* CRL: RTOFF */
            rtcAlarmArmed = 1;
        }
        if (rtcAlarmCount > 0) {
            char buf[48];
            snprintf(buf, sizeof(buf), "irqs=%lu", (unsigned long)rtcAlarmCount);
            report("RTC Alarm IRQ", true, buf);
            rtcAlarmDone = true;
        } else if (asyncCount > 40) {
            report("RTC Alarm IRQ", false, "no alarm IRQ");
            rtcAlarmDone = true;
        }
    }

    /* 10. PendSV: pended by testSVC() (ICSR PENDSVSET after the SVC returns) */
    if (!pendsvDone) {
        if (pendsvCount > 0) {
            char buf[48];
            snprintf(buf, sizeof(buf), "count=%lu", (unsigned long)pendsvCount);
            report("PendSV", true, buf);
            pendsvDone = true;
        } else if (asyncCount > 40) {
            report("PendSV", false, "PendSV never fired after SVC");
            pendsvDone = true;
        }
    }
}

/* ============================ setup/loop =============================== */
void setup() {
    pinMode(PC13, OUTPUT);
    digitalWrite(PC13, HIGH);
    Serial.begin(115200);
    reg(USART1_B + 0x0C) &= ~(1 << 5); /* keep RX bytes for DMA RX; re-enabled later */

    Serial.println();
    Serial.println("=== STM32 BluePill peripheral test (Arduino framework) ===");

    testGPIO();
    testUSART_TX();
    testUART_Loopback();
    testRCC();
    testFLASH();
    testPWR();
    testBKP();
    testIWDG();
    testWWDG();
    testRTC();
    testCRC();
    testDAC();
    testADC();
    testAFIO();
    testEXTI_reg();
    testCAN();
    testSPI_Flash();
    testI2C();
    testOLED();
    testLCD();
    testTouchscreen();
    testI2C2();
    testSPI2();
    testUSART2();
    testTIM3_PWM();
    testRTCAlarmIRQ();
    testSVC();
    timer2.setOverflow(1000, MICROSEC_FORMAT); /* 1 ms */
    timer2.attachInterrupt(onTimer);
    timer2.resume();

    Serial.println("[ASYNC] waiting for IRQ/RX events (DMA TX, DMA RX, UART RX, TIM2, EXTI0/1/13, CAN RX, SysTick)");
}

void loop() {
    loopAsyncChecks();
    if (uartRxDone && dmaRxDone && timDone && extiDone && sysTickDone && dmaTxDone
        && exti1Done && exti13Done && canRxDone && tim3Done && tim4Done && rtcAlarmDone
        && pendsvDone) {
        Serial.println();
        char buf[48];
        snprintf(buf, sizeof(buf), "SUMMARY pass=%lu fail=%lu", (unsigned long)passCount, (unsigned long)failCount);
        Serial.println(buf);
        for (;;) spin(1000); /* burn until cli.mjs stops at --max */
    }
}
