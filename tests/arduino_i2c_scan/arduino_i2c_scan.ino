// STM32 Bluepill WASM emulator demo: I2C bus scanner.
//
// Probes every 7-bit address with a zero-length write (Wire API) and prints
// the ACKed ones. With the page preset (24Cxx EEPROM at 0x50 + SSD1306 OLED
// at 0x3C) it finds both; headless it finds whatever devices are attached.
// malloc-free (no String/new).
#include <Arduino.h>
#include <Wire.h>

void setup() {
    pinMode(PC13, OUTPUT);
    Serial.begin(115200);
    Serial.println("\r\n=== I2C scanner demo (Wire, 0x08..0x77) ===");
    Wire.begin();
    uint32_t found = 0;
    for (uint8_t addr = 0x08; addr < 0x78; addr++) {
        Wire.beginTransmission(addr);
        uint8_t err = Wire.endTransmission();
        if (err == 0) {
            Serial.print("found 0x");
            if (addr < 0x10) Serial.print('0');
            Serial.println(addr, HEX);
            found++;
            digitalWrite(PC13, LOW);
            delay(20);
            digitalWrite(PC13, HIGH);
        }
    }
    Serial.print("done, found=");
    Serial.println(found);
}

void loop() {
    // One-shot scan; blink slowly so the page stays alive.
    digitalWrite(PC13, LOW);
    delay(500);
    digitalWrite(PC13, HIGH);
    delay(500);
}
