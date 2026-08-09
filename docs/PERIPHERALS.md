# Peripheral & Feature Support

Everything below is emulated at **register level** — the firmware talks to the exact
memory-mapped registers of an STM32F103C8 and the emulator behaves like the hardware,
including interrupt generation, status flags, and timing. Support levels:

- **Full** — register-accurate, with interrupts and timing, exercised by the 37-check
  firmware test (`tests/arduino_periph_test`) and/or unit tests.
- **Partial** — some sub-features implemented; see notes.
- **Stub** — registers exist / reads return 0; listed for completeness.

## CPU core

| Unit | Level | Notes |
|---|---|---|
| Cortex-M3 (Thumb-2) | Full | Unicorn engine. One known gap: the `mrs rX, msp` instruction can't be decoded — patched in JS (`patchMrsMsp`). |
| NVIC | Full | 68 IRQs, priority-based dispatch, pending/active sets, PRIMASK/BASEPRI gating, `last_popped` fairness so hot IRQs don't starve others. |
| SysTick | Full | 1 ms debt accrual, multiple 1 ms IRQs per batch, COUNTFLAG, calibration register. `millis()`/`delay()` work. |
| SCB | Full | Core system control block registers. |

## Power, reset, clock

| Unit | Level | Notes |
|---|---|---|
| RCC | Full | Clock enables (AHB/APB1/APB2), reset registers, IRQ, clock configuration accepted. Emulated clock is a fixed 8 MHz instruction budget — PLL values are accepted but the emulated "Hz" is derived from instruction count. |
| PWR | Full | Power control registers (PDDS, SLEEPDEEP, etc.). |
| BKP | Full | Backup registers + tamper. |
| IWDG | Full | Independent watchdog: down-counter, refresh, **triggers an emulator stop** on expiry (JS checks `is_watchdog_reset_requested()`). |
| WWDG | Full | Window watchdog, same reset semantics. |
| FLASH | Full | Flash interface: unlock/lock, program (byte/halfword/word), erase, option bytes, status flags. |
| FSMC | Partial | Register file registered (addresses decode); no actual external-memory backing. |

## GPIO & system interconnect

| Unit | Level | Notes |
|---|---|---|
| GPIO A–D | Full | CRL/CRH/IDR/ODR/BSRR/BRR/LCKR, pull-ups, open-drain, alternate function, `read_pin_effective()` (input callback else driven output) for CS/touch lines. Readbacks (`gpio_read_output`, `gpio_read_input`) are exposed to JS. |
| AFIO | Full | Remap registers, EXTI line selectors. |
| EXTI | Full | 20 lines, rising/falling/level triggers, software-triggered (SWIER), per-line IRQ mapping to NVIC (including the enable side — IMR writes enable the mapped IRQ). |
| DMA1 | Full | 7 channels: peripheral→memory, memory→peripheral, memory→memory; CNDTR counts down across batches; transfer-complete IRQs; JS-side batched transfer (`dma_get_all_pending`/`dma_set_completed_many`). |

## Serial buses

| Unit | Level | Notes |
|---|---|---|
| USART1–3 | Full | TXE/TC with **baud-rate byte pacing** (byte_time = 8M/baud instructions), RXNE + RX interrupt, overrun, HDSEL half-duplex loopback (loopback works for self-test), `uart_rx_byte()` injection, `uart_rx_pending()` gate, `get_uart_output()` capture of all transmitted text. |
| SPI1–2 | Full | Master mode with 8/16-bit frames, CPOL/CPHA, bit-order; device selection via GPIO CS; serves external devices (flash/OLED/LCD/touchscreen); TX/RX FIFO behaviour. I2S registers decode but audio output is simulated (`generate_i2s_audio`). |
| I2C1–2 | Full | Master state machine (START, address 7-bit, TX/RX, STOP, repeated START), SR1/SR2 status flags, error handling (AF, BER), interrupts (EV/ER). 10-bit addressing and slave mode are **not** implemented. |
| CAN1 | Full | Mailboxes (TIR/TDTR/TDLR/TDHR), filters (ID-list and mask modes), TX request + TX-complete IRQ, RX FIFO + RX IRQ, `can_inject_message()` to inject a received frame from JS. |
| USB | **Stub** | Registers read 0; USB firmware will not work. |

