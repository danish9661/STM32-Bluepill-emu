// STM32 Bluepill WASM emulator demo: UART echo + LED feedback.
// Type in the terminal window: every character is echoed back,
// and the onboard LED (PC13) blinks once per received byte.
#include <Arduino.h>

void setup() {
    pinMode(PC13, OUTPUT);
    digitalWrite(PC13, HIGH); // LED off (active-low on the bluepill)
    Serial1.begin(115200);
    Serial1.print("\r\n=== UART Echo demo ===\r\nType anything; each char is echoed back and the LED blinks.\r\n");
}

void loop() {
    if (Serial1.available()) {
        uint8_t ch = (uint8_t)Serial1.read();
        Serial1.write(ch);
        digitalWrite(PC13, LOW);   // LED on
        delay(50);
        digitalWrite(PC13, HIGH);  // LED off
    }
}
