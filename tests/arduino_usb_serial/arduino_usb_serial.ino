// Real STM32duino USBSerial (CDC-ACM) firmware for the Blue Pill: prints a
// banner and echoes every received byte over USB. Exercises the emulator's
// USB FS-device model through Arduino's real USB stack (not hand-rolled
// registers): enumeration, control transfers, bulk EP1 echo.
#include <Arduino.h>
#include <USBSerial.h>

void setup() {
    SerialUSB.begin();
    while (!SerialUSB) { }
    SerialUSB.println("\r\n=== USBSerial echo (CDC-ACM, real stack) ===");
}

void loop() {
    if (SerialUSB.available()) {
        int c = SerialUSB.read();
        SerialUSB.write((uint8_t)c);
    }
}
