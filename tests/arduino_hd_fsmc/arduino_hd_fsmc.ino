// High-density desk for Generic F103RC: exercises peripherals the Blue Pill
// (medium-density C8) lacks in silicon — FSMC NOR and dual DAC — all real
// on the 256K/48K RC. Prints FSMC verify + DAC loopbacks. (ADC3 exists only
// on the SVD map, so loopback stays on ADC1.)
#include <Arduino.h>

#define FSMC_BCR1   (*(volatile uint32_t *)0xA0000000u)
#define NE1         ((volatile uint32_t *)0x60000000u)
#define DAC_CR      (*(volatile uint32_t *)0x40007400u)
#define DAC_DHR12R1 (*(volatile uint32_t *)0x40007408u)
#define DAC_DHR12R2 (*(volatile uint32_t *)0x40007414u)
#define ADC1_B      0x40012400u
#define ADC_SR      0x00u
#define ADC_CR2     0x08u
#define ADC_SQR3    0x34u
#define ADC_DR      0x4Cu
#define regB(b, o)  (*(volatile uint32_t *)((b) + (o)))

static uint32_t adc_once(uint32_t base, uint8_t ch) {
    regB(base, ADC_SQR3) = ch;
    regB(base, ADC_CR2) = (1u << 0);              // ADON
    for (volatile uint32_t i = 0; i < 100; i++) ;
    regB(base, ADC_CR2) = (1u << 0) | (1u << 22); // ADON + SWSTART
    for (uint32_t t = 0; t < 100000; t++) {
        if (regB(base, ADC_SR) & (1u << 1)) break; // EOC
    }
    return regB(base, ADC_DR) & 0xFFFu;
}

void setup() {
    Serial.begin(115200);
    Serial.println("\r\n=== HD desk (F103RC): FSMC + DAC ===");
    RCC->AHBENR |= 1u;                            // FSMCEN
    RCC->APB1ENR |= (1u << 29);                   // DACEN
    RCC->APB2ENR |= (1u << 9);                    // ADC1EN
    FSMC_BCR1 = 0x3u;                             // MBKEN + WREN
    DAC_CR = (1u << 0) | (1u << 16);              // EN1 + EN2
}

void loop() {
    static uint32_t n = 0;
    // FSMC NOR write/read verify.
    NE1[0x40] = 0x12345678u + n;
    uint32_t v = NE1[0x40];
    bool fok = (v == 0x12345678u + n);
    // DAC1 midscale -> ADC1 CH4 loopback; DAC2 ramp -> ADC1 CH5.
    DAC_DHR12R1 = 0x800u;
    DAC_DHR12R2 = (n * 0x111u) & 0xFFFu;
    uint32_t dac1 = adc_once(ADC1_B, 4);
    uint32_t dac2 = adc_once(ADC1_B, 5);
    Serial.print("hd ");
    Serial.print(n);
    Serial.print(" fsmc=");
    Serial.print(fok ? "ok" : "FAIL");
    Serial.print(" dac1=");
    Serial.print(dac1, HEX);
    Serial.print(" dac2=");
    Serial.println(dac2, HEX);
    n++;
    delay(1000);
}
