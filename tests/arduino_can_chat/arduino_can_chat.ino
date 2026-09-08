// STM32 Bluepill WASM emulator demo: CAN self-chat in loopback mode.
//
// CAN1 runs with LBKM (loopback, BTR.30) and an accept-all filter, so every
// transmitted frame returns into RX FIFO0: the firmware sends an incrementing
// frame and prints both sides (tx id/data, rx id/data). No transceiver or
// second node needed. malloc-free (no String/new).
#include <Arduino.h>

#define CAN1_B 0x40006400u
#define reg(a) (*(volatile uint32_t *)(a))

static void spin(volatile uint32_t n) { while (n--) { } }

void setup() {
    pinMode(PC13, OUTPUT);
    Serial.begin(115200);
    Serial.println("\r\n=== CAN loopback chat demo (CAN1 LBKM) ===");
    Serial.println("tx id=.. data=..  ->  rx id=.. data=..");

    RCC->APB1ENR |= (1 << 25);   // CAN1EN
    RCC->APB2ENR |= (1 << 3);    // GPIOBEN (pins idle in loopback)
    reg(CAN1_B + 0x00) &= ~1u;   // leave init mode
    spin(100);
    // BTR: LBKM (bit 30) + 500kbps-ish timing
    reg(CAN1_B + 0x1C) = (1u << 30) | (1u << 24) | (3u << 20) | (4u << 16) | 1u;
    reg(CAN1_B + 0x200) = 1;     // filter init mode
    reg(CAN1_B + 0x204) = 1;     // 32-bit scale, bank 0
    reg(CAN1_B + 0x20C) = 0;     // mask mode (not list)
    reg(CAN1_B + 0x240) = 0;     // ID=0, mask=0: accept all
    reg(CAN1_B + 0x21C) = 1;     // activate filter 0
    reg(CAN1_B + 0x200) = 0;     // exit filter init
}

void loop() {
    static uint32_t n = 0;
    uint32_t id = 0x100 + (n & 0xFF);
    uint32_t data = 0xC0DE0000u | (n & 0xFFFFu);
    reg(CAN1_B + 0x180) = (id << 21);      // TI0R: STDID (no TXRQ yet)
    reg(CAN1_B + 0x184) = 4;               // DLC=4
    reg(CAN1_B + 0x188) = data;
    reg(CAN1_B + 0x18C) = 0;
    reg(CAN1_B + 0x180) |= 1;              // TXRQ: mailbox complete now
    // Wait for the looped-back frame (FMP0), with a generous timeout.
    uint32_t spins = 0;
    while ((reg(CAN1_B + 0x0C) & 0x3) == 0 && spins < 1000000) spins++;
    uint32_t rtir = 0, rdata = 0;
    if ((reg(CAN1_B + 0x0C) & 0x3) != 0) {
        rtir = reg(CAN1_B + 0x1B0);        // RIR0 (FMP decrements on read)
        rdata = reg(CAN1_B + 0x1B8);       // RDL0R
    }
    digitalWrite(PC13, (n & 1) ? LOW : HIGH);
    Serial.print("tx id=");
    Serial.print(id, HEX);
    Serial.print(" data=");
    Serial.print(data, HEX);
    Serial.print("  ->  rx id=");
    Serial.print((rtir >> 21) & 0x7FF, HEX);
    Serial.print(" data=");
    Serial.println(rdata, HEX);
    n++;
    delay(500);
}
