// STM32 Bluepill WASM emulator demo: TIM2 CH1 (PA0) breathing PWM.
// Register-level setup (no HAL): 1kHz PWM on PA0, duty ramps 0->100%->0,
// PC13 LED blinks each cycle, current duty printed to UART periodically.
#include <Arduino.h>

void setup() {
    pinMode(PC13, OUTPUT);
    digitalWrite(PC13, HIGH);
    Serial1.begin(115200);
    Serial1.print("\r\n=== TIM2 PWM fade demo ===\r\nPA0: 1kHz PWM, duty breathes 0..100%. PC13 blinks once per cycle.\r\n");
    RCC->APB1ENR |= RCC_APB1ENR_TIM2EN;
    RCC->APB2ENR |= RCC_APB2ENR_IOPAEN | RCC_APB2ENR_AFIOEN;
    GPIOA->CRL = (GPIOA->CRL & ~0x0F) | 0x0B;  // PA0: AF push-pull 50MHz
    TIM2->PSC = 71;
    TIM2->ARR = 1000;
    TIM2->CCMR1 = 0x0020;  // CH1: PWM mode 1, no preload
    TIM2->CCER = 0x0001;   // CH1 enable
    TIM2->CCR1 = 0;
    TIM2->CR1 = 1;         // CEN
}

uint16_t fade = 0;

void loop() {
    TIM2->CCR1 = fade < 1000 ? fade : 2000 - fade;
    fade = (fade + 2) % 2000;
    digitalWrite(PC13, LOW);   // LED on at the top of each cycle
    if (fade < 4) { Serial1.print("duty="); Serial1.print(TIM2->CCR1); Serial1.print("\r\n"); }
    delay(2);
    digitalWrite(PC13, HIGH);  // LED off
}
