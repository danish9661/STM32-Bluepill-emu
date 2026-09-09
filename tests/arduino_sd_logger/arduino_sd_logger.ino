// STM32 Bluepill WASM emulator demo: SD data logger (SDIO + ADC + RTC).
//
// Register-level, no HAL: every RTC second the firmware samples the ADC
// internal temperature sensor, writes {magic, rtc, adc} to the next SD
// block (CMD24), reads it back (CMD17) and verifies. Progress prints as
// "log <n> rtc=<R> adc=<A> ok". Stops after 4 samples. malloc-free.
#include <Arduino.h>

#define SDIO_B 0x40018000u
#define S_POWER (SDIO_B + 0x00)
#define S_CLKCR (SDIO_B + 0x04)
#define S_ARG   (SDIO_B + 0x08)
#define S_CMD   (SDIO_B + 0x0C)
#define S_RESP1 (SDIO_B + 0x14)
#define S_DLEN  (SDIO_B + 0x28)
#define S_DCTRL (SDIO_B + 0x2C)
#define S_STA   (SDIO_B + 0x34)
#define S_ICR   (SDIO_B + 0x38)
#define S_FIFO  (SDIO_B + 0x80)
#define CPSMEN (1u << 10)
#define F_CMDREND (1u << 6)
#define F_DATAEND (1u << 8)

#define RTC_B 0x40002800u
#define ADC1_B 0x40012400u

static inline void w32(uint32_t a, uint32_t v) { *(volatile uint32_t *)a = v; }
static inline uint32_t r32(uint32_t a) { return *(volatile uint32_t *)a; }
static inline void w16(uint32_t a, uint16_t v) { *(volatile uint16_t *)a = v; }
static inline uint16_t r16(uint32_t a) { return *(volatile uint16_t *)a; }

static void sdcmd(uint32_t idx, uint32_t arg, uint32_t rsp) {
    w32(S_ARG, arg);
    w32(S_CMD, (idx & 0x3F) | rsp | CPSMEN);
}

static bool sd_init(void) {
    w32(S_POWER, 0x03);
    w32(S_CLKCR, 0x100 | 0x76);
    sdcmd(0, 0, 0);
    sdcmd(8, 0x1AA, 1 << 6);
    if (r32(S_RESP1) != 0x1AA) return false;
    for (int i = 0; i < 10; i++) {
        sdcmd(55, 0, 1 << 6);
        sdcmd(41, 1u << 30, 1 << 6);
        if (r32(S_RESP1) & 0x80000000u) break;
    }
    if (!(r32(S_RESP1) & 0x80000000u)) return false;
    sdcmd(2, 0, 3 << 6);
    sdcmd(3, 0, 1 << 6);
    sdcmd(7, 0x12340000, 1 << 6);
    sdcmd(16, 512, 1 << 6);
    w32(S_ICR, 0xFFFFFFFF);
    return true;
}

static uint32_t read_temp(void) {
    w32(ADC1_B + 0x34, 16);                    // SQ1 = ch16 (temp sensor)
    w32(ADC1_B + 0x08, (1 << 0) | (1 << 22));  // ADON + SWSTART
    for (uint32_t i = 0; i < 100000 && !(r32(ADC1_B) & 2); i++) {}
    return r32(ADC1_B + 0x4C) & 0xFFF;
}

static void sd_write_block(uint32_t blk, uint32_t rtc, uint32_t adc) {
    w32(S_DLEN, 512);
    w32(S_DCTRL, 0x3); // DTEN + DTDIR(write)
    sdcmd(24, blk, 1 << 6);
    w32(S_FIFO, 0x4C4F4731); // "LOG1"
    w32(S_FIFO, rtc);
    w32(S_FIFO, adc);
    for (int i = 3; i < 128; i++) w32(S_FIFO, 0);
    for (uint32_t i = 0; i < 100000 && !(r32(S_STA) & F_DATAEND); i++) {}
    w32(S_ICR, 0xFFFFFFFF);
}

static bool sd_verify_block(uint32_t blk, uint32_t rtc, uint32_t adc) {
    w32(S_DLEN, 512);
    w32(S_DCTRL, 0x1); // DTEN (read)
    sdcmd(17, blk, 1 << 6);
    for (uint32_t i = 0; i < 100000 && !(r32(S_STA) & F_CMDREND); i++) {}
    uint32_t m = r32(S_FIFO), r = r32(S_FIFO), a = r32(S_FIFO);
    for (int i = 3; i < 128; i++) (void)r32(S_FIFO);
    w32(S_ICR, 0xFFFFFFFF);
    return m == 0x4C4F4731 && r == rtc && a == adc;
}

void setup() {
    pinMode(PC13, OUTPUT);
    Serial.begin(115200);
    Serial.println("\r\n=== SD data logger (SDIO block log + ADC temp + RTC) ===");
    RCC->APB1ENR |= RCC_APB1ENR_PWREN | RCC_APB1ENR_BKPEN;
    PWR->CR |= PWR_CR_DBP;
    w32(RTC_B + 0x08, 0x000F);   // PRLH: prescaler 1M instr/tick
    w32(RTC_B + 0x0C, 0x4240);   // PRLL
    w32(RTC_B + 0x18, 0);        // CNT = 0
    w32(RTC_B + 0x1C, 0);
    (void)r32(RTC_B + 0x04);
    RCC->APB2ENR |= RCC_APB2ENR_ADC1EN;
    w32(ADC1_B + 0x08, 1);       // ADON
    Serial.println(sd_init() ? "sd ready" : "sd INIT FAILED");
}

void loop() {
    static uint32_t last = 0xFFFFFFFFu, n = 0;
    if (n >= 4) return;
    uint32_t rtc = (r32(RTC_B + 0x18) << 16) | (r32(RTC_B + 0x1C) & 0xFFFF);
    if (rtc != last) {
        last = rtc;
        uint32_t adc = read_temp();
        sd_write_block(100 + n, rtc, adc);
        bool ok = sd_verify_block(100 + n, rtc, adc);
        Serial.print("log ");
        Serial.print(n);
        Serial.print(" rtc=");
        Serial.print(rtc);
        Serial.print(" adc=");
        Serial.print(adc);
        Serial.println(ok ? " ok" : " MISMATCH");
        n++;
        if (n >= 4) Serial.println("logger done");
    }
}
