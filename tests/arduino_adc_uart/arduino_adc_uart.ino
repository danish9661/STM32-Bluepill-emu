/*
 * STM32 Bluepill WASM emulator demo: ADC1 on PA0 printed over UART each
 * second. The cadence comes from TIM2 (HardwareTimer, ~1M instructions per
 * period in emulator time); each tick starts a real ADC1 conversion
 * (SWSTART) with full sample timing and prints the 12-bit result.
 * malloc-free (no String/new).
 */
#include <Arduino.h>
#include <HardwareTimer.h>

HardwareTimer timer2(TIM2);
volatile bool tick = false;

void onTimer() {
    tick = true;
}

void setup() {
    Serial.begin(115200);

    timer2.setPrescaleFactor(1);        // PSC = 0: 1 instr = 1 tick
    timer2.setOverflow(999999, TICK_FORMAT); // ~1M instructions per period
    timer2.attachInterrupt(onTimer);
    timer2.resume();

    RCC->APB2ENR |= RCC_APB2ENR_ADC1EN;
    GPIOA->CRL = 0x44444444; // PA0 analog input
    ADC1->SQR3 = 0;          // sequence = ch0 (PA0)
    ADC1->SMPR2 = 0;         // SMP0 = 1.5 cycles (fast)
    ADC1->CR2 = 1;           // ADON

    Serial.println("\r\n=== ADC on UART demo (PA0) ===");
    Serial.println("adc=<0..4095> each second");
}

void loop() {
    if (!tick) return;
    tick = false;
    ADC1->CR2 |= (1 << 22);    // SWSTART
    while ((ADC1->SR & (1 << 1)) == 0) { } // wait EOC (real conversion timing)
    Serial.print("adc=");
    Serial.println((ADC1->DR & 0xFFF));
}
