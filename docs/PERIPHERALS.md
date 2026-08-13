# Peripheral & Feature Support

Everything below is emulated at **register level** — the firmware talks to the exact
memory-mapped registers of an STM32F103C8 and the emulator behaves like the hardware,
including interrupt generation, status flags, and timing. Support levels:

- **Full** — register-accurate, with interrupts and timing, exercised by the 39-check
  firmware test (`tests/arduino_periph_test`) and/or unit tests.
- **Partial** — some sub-features implemented; see notes.
- **Stub** — registers exist / reads return 0; listed for completeness.

## CPU core

| Unit | Level | Notes |
|---|---|---|
| Cortex-M3 (Thumb-2) | Full | Unicorn engine. One known gap: the `mrs rX, msp` instruction can't be decoded — patched in JS (`patchMrsMsp`). |
| NVIC | Full | 68 IRQs, priority-based dispatch, pending/active sets, PRIMASK/BASEPRI gating, `last_popped` fairness so hot IRQs don't starve others. ISR return is one Rust call (`finish_interrupt(irq)`) that pops the active-priority stack **and** drains SysTick debt ticks internally — no JS re-pend loop. |
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
| FSMC | Full | All 7 external-memory banks (NE1–4 @ 0x6000/0x6400/0x6800/0x6C00, NAND2 @ 0x7000, NAND3 @ 0x8000, PC-Card @ 0x9000) with BCR/BTR/BWTR/PCR/PMEM/PATT registers, MBKEN/WREN gating, byte/16/32-bit accesses. Backed by a JS `Uint8Array` image per bank (`add_fsmc_bank('FSMC.BANK1', data)`). |

## GPIO & system interconnect

| Unit | Level | Notes |
|---|---|---|
| GPIO A–D | Full | CRL/CRH/IDR/ODR/BSRR/BRR/LCKR, pull-ups, open-drain, alternate function, `read_pin_effective()` (input callback else driven output) for CS/touch lines. **Electrical model**: IDR readback honors input pull-up/down (ODR bit selects direction), push-pull output readback, open-drain released level (external pull or 0), external drivers win over driven state, and output **slew** (IDR shows the old level until the transition settles, `gpio_set_slew(n)` instructions). Readbacks (`gpio_read_output`, `gpio_read_input`) are exposed to JS. |
| AFIO | Full | Remap registers, EXTI line selectors. |
| EXTI | Full | 20 lines, rising/falling/level triggers, software-triggered (SWIER), per-line IRQ mapping to NVIC (including the enable side — IMR writes enable the mapped IRQ). **Input pins fire edges too**: `gpioSetInput()` level changes go through the same edge detection as GPIO output writes, so page-driven button widgets drive `attachInterrupt()`. |
| DMA1 | Full | 7 channels: peripheral→memory, memory→peripheral, memory→memory; CNDTR counts down across batches; transfer-complete IRQs; **pump runs in Rust** (`dma_pump_all()` pops the queue, absorbs/pushes peripheral bytes internally, returns a flat op plan for JS: memcpy / store-absorbed / read-RAM-then-push / done-bits — JS only touches Unicorn RAM via `uc.mem_read`/`mem_write`; completion signaled last so TC IRQs fire after the data lands). DMA1@0x40020000 / DMA2@0x40020400 on both the builtin map and SVD maps (dual-map bug class removed 2026-08-11). **Real-HW semantics (2026-08-13)**: ISR completion flags sit at channel N's real nibble (TCIF_N = `(N-1)*4+1` — was off by one channel, CH4 lit bit 16 instead of 13); DIR follows CMSIS (DIR=1 → push CMAR→CPAR, DIR=0 → absorb CPAR→CMAR, M2M → CPAR→CMAR memcpy per RM0008 — was inverted, so a mem→periph channel silently absorbed from the peripheral address); 8-bit SPI data needs PSIZE/MSIZE=00 (16-bit pushes `max(psize,msize)*ndtr` = 2× the wire bytes, since the F103 SPI clocks only 8 bits per 16-bit DR write). |

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
| ADC1–2 | Full | Real conversion state machine: per-sequence channels (SQR/JSQR), sample-time timing (`Tconv = SMP + 12.5` cycles, 1 instr = 1 cycle), EOC/STRT/JEOC/JSTRT flags, EOCS, AWD with HTR/LTR + AWDIE, CONT auto-restart, ADC1→DMA1 ch1 / ADC2→DMA1 ch2 requests. **Sources**: `adc_set_sim_value()` serves exact readings (legacy), `gpioSetAnalog()` wires a 12-bit pin voltage, and enabled DAC channels drive their output pins (DAC1→PA4/ch4, DAC2→PA5/ch5) — real sources sample through an RC sample-and-hold (`adcSetRcTau`). **External triggers**: EXTTRIG/JEXTTRIG with EXTSEL/JEXTSEL sources TIM1_CC1/2/3, TIM1_TRGO, TIM2_CC2, TIM3_TRGO, TIM4_CC4 (+ injected TIM1_CC4/TIM2_TRGO/TIM2_CC2/TIM3_CC4/TIM4_TRGO) emitted from timer update (MMS=update) and compare events, and EXTI lines 11 (regular) / 15 (injected); a new conversion starts when a trigger arrives (ignored while busy). |
| DAC | Full | DHR12/8 (L/R/D) registers, dual channels, output registers DOR1/2 driven on writes. Enabled channels drive a 12-bit analog wire on their pins (F103: DAC1→PA4, DAC2→PA5) that ADC channels mapped to those pins sample via the RC path. |
| CRC | Full | CRC32 computation over written bytes, DR readback. |

