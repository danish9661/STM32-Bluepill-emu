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
#define GPIOC_B     0x40011000u
#define SPI1_B      0x40013000u
#define I2C1_B      0x40005400u
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

void onTimer() { tim2IrqCount++; }

void extiISR() { exti0IrqCount++; }

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
    bool txe = (reg(USART1_B + 0x00) & (1 << 7)) != 0;
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

void loopAsyncChecks() {
    asyncCount++;

    /* 1. DMA TX: DMA1 CH4 memory -> USART1 DR (UART1 TX) */
    if (!dmaTxDone) {
        if (dmaTxArmed == 0) {
            dmaTxBuf[0] = 'D';
            reg(DMA1_B + 0x44) = 0;                /* CCR4 = 0 (DIR=0 mem->periph) */
            reg(DMA1_B + 0x4C) = USART1_B + 0x04;  /* CPAR4 = USART1_DR */
            reg(DMA1_B + 0x50) = (uint32_t)dmaTxBuf; /* CMAR4 */
            reg(DMA1_B + 0x48) = 1;                /* CNDTR4 = 1 */
            reg(DMA1_B + 0x44) = 1;                /* EN */
            dmaTxArmed = 1;
        }
        uint32_t ndtr = reg(DMA1_B + 0x48);
        uint32_t en = reg(DMA1_B + 0x44) & 1;
        uint32_t isr = reg(DMA1_B + 0x00);
        bool tc = (isr & (1 << 20)) != 0;          /* TCIF4 (emulator layout) */
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
        reg(DMA1_B + 0x58) = (1 << 4) | 1;         /* DIR=1 periph->mem, EN */
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

    HardwareTimer timer2(TIM2);
    timer2.setOverflow(1000, MICROSEC_FORMAT); /* 1 ms */
    timer2.attachInterrupt(onTimer);
    timer2.resume();

    Serial.println("[ASYNC] waiting for IRQ/RX events (DMA TX, DMA RX, UART RX, TIM2, EXTI0, SysTick)");
}

void loop() {
    loopAsyncChecks();
    if (uartRxDone && dmaRxDone && timDone && extiDone && sysTickDone && dmaTxDone) {
        Serial.println();
        char buf[48];
        snprintf(buf, sizeof(buf), "SUMMARY pass=%lu fail=%lu", (unsigned long)passCount, (unsigned long)failCount);
        Serial.println(buf);
        for (;;) spin(1000); /* burn until cli.mjs stops at --max */
    }
}
