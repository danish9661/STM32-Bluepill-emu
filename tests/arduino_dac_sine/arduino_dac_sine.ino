// STM32 Bluepill WASM emulator demo: DAC sine wave with ADC loopback.
//
// Register-level DAC1 CH1 (PA4) driven through a 16-point sine table while
// ADC1 CH4 (same PA4 pin) samples the wire: the emulator's DAC->ADC analog
// loopback carries each output back into the converter. Each step prints
// the DAC set-point and the ADC readback. malloc-free (no String/new).
#include <Arduino.h>

#define DAC_B   0x40007400u
#define DAC_CR  (DAC_B + 0x00)
#define DAC_DHR (DAC_B + 0x08)  // DHR12R1

// 16-point sine, 0..4095 (offset 2048, amplitude 2000).
static const uint16_t SINE[16] = {
    2048, 2831, 3495, 3939, 4048, 3939, 3495, 2831,
    2048, 1264, 600, 156, 48, 156, 600, 1264,
};

void setup() {
    pinMode(PC13, OUTPUT);
    Serial.begin(115200);
    Serial.println("\r\n=== DAC sine + ADC loopback demo (PA4) ===");
    Serial.println("dac=<set>  adc=<readback>");

    RCC->APB1ENR |= (1 << 29);   // DACEN
    RCC->APB2ENR |= RCC_APB2ENR_ADC1EN | RCC_APB2ENR_IOPAEN;
    GPIOA->CRL &= ~(0xFu << 16);   // PA4 analog
    *(volatile uint32_t *)DAC_CR |= 0x01;  // DAC CH1 enable
    ADC1->SQR3 = 4;                // sequence = ch4 (PA4)
    ADC1->SMPR2 = 0;               // fast sample
    ADC1->CR2 = 1;                 // ADON
}

void loop() {
    static uint32_t step = 0;
    uint16_t v = SINE[step % 16];
    *(volatile uint32_t *)DAC_DHR = v;
    ADC1->CR2 |= (1 << 22);        // SWSTART
    while ((ADC1->SR & (1 << 1)) == 0) { }  // wait EOC
    uint16_t back = ADC1->DR & 0xFFF;
    digitalWrite(PC13, (step & 1) ? LOW : HIGH);
    Serial.print("dac=");
    Serial.print(v);
    Serial.print("  adc=");
    Serial.println(back);
    step++;
    delay(50);
}
