/*
 * STM32 Bluepill WASM emulator demo: TIM2 1-second interrupt printing the
 * live timer registers on UART.
 *
 * Every second prints:  t=<elapsed seconds>  cnt=<live TIM2 counter>  arr=<auto-reload>
 * PC13 toggles once per second.
 *
 * The emulator maps 1 instruction = 1 timer tick, so the period is tuned to
 * ~1M instructions (~0.2 s wall in the browser demo) — on real silicon this
 * is a 13.9 ms period; this firmware is for the emulator demo.
 * malloc-free (no String/new).
 */
#include <Arduino.h>
#include <HardwareTimer.h>

HardwareTimer timer2(TIM2);
volatile uint32_t secs = 0;

void onTimer() {
    secs++;
}

void setup() {
    pinMode(PC13, OUTPUT);
    Serial.begin(115200);

    timer2.setPrescaleFactor(1);        // PSC = 0: 1 instr = 1 tick
    timer2.setOverflow(999999, TICK_FORMAT); // ~1M instructions per period
    timer2.attachInterrupt(onTimer);
    timer2.resume();

    Serial.println("\r\n=== TIM2 timer on UART demo ===");
    Serial.println("t=<elapsed s>  cnt=<live counter>  arr=<auto-reload>");
}

void loop() {
    static uint32_t last = 0;
    uint32_t now = secs;
    if (now != last) {
        last = now;
        digitalWrite(PC13, now & 1);
        Serial.print("t=");
        Serial.print(now);
        Serial.print("s cnt=");
        Serial.print((uint32_t)TIM2->CNT);
        Serial.print(" arr=");
        Serial.println((uint32_t)TIM2->ARR);
    }
}
