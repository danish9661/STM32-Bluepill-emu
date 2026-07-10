void setup() {
  // PC13 LED
  *(volatile uint32_t*)0x40021018 |= (1 << 4);
  volatile uint32_t crh = *(volatile uint32_t*)0x40011004;
  crh = (crh & ~(0xF << 20)) | (0x3 << 20);
  *(volatile uint32_t*)0x40011004 = crh;
}

void loop() {
  volatile uint32_t *bsrr = (uint32_t*)0x40011010;
  *bsrr = (1 << 13);
  HAL_Delay(500);
  *bsrr = (1 << 29);
  HAL_Delay(500);
}
