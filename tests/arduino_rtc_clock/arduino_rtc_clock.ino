// STM32 Bluepill WASM emulator demo: RTC wall-clock on UART.
//
// Register-level RTC setup (no HAL): the counter is preset to 12:00:00 and
// the prescaler is set so CNT advances once per ~1M instructions (~1 emulated
// second). Each tick prints HH:MM:SS and PC13 blinks.
// malloc-free (no String/new).
#include <Arduino.h>

#define RTC_B   0x40002800u
#define RTC_CRH (RTC_B + 0x00)
#define RTC_CRL (RTC_B + 0x04)
#define RTC_PRLH (RTC_B + 0x08)
#define RTC_PRLL (RTC_B + 0x0C)
#define RTC_CNTH (RTC_B + 0x18)
#define RTC_CNTL (RTC_B + 0x1C)

static inline void reg_write(uint32_t addr, uint32_t v) {
    *(volatile uint32_t *)addr = v;
}
static inline uint32_t reg_read(uint32_t addr) {
    return *(volatile uint32_t *)addr;
}

static void print2(uint32_t v) {
    Serial.print((char)('0' + (v / 10) % 10));
    Serial.print((char)('0' + v % 10));
}

static const char *boardName() {
#if defined(ARDUINO_NUCLEO_F103RB)
    return "Nucleo-F103RB";
#elif defined(ARDUINO_MAPLEMINI_F103CB)
    return "Maple Mini";
#elif defined(ARDUINO_GENERIC_F103RCTX)
    return "Generic F103RC";
#else
    return "Blue Pill";
#endif
}

void setup() {
    pinMode(LED_BUILTIN, OUTPUT);
    Serial.begin(115200);
    Serial.println("\r\n=== RTC clock demo ===");
    Serial.print("board: ");
    Serial.println(boardName());
    Serial.println("HH:MM:SS prints every emulated second (RTC CNT, PRL=1M).");

    RCC->APB1ENR |= RCC_APB1ENR_PWREN | RCC_APB1ENR_BKPEN;
    PWR->CR |= PWR_CR_DBP;             // allow BKP domain writes
    reg_write(RTC_PRLH, 0x000F);       // prescaler = 0xF4240 (1M instr/tick)
    reg_write(RTC_PRLL, 0x4240);
    reg_write(RTC_CNTH, 0);            // start at 12:00:00
    reg_write(RTC_CNTL, 12u * 3600u);
    (void)reg_read(RTC_CRL);
}

void loop() {
    static uint32_t last = 0xFFFFFFFFu;
    uint32_t cnt = (reg_read(RTC_CNTH) << 16) | (reg_read(RTC_CNTL) & 0xFFFF);
    if (cnt != last) {
        last = cnt;
        uint32_t day = cnt % 86400u;
        digitalWrite(LED_BUILTIN, (cnt & 1) ? LOW : HIGH);
        print2(day / 3600u);
        Serial.print(':');
        print2((day / 60u) % 60u);
        Serial.print(':');
        print2(day % 60u);
        Serial.print("  rtc=");
        Serial.println(cnt);
    }
}