## Timers

| Unit | Level | Notes |
|---|---|---|
| TIM1–7 | Full | PSC/ARR/CNT with instruction-delta advance (no `ticks.min()` cap — ALL accumulated ticks processed per batch), PWM1/2 output compare (duty exposed via `pwm_duty()`), input capture, update events, UIE/CCIE interrupts, CCR1–4. TIM6/7 basic timers included. |
| RTC | Full | Calendar registers, **alarm with IRQ** (custom `RTC_IRQHandler` works), BKP interface. |
| DAC | Full | DHR/DOR registers, output value readback. |

## Analog

| Unit | Level | Notes |
|---|---|---|
| ADC1–2 | Partial | Full register file (CR1/CR2/SR/SQR/SMPR), conversion is **simulated**: firmware reads come from `adc_set_sim_value()` — no real analog model. |
| CRC | Full | CRC32 computation over written bytes, DR readback. |

## External devices (emulated bus devices)

These are *extra* peripherals the STM32 talks to over SPI/I2C — the "rest of the board".

| Device | Bus | Level | Notes |
|---|---|---|---|
| SPI NOR flash (e.g. W25Q) | SPI1/2 | Full | JEDEC ID (RDID/0x9F), manufacturer/device ID (0x90), status regs (WEL), page program, sector/subsector/bulk erase, continuous read, fast read (dummy byte), CS-gated command state machine. Backed by a JS `Uint8Array` file image. |
| EEPROM (e.g. 24Cxx) | I2C1/2 | Full | 7-bit addressing, byte/sequential read & write, page behavior, repeated START, address counter. File-backed. |
| OLED (e.g. SSD1306-style 128×64) | I2C | Full | Command + display-data state machine with a framebuffer; `add_i2c_oled` configures size. |
| LCD (e.g. SPI TFT) | SPI | Partial | Command/data stream state machine (`0xFB` command latch); pixel rendering is not provided. |
| Resistive touchscreen (ADS7846) | SPI | Full | Command decoding (channels incl. pressure 0x94), 8/12-bit modes, **deferred reply** (reply arrives on the SPI transfer *after* the command, like the real part), touch injection via `touchscreen_set_touch()` + touch-detect GPIO line. |
| Software SPI (bit-banged GPIO) | GPIO | Full | `add_software_spi()`: CS/CLK/MISO/MOSI pins, emulated on GPIO transitions. |

## What is NOT emulated

- **USB** (stub).
- **Real ADC conversion** (values are injected), real analog comparators.
- **CPU exceptions other than IRQs**: hard faults are tolerated (unmapped access →
  instruction skipped), but there is no fault handler execution model.
- **Multiple cores / sleep state timing** — firmware runs at a flat instruction rate;
  `delay()`/`millis()` are correct, but power-down modes don't slow the wall clock.
- Real GPIO/electrical behaviour (glitches, rise times, external pull strength).

## Verification coverage

- **158 unit tests** (`node tests/test_all.mjs`) — GPIO, USART, ADC, RCC, SysTick, TIM,
  IWDG, NVIC, CRC, SPI, I2C, RTC, PWR, FLASH, CAN, DMA, AFIO, EXTI, BKP, DAC, TIM6,
  RTC Alarm, UART RX.
- **37-check firmware test** (`node tests/canary.mjs`, 37/37): runs a real Arduino sketch
  compiled with STM32duino against sync + async scenarios (DMA TX/RX, UART RX, TIM2
  overflow IRQ, EXTI0/1/13, CAN RX injection, SysTick, TIM3 PWM, TIM4 CNT, RTC alarm IRQ).
- CI (`.github/workflows/test.yml`) rebuilds the WASM and runs both suites on every push.
