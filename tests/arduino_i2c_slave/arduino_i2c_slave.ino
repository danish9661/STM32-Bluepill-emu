// STM32 Bluepill WASM emulator demo: I2C slave @ 0x42 over Wire.
//
// Register-level host on the test side (i2c_inject_*): the host writes
// bytes (onReceive prints them) and reads bytes back (onRequest replies
// "Hi"). Exercises slave ADDR match, RXNE/TXE sequencing, STOPF and the
// EV interrupts, all with zero CPU in the transfer path. malloc-free
// (no String/new).
#include <Arduino.h>
#include <Wire.h>

static void onRecv(int n) {
    Serial.print("rx=");
    Serial.print(n);
    Serial.print(":");
    while (Wire.available()) {
        Serial.print(" ");
        Serial.print(Wire.read(), HEX);
    }
    Serial.println();
}

static void onReq() {
    Wire.write("Hi");
}

void setup() {
    pinMode(PC13, OUTPUT);
    Serial.begin(115200);
    Serial.println("\r\n=== I2C slave demo (Wire @ 0x42, onReceive/onRequest) ===");
    Wire.begin(0x42);
    Wire.onReceive(onRecv);
    Wire.onRequest(onReq);
    Serial.println("slave ready");
}

void loop() {
}
