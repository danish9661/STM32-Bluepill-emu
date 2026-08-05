## Progress

### Done
- **Comprehensive firmware** (`tests/comprehensive_test/comprehensive_test.ino`) — register-level Arduino sketch testing all 10 peripherals
- **10 peripheral tests**:
  1. GPIO (PC13 LED blink)
  2. USART TX (string output)
  3. **UART RX** (loopback via HDSEL — write 0xA5, read back match)
  4. I2C (scan + EEPROM write/read-back at addr 0x50)
  5. SPI (JEDEC ID read via 0x9F from Winbond W25Q64)
  6. ADC (read on PA0)
  7. Timer/PWM (TIM2 CH2 on PA1)
  8. RCC system info (HSI/HSE/PLL/SYSCLK status)
  9. EXTI (SWIER trigger via software)
  10. SysTick (1 ms timer, elapsed ms print)
- **GPIO pin config effect**: IDR reads respect pin mode (analog→0, input→callback, output→ODR); ODR/BSRR/BRR writes only affect output-mode pins
- **RCC clock gating**: Peripheral reads return 0 and writes are ignored when the corresponding AHB/APB2/APB1 enable bit is not set
- **CLI**: Added `--max=N` flag; fixed max instructions parsing for `--config` mode
- **Touchscreen**: XPT2046 protocol state machine with configurable touch coordinates, WASM bindings
- **USART software loopback**: HDSEL (CR3 bit 2) echoes TX bytes to RX buffer, enabling self-contained RX testing
- Fixed critical emulator bugs (flash offsets, RTC offsets, SYSTICK CVR, SPI DR/CRCPR, EXTI IRQ 42, TIM compare match, SCB CPUID M3, CRC bit-reversal, ADC interrupts, DMA deferred flags)

### In Progress
- *(none)*

### Known Limitations
- AFIO pin remapping: MAPR/EXTICR/MAPR2 registers are stored but pin remapping is not applied to peripherals
- Full peripherals not exercised: CAN, DAC, RTC, Flash, USB, PWR, WWDG, IWDG, CRC, DBGMCU

### Usage
```bash
# Run all 10 tests with config file
node pkg/cli.mjs --config=tests/comprehensive_test/config.yaml --max=10000000

# Run directly with firmware path
node pkg/cli.mjs tests/comprehensive_test/build/comprehensive_test.ino.bin 10000000

# Show register dump
node pkg/cli.mjs --config=tests/comprehensive_test/config.yaml --max=10000000 --regs
```

### Build
```bash
wasm-pack build --target web
```
