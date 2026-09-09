// Per-board demo firmware: prints its board name, blinks LED_BUILTIN and
// ticks on UART. Compiled once per target (Blue Pill / Maple Mini /
// Nucleo-F103RB / Generic F103RC) — the binary proves which board build
// is running via the banner; LED_BUILTIN resolves per variant.
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
    Serial.begin(115200);
    Serial.print("\r\n=== board demo: ");
    Serial.print(boardName());
    Serial.println(" ===");
}

void loop() {
    static uint32_t n = 0;
    digitalWrite(LED_BUILTIN, n & 1);
    Serial.print("tick ");
    Serial.println(n);
    n++;
    delay(500);
}
