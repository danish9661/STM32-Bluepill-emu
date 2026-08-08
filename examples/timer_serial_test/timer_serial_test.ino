/*
 * Timer + Serial test firmware for STM32 BluePill (F103C8)
 *
 * - TIM2 overflow interrupt fires every 1 second, increments a counter
 * - Every second prints "T+N s" on Serial (USART1, PA9 TX / PA10 RX)
 * - Anything typed into Serial is echoed back prefixed with "ECHO: "
 *
 * Tests: serial TX, serial RX, timer interrupt accuracy, custom firmware load
 *
 * NOTE: avoid `new`/String (malloc) — STM32duino's _sbrk uses `mrs msp`,
 * which this WASM emulator's Unicorn CPU cannot decode.
 */
#include <Arduino.h>
#include <HardwareTimer.h>

HardwareTimer timer2(TIM2);
volatile uint32_t tickSec = 0;

void onTimer() {
    tickSec++;
}

void setup() {
    pinMode(PC13, OUTPUT);
    Serial.begin(115200);

    timer2.setOverflow(1000000, MICROSEC_FORMAT); // 1 second period
    timer2.attachInterrupt(onTimer);
    timer2.resume();

    Serial.println("STM32 timer/serial test started");
    Serial.println("1 line per second, input is echoed back:");
}

void loop() {
    static uint32_t last = 0;
    uint32_t now = tickSec;
    if (now != last) {
        last = now;
        digitalWrite(PC13, last & 1);
        Serial.print("T+");
        Serial.print(last);
        Serial.println(" s");
    }
    static char line[64];
    static uint8_t li = 0;
    while (Serial.available() > 0) {
        char c = (char)Serial.read();
        if (c == '\n' || li >= sizeof(line) - 1) {
            line[li] = 0;
            Serial.print("ECHO: ");
            Serial.println(line);
            li = 0;
        } else if (c != '\r') {
            line[li++] = c;
        }
    }
}
