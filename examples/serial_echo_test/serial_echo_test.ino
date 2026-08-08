void setup() {
    Serial.begin(115200);
    Serial.println("SERIAL TX OK");
}

void loop() {
    while (Serial.available() > 0) {
        char c = (char)Serial.read();
        Serial.print("ECHO:");
        Serial.write(c);
        Serial.println(".");
    }
}