## External devices (emulated bus devices)

These are *extra* peripherals the STM32 talks to over SPI/I2C — the "rest of the board".

| Device | Bus | Level | Notes |
|---|---|---|---|
| SPI NOR flash (e.g. W25Q) | SPI1/2 | Full | JEDEC ID (RDID/0x9F), manufacturer/device ID (0x90), status regs (WEL), page program, sector/subsector/bulk erase, continuous read, fast read (dummy byte), CS-gated command state machine. Backed by a JS `Uint8Array` file image. |
| EEPROM (e.g. 24Cxx) | I2C1/2 | Full | 7-bit addressing, byte/sequential read & write, page behavior, repeated START, address counter. File-backed. |
| OLED (e.g. SSD1306-style 128×64) | I2C | Full | Command + display-data state machine with a framebuffer; `add_i2c_oled` configures size. Framebuffer readback via `i2c_oled_fb('I2C1', 0x3C)` (128×64 bytes, 8 vertical pixels/byte) for page-side canvas rendering. |
| LCD (e.g. SPI TFT) | SPI | Partial | Command/data stream state machine (`0xFB` command latch); pixel rendering is not provided — the 128×64 byte-per-pixel framebuffer is exposed via `lcd_fb('SPI1')` and the page renders it. |
| Resistive touchscreen (ADS7846) | SPI | Full | Command decoding (channels incl. pressure 0x94), 8/12-bit modes, **deferred reply** (reply arrives on the SPI transfer *after* the command, like the real part), touch injection via `touchscreen_set_touch()` + touch-detect GPIO line. |
| Software SPI (bit-banged GPIO) | GPIO | Full | `add_software_spi()`: CS/CLK/MISO/MOSI pins, emulated on GPIO transitions. |

## What is NOT emulated

- **USB** (stub).
- **Real analog input model** — ADC converts with real timing/flags. By default
  it samples the injected `adc_set_sim_value()` exactly, but wiring a pin with
  `gpioSetAnalog(port, pin, level)` engages an RC sample-and-hold: the sampling
  cap charges from its held voltage over the SMP window (`adcSetRcTau` time
  constant) and holds its charge across conversions; channels 16/17/18 use
  nominal internal values. No comparator peripherals.
- **Cortex-M fault-preemption details** — faults are raised with CFSR/HFSR/BFAR
  bookkeeping and run through the same handler dispatch as IRQs (with SHCSR
  escalation to HardFault), but precise stack/return-address semantics of a
  real core are approximated.
- **Power consumption / wall-clock slowdown** — STOP/STANDBY freezes all
  peripherals except RTC + IWDG (SysTick included), but the emulated wall clock
  doesn't slow down and wake is immediate on the next IRQ.
- Slew rise/fall shaping (transitions are 2-state), glitches, and external pull
  *strength* (drivers are digital).

## Verification coverage

- **236 unit tests** (`node tests/test_all.mjs`) — GPIO (incl. electrical model: pull-ups,
  open-drain, external-driver precedence, slew readback, pin-change events), USART, ADC (real conversion
  timing, RC sample-and-hold via gpioSetAnalog, DAC→ADC loopback, AWD IRQ, TIM1 TRGO /
  TIM1_CC1 / EXTI 11 external triggers), RCC, SysTick, TIM, IWDG, NVIC, CRC, SPI, I2C,
  RTC, PWR, FLASH, CAN, DMA, AFIO, EXTI, BKP, DAC, TIM6, RTC Alarm, UART RX, FSMC
  (MBKEN/WREN gating, byte/word access), deep-sleep gating (TIM frozen, RTC alive, resume
  without catch-up), fault escalation (CFSR/HFSR/BFAR, BusFault vs HardFault, IBUSERR,
  SHPR routing).
- **39-check firmware test** (`node tests/canary.mjs`, 39/39): runs a real Arduino sketch
  compiled with STM32duino against sync + async scenarios (DMA TX/RX with real-HW ISR
  bits + CMSIS DIR, UART RX, TIM2
  overflow IRQ, EXTI0/1/13, CAN RX injection, SysTick, TIM3 PWM, TIM4 CNT, RTC alarm IRQ,
  **SVC + PendSV**).
- **WS2812 strip demo** (`tests/arduino_ws2812/` + browser preset): an 8-LED 800 kHz strip
  streamed over SPI1 at 2.25 MHz (div32) via DMA1 CH3 — validates the DMA **mem→peripheral
  data path** end-to-end (72 bytes/frame decoded to exact GRB colors across frames;
  previously the direction inversion made such transfers complete with zero bytes moved).
- CI (`.github/workflows/test.yml`) rebuilds the WASM and runs both suites on every push.
