// Per-board UART echo firmware: prints its board name, then echoes every
// received byte on the board's native Serial port (USART1 on Blue Pill /
// Maple Mini, USART2 on Nucleo-F103RB / Generic F103RC) and blinks
// LED_BUILTIN on each byte. Compiled once per target — the binary proves
// which board build is running via the banner.
#include <Arduino.h>

static const char *boardName() {
#if defined(ARDUINO_NUCLEO_F103RB)
    return "Nucleo-F103RB";
#elif defined(ARDUINO_MAPLEMINI_F103CB)
    return "Maple Mini";
#elif defined(ARDUINO_GENERIC_F103RCTX)
    return "Generic F103RC";
#else
    return "Blue Pill";
#endif
}

void setup() {
    pinMode(LED_BUILTIN, OUTPUT);
    digitalWrite(LED_BUILTIN, LOW);
    Serial.begin(115200);
    Serial.print("\r\n=== board echo: ");
    Serial.print(boardName());
    Serial.println(" (type to echo) ===");
}

void loop() {
    if (Serial.available()) {
        int c = Serial.read();
        Serial.write((uint8_t)c);
        digitalWrite(LED_BUILTIN, HIGH);
        delay(20);
        digitalWrite(LED_BUILTIN, LOW);
    }
}
