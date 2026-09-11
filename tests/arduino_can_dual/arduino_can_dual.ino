// STM32 emulator demo: dual-CAN self-talk (CAN1 + CAN2 loopback).
//
// Runs on the STM32F105 SVD map (CAN2 @ 0x40006800 exists only there).
// Both controllers run LBKM with accept-all filters; each iteration sends
// one frame per bus and prints both looped-back frames. No transceiver or
// second node needed. malloc-free (no String/new).
#include <Arduino.h>

#define CAN1_B 0x40006400u
#define CAN2_B 0x40006800u
#define reg(a) (*(volatile uint32_t *)(a))

static void spin(volatile uint32_t n) { while (n--) { } }

static void can_setup(uint32_t base) {
    reg(base + 0x00) &= ~1u;   // leave init mode
    spin(100);
    reg(base + 0x1C) = (1u << 30) | (1u << 24) | (3u << 20) | (4u << 16) | 1u;
}

// Silicon filter bank: all 28 banks live in CAN1 (CAN2SB=14 default splits
// 0..13 to CAN1, 14..27 to CAN2); CAN2 owns no filter registers. Bank 0
// (CAN1) and bank 14 (CAN2) both accept-all, 32-bit mask mode.
static void can_filters(void) {
    reg(CAN1_B + 0x200) = 1;     // filter init mode
    reg(CAN1_B + 0x204) = (1u << 0) | (1u << 14); // 32-bit scale, banks 0 + 14
    reg(CAN1_B + 0x20C) = 0;     // mask mode (not list)
    reg(CAN1_B + 0x240) = 0;     // bank 0: ID=0, mask=0: accept all
    reg(CAN1_B + 0x244) = 0;
    reg(CAN1_B + 0x240 + 14 * 8) = 0; // bank 14: ID=0, mask=0: accept all
    reg(CAN1_B + 0x244 + 14 * 8) = 0;
    reg(CAN1_B + 0x21C) = (1u << 0) | (1u << 14); // activate filters 0 + 14
    reg(CAN1_B + 0x200) = (14u << 8); // exit init, CAN2SB=14 (default)
}

static void can_xfer(uint32_t base, const char *name, uint32_t id, uint32_t data) {
    reg(base + 0x180) = (id << 21);
    reg(base + 0x184) = 4;
    reg(base + 0x188) = data;
    reg(base + 0x18C) = 0;
    reg(base + 0x180) |= 1;    // TXRQ
    uint32_t spins = 0;
    while ((reg(base + 0x0C) & 0x3) == 0 && spins < 1000000) spins++;
    uint32_t rtir = 0, rdata = 0;
    if ((reg(base + 0x0C) & 0x3) != 0) {
        rtir = reg(base + 0x1B0);
        rdata = reg(base + 0x1B8);
    }
    Serial.print(name);
    Serial.print(" tx id=");
    Serial.print(id, HEX);
    Serial.print(" data=");
    Serial.print(data, HEX);
    Serial.print("  ->  rx id=");
    Serial.print((rtir >> 21) & 0x7FF, HEX);
    Serial.print(" data=");
    Serial.println(rdata, HEX);
}

void setup() {
    pinMode(PC13, OUTPUT);
    Serial.begin(115200);
    Serial.println("\r\n=== Dual-CAN demo (CAN1 + CAN2 LBKM, F105 map) ===");
    RCC->APB1ENR |= (1 << 25) | (1 << 26);   // CAN1EN + CAN2EN
    RCC->APB2ENR |= (1 << 3);                // GPIOBEN (pins idle in loopback)
    can_setup(CAN1_B);
    can_setup(CAN2_B);
    can_filters();
    Serial.println("both buses up");
}

void loop() {
    static uint32_t n = 0;
    uint32_t data = 0xC0DE0000u | (n & 0xFFFFu);
    can_xfer(CAN1_B, "CAN1", 0x100 + (n & 0xFF), data);
    can_xfer(CAN2_B, "CAN2", 0x200 + (n & 0xFF), data ^ 0x5555);
    digitalWrite(PC13, (n & 1) ? LOW : HIGH);
    n++;
    delay(500);
}
