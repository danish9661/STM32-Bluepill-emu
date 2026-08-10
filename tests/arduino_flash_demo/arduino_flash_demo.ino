/*
 * STM32 Bluepill WASM emulator demo: SPI flash JEDEC ID + I2C EEPROM
 * write/readback + OLED presence scan, printed once on UART.
 *
 * malloc-free (no String/new).
 */
#include <Arduino.h>
#include <SPI.h>
#include <Wire.h>

void setup() {
    Serial.begin(115200);
    delay(10);
    Serial.println("\r\n=== SPI flash + I2C devices on UART ===");

    char buf[40];

    pinMode(PA4, OUTPUT);              /* SPI1 flash CS */
    digitalWrite(PA4, HIGH);
    SPI.begin();                       /* SPI lib global = SPI1 on this variant */
    digitalWrite(PA4, LOW);
    /* Emulator flash model: the ID's first byte arrives as the response of the
     * JEDEC cmd byte itself (real silicon shifts it in during the cmd byte). */
    uint8_t j0 = SPI.transfer(0x9F);   /* JEDEC read ID cmd */
    uint8_t j1 = SPI.transfer(0xFF);
    uint8_t j2 = SPI.transfer(0xFF);
    digitalWrite(PA4, HIGH);
    snprintf(buf, sizeof(buf), "JEDEC=%02X%02X%02X", j0, j1, j2);
    Serial.println(buf);

    Wire.begin();                      /* I2C1 */
    Wire.beginTransmission(0x3C);
    bool oled = Wire.endTransmission() == 0;
    snprintf(buf, sizeof(buf), "OLED=%s", oled ? "found" : "missing");
    Serial.println(buf);

    Wire.beginTransmission(0x50);      /* EEPROM: write 0x42 @ addr 0 */
    Wire.write(0x00);
    Wire.write(0x00);
    Wire.write(0x42);
    bool wr = Wire.endTransmission() == 0;
    delay(10);

    Wire.beginTransmission(0x50);
    Wire.write(0x00);
    Wire.write(0x00);
    uint8_t val = 0xFF;
    if (Wire.endTransmission(false) == 0) {
        if (Wire.requestFrom((uint8_t)0x50, (uint8_t)1) == 1) val = Wire.read();
    }
    snprintf(buf, sizeof(buf), "EEPROM wr=%d rd=%02X", wr, val);
    Serial.println(buf);
}

void loop() {
    delay(1000);
}