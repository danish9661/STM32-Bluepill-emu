// STM32 Bluepill WASM emulator demo: hobby-servo sweep on TIM3 CH1 (PA6).
// Register-level setup (no HAL): 50Hz PWM (20ms period in emulator ticks),
// 1ms..2ms pulse width maps 0..180 degrees and back. Each step prints the
// angle; PC13 toggles at each sweep end. The PA6 pulse train is visible in
// the page's pin-activity monitor. malloc-free (no String/new).
#include <Arduino.h>

void setup() {
    pinMode(PC13, OUTPUT);
    digitalWrite(PC13, HIGH);
    Serial.begin(115200);
    Serial.println("\r\n=== Servo sweep demo (TIM3 CH1/PA6, 50Hz) ===");
    Serial.println("deg=<0..180>  pulse=<1000..2000>");

    RCC->APB1ENR |= RCC_APB1ENR_TIM3EN;
    RCC->APB2ENR |= RCC_APB2ENR_IOPAEN | RCC_APB2ENR_AFIOEN;
    GPIOA->CRL = (GPIOA->CRL & ~(0xFu << 24)) | (0xBu << 24);  // PA6: AF push-pull
    TIM3->PSC = 999;      // 1 tick per 1000 instructions
    TIM3->ARR = 19;       // 20ms period
    TIM3->CCMR1 = 0x0060; // CH1: PWM mode 1
    TIM3->CCER = 0x0001;  // CH1 enable
    TIM3->CCR1 = 1;       // 1ms pulse (0 deg)
    TIM3->CR1 = 1;        // CEN
}

void loop() {
    static int32_t deg = 0;
    static int32_t dir = 5;
    uint32_t pulse = 1000u + (uint32_t)deg * 1000u / 180u;  // 1..2ms in ticks
    TIM3->CCR1 = pulse / 1000u;  // CCR units of 1000 instr (1..2)
    if (deg == 0 || deg == 180) {
        digitalWrite(PC13, LOW);
        Serial.print("turn deg=");
        Serial.println(deg);
    }
    Serial.print("deg=");
    Serial.print(deg);
    Serial.print("  pulse=");
    Serial.println(pulse);
    deg += dir;
    if (deg >= 180) { deg = 180; dir = -5; }
    if (deg <= 0) { deg = 0; dir = 5; }
    digitalWrite(PC13, HIGH);
    delay(20);
}
