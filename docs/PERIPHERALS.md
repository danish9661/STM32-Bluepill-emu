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
| Cortex-M3 (Thumb-2) | Full | Native Rust interpreter (`src/cpu/`): full integer Thumb-2 incl. IT blocks, DSP faults (M3-correct), FPU/coprocessor faults, SVC/WFI/MSP-PSP banking, DWT CYCCNT (1+LATENCY cycles/instr). |
| ITM stimulus (PPB @0xE0000000) | Full | Stimulus port 0 printf channel: TER[0]+TCR.ITMENA-gated word writes drain as `ItmByte` events (disc 22, `onItmByte`); TER/TPR/TCR stored, STIM reads ready. ATB/TPIU/timestamps/ports 1–31 out of scope. |
| NVIC | Full | 68 IRQs, priority-based dispatch, pending/active sets, PRIMASK/BASEPRI gating, `last_popped` fairness so hot IRQs don't starve others. ISR return is one Rust call (`finish_interrupt(irq)`) that pops the active-priority stack **and** drains SysTick debt ticks internally — no JS re-pend loop. Driver note: pending bits coalesce intra-batch events (one delivery per batch per IRQ) — rate-accurate runs need production-sized batches (~20K), never 1M steps. |
| SysTick | Full | 1 ms debt accrual with phase-preserving trigger (no overshoot loss at any batch size), one re-pend per delivery (whole-debt drains coalesce into the single pending bit and lose ticks), COUNTFLAG, calibration register. `millis()`/`delay()` run at exact instruction rate. |
| SCB | Full | Core system control block registers. |

## Power, reset, clock

| Unit | Level | Notes |
|---|---|---|
| RCC | Full | Clock enables (AHB/APB1/APB2), reset registers, IRQ, clock configuration accepted. Emulated clock is a fixed 8 MHz instruction budget — PLL values are accepted but the emulated "Hz" is derived from instruction count. Decoded tree queryable (`rcc_clocks_hz` → sys/hclk/pclk1/pclk2; `rcc_mco_hz` → MCO pin selection, square wave itself out of scope). |
| PWR | Full | Power control registers (PDDS, SLEEPDEEP, etc.) + live `pwrMode()` query (RUN/SLEEP/STOP/STANDBY, WFI-tracked). PVD has PLS threshold levels against a settable supply (`pwr_set_supply_mv`, default 3.3 V) with EXTI16 edges; standby (PDDS) wakes only on WKUP (PA0+EWUP→WUF), RTC alarm and IWDG/NRST, other EXTI lines record PR without pending. Current draw stays a documented DS5319-typical estimate, not a modeled quantity. |
| BKP | Full | Backup registers + tamper. |
| IWDG | Full | Independent watchdog: down-counter, refresh, **triggers an emulator stop** on expiry (JS checks `is_watchdog_reset_requested()`). |
| WWDG | Full | Window watchdog, same reset semantics. |
| FLASH | Full | Flash interface: unlock/lock, program (byte/halfword/word), erase, option bytes, status flags. OBR USER byte is settable (erased default = software watchdog); WDG_SW clear runs the IWDG from reset without a KR start; nRST_STOP/nRST_STDBY stored (no reset sequencing, consistent with wake-without-reset). |
| FSMC | Full | All 7 external-memory banks (NE1–4 @ 0x6000/0x6400/0x6800/0x6C00, NAND2 @ 0x7000, NAND3 @ 0x8000, PC-Card @ 0x9000) with BCR/BTR/BWTR/PCR/PMEM/PATT registers, MBKEN/WREN gating, byte/16/32-bit accesses. Backed by a JS `Uint8Array` image per bank (`add_fsmc_bank('FSMC.BANK1', data)`). |

## GPIO & system interconnect

| Unit | Level | Notes |
|---|---|---|
| GPIO A–D | Full | CRL/CRH/IDR/ODR/BSRR/BRR/LCKR, pull-ups, open-drain, alternate function, `read_pin_effective()` (input callback else driven output) for CS/touch lines. **Electrical model**: IDR readback honors input pull-up/down (ODR bit selects direction), push-pull output readback, open-drain released level (external pull or 0), external drivers win over driven state, and output **slew** (IDR shows the old level until the transition settles, `gpio_set_slew(n)` instructions). Readbacks (`gpio_read_output`, `gpio_read_input`) are exposed to JS. **LCKR**: full lock sequence (LCKK+LCK, LCK, LCKK+LCK) freezes configuration nibbles until reset. |
| AFIO | Full | Remap registers, EXTI line selectors. **SWJ_CFG** (MAPR[26:24]): debug-port reservation is enforced — reserved JTAG/SWD pins (PA13–15/PB3–4 per mode) ignore GPIO configuration writes until released. |
| EXTI | Full | 20 lines, rising/falling/level triggers, software-triggered (SWIER), per-line IRQ mapping to NVIC (including the enable side — IMR writes enable the mapped IRQ). **Input pins fire edges too**: `gpioSetInput()` level changes go through the same edge detection as GPIO output writes, so page-driven button widgets drive `attachInterrupt()`. **Standby gating**: with SLEEPDEEP+PDDS, lines other than WKUP (EXTI0+EWUP) and the RTC alarm (line 17) record PR without pending. |
| DMA1 | Full | 7 channels: peripheral→memory, memory→peripheral, memory→memory; CNDTR counts down across batches; transfer-complete IRQs; **pump runs fully in Rust** (`rustcpu_dma_pump()` builds the op plan and executes it against Rust RAM with zero JS crossings; completion signaled last so TC IRQs fire after the data lands). DMA1@0x40020000 / DMA2@0x40020400 on both the builtin map and SVD maps (dual-map bug class removed 2026-08-11). **Real-HW semantics (2026-08-13)**: ISR completion flags sit at channel N's real nibble (TCIF_N = `(N-1)*4+1` — was off by one channel, CH4 lit bit 16 instead of 13); DIR follows CMSIS (DIR=1 → push CMAR→CPAR, DIR=0 → absorb CPAR→CMAR, M2M → CPAR→CMAR memcpy per RM0008 — was inverted, so a mem→periph channel silently absorbed from the peripheral address); 8-bit SPI data needs PSIZE/MSIZE=00 (16-bit pushes `max(psize,msize)*ndtr` = 2× the wire bytes, since the F103 SPI clocks only 8 bits per 16-bit DR write). **Circular mode**: CCR CIRC reloads CNDTR from the EN-armed count on every completion (EN stays set) with HTIF+TCIF set per pass; IFCR clears both. |

## Serial buses

| Unit | Level | Notes |
|---|---|---|
| USART1–3 | Full | TXE/TC with **baud-rate byte pacing** (byte_time = 8M/baud instructions), RXNE + RX interrupt, overrun, HDSEL half-duplex loopback (CR3 bit 3; loopback works for self-test), IrDA IREN/IRLP stored (pulse shaping invisible — transfers verified byte-identical), smartcard SCEN/NACK/GTPR stored, synchronous CLKEN transfers verified byte-identical (no START/STOP framing exists at this abstraction), `uart_rx_byte()` injection, `uart_rx_pending()` gate, `get_uart_output()` capture of all transmitted text. |
| SPI1–3 (all on the builtin map) | Full | Master mode with 8/16-bit frames, CPOL/CPHA, bit-order; NSS hardware output (CR2 SSOE drives PA4/PB12/PA15 low while SPE, released on disable; slave NSS input/MODF multimaster-only, not modeled); TI frame format (CR2 FRF) decoded — NSS pulses per frame, CPOL/CPHA don't-care, identical shift data; device selection via GPIO CS; serves external devices (flash/OLED/LCD/touchscreen); TX/RX FIFO behaviour. I2S registers decode but audio output is simulated (`generate_i2s_audio`). |
| I2C1–2 | Full | Master state machine (START, address 7-bit, TX/RX, STOP, repeated START; 10-bit headers NACK — no 10-bit peers), SR1/SR2 status flags, error handling (AF, BER), interrupts (EV/ER). Slave mode: host-driven via `i2c_inject_start/write/read/stop` (OAR1 7/10-bit + OAR2 + general call, ADDR/STOPF sequences, RXNE/TXE + EV IRQs, stretch-equivalent NACK/None when not ready). SMBus ALERT: SR1 SMBALERT via `i2c_inject_alert` (write-0-clears, ER IRQ via ITERREN), CR1 ALERT drive edges as `I2cAlert` events. |
| CAN1 | Full | Mailboxes (TIR/TDTR/TDLR/TDHR), shared filter bank (CAN1 owns all 28; CAN2 borrows [CAN2SB..28] with no filter regs of its own — silicon layout), TX request + TX-complete IRQ, RX FIFO + RX IRQ, `can_inject_message()` to inject a received frame from JS. Silent mode (SILM, incl. silent-loopback self-test) completes identically without bus disturbance. |
| SDIO | Full | SD host @ 0x40018000, IRQ 49: CMD0/1/2/3/6/7/8/9/12/13/16/17/18/24/25/32/33/35/36/38/55 + ACMD41 (busy-first power-up), 32-word FIFO, DATAEND/DBCKEND/CMDREND/CMDSENT/CTIMEOUT + MASK-gated IRQ, DMA2 CH4 requests. SD mode (CMD55+ACMD41, CSD v2) and MMC mode (CMD1, EXT_CSD with sector count) share the block R/W path; SDSC cards (ACMD41 HCS=0) use byte addressing (ARG/blocklen), CSD v1 and CCS-clear OCR; erase commands fill 0xFF. Backed by an `SdCard` image (`add_sd_card('SDIO', data)`); CSD capacity derived from image size. |
| USB | Full | FS device @ 0x40005C00 + 1024 B packet-memory window @ 0x40006000 (byte-exact sub-word access, ST layout: 16 B BTABLE stride/endpoint — DESC0 ADDR/CNT @ +0/+4, DESC1 @ +8/+12; single-buffered TX=DESC0/RX=DESC1, double-buffered ping-pongs on DTOG; buffer addresses are PMA words with `PMA_ACCESS = 2` spread): EP0-7R with real toggle semantics (STAT toggle-on-1, CTR clear-on-0, DTOG read-only), CNTR masks, ISTR (event flags W0C; CTR/DIR/EP_ID derived like hardware — lowest endpoint wins), DADDR, BTABLE. USB RESET on FRES release, SETUP/OUT injection (`usb_inject_setup/out`; OUT NAKs unless VALID, SETUP always ACKed like silicon, DTOG sequencing), IN completion drained as `UsbIn` event + IRQ20. SOF engine (1 ms frames, FNR + RXDP, SOF/SUSP/WKUP IRQs, wakeup IRQ42; SOF is bus activity so an attached host never idles into suspend — suspend is FSUSP-forced with RESUME/FSUSP-clear recovery), double-buffered bulk endpoints (DTOG-selected DESC0/DESC1, stay VALID across first fill). Isochronous: no STALL, CTR pends HP vector 19 (shared with CAN1 TX; LP 20 covers the rest). DADDR hardware address filter (optional `addr` on `usb_inject_setup/out`, absent = correctly-addressed host). PDWN gates the macro (no RX/TX/IRQs/SOF; FRES release alone is NOT a bus reset — only `usb_bus_reset()` raises RESET). Detach API (`usb_detach()`: tokens stop, IN never completes, SOF freezes, FNR RXDP clears; reset reattaches). ESOF raises on detach (missed host SOFs), mask-gated. Demos: `tests/arduino_usb_cdc/` register-level CDC-ACM device (EP0 control + EP1 bulk echo, 22-check `tests/test_usb_cdc.mjs`); `tests/arduino_usb_serial/` real STM32duino USBSerial stack — full host enumeration + CDC class + bulk echo proven 11/11 (`tests/test_usb_serial.mjs`; Arduino CDC uses EP1-OUT/EP2-IN/EP3-CMD); `tests/arduino_dfu/` Maple-style DFU bootloader (EP0 DNLOAD/UPLOAD/GETSTATUS/GETSTATE/CLRSTATUS/ABORT, SetAddressPointer, manifest, real flash unlock/program sequence, 51-check `tests/test_dfu.mjs`). F103-only (`0x40005C00`). |
| USB OTG FS | Full (device + host) | Synopsys OTG_FS core @ 0x50000000 + DFIFO0-3 @ 0x50001000+EP*0x1000, IRQ 67 (F105 map only): GOTGCTL/GOTGINT, GAHBCFG (GINT gate), GUSBCFG, GRSTCTL (CSFTRST/RXFFLSH/TXFFLSH+TXFNUM, AHBIDL=1; CSFTRST preserves a physically attached device — silicon keeps the PHY, otherwise a pre-boot attach boots into an E0 spin), GINTSTS/GINTMSK (W1C, RXFLVL/NPTXFE/CMOD derived, level IRQ), GRXSTSR/GRXSTSP (real status queue), FIFO carve regs (stored), GNPTXSTS/DTXFSTS (generous), GCCFG (PWRDWN gates), CID, DCFG (DAD filter), DCTL (RWUSIG/SDIS/SGONAK/CGONAK), DSTS (SUSPSTS/ENUMSPD/FNSOF), DIEPMSK/DOEPMSK + DAINT + DAINTMSK into IEPINT/OEPINT, EP0-3 DIEPCTL/DOEPCTL (EPENA/SNAK/CNAK/STALL/NAKSTS/USBAEP, EPDIS events), DIEPINT/DOEPINT (W1C XFRC/EPDISD/STUP), DIEPTSIZ/DOEPTSIZ (XFRSIZ/PKTCNT/STUPCNT), PCGCCTL, SOF engine, suspend/resume, `otg_bus_reset` (USBRST+ENUMDNE) / `otg_detach` / `otg_inject_setup/out` (+JS + `.d.ts`). Synchronous completion like the FS model, except IN completes when pushed FIFO bytes reach XFRSIZ (ST stages EPENA before pushing data). Host mode: 8 channels (HCCHAR/HCSPLT/HCINT/HCINTMSK/HCTSIZ, CHENA-edge arming, CHDIS halt with CHHLT), HCFG/HFIR/HFNUM/HPTXSTS/HAINT/HAINTMSK/HPRT (attach/detach/PPWR/PRST/PENA/PCDET), RXFIFO + GRXSTSP shared with device mode,   `HostTx` (disc 20) / `HostRx` (disc 21) events,
  `otg_host_feed_in` / `otg_host_attach` (+JS + `.d.ts`); received statuses
  carry the byte count, transfer-completed statuses carry BCNT 0 (drain on
  received, size from HCTSIZ remaining — the device-side DONE keeps full
  BCNT, which the CDC echo sizes from); no-DMA-engine registers read 0; TXFE never fires (FIFOs never fill). Demos: `tests/otg_cdc/` bare-metal CDC-ACM device (EP1 bulk echo, `tests/test_otg_cdc.mjs` 23/23 + page `otg_cdc` preset); `tests/otg_host/` bare-metal HCD (control + bulk echo, `tests/test_otg_host.mjs` 5/5 + page `otg_host` preset); `tests/test_otg.mjs` 119/119. F105-map only; the FS device above stays F103-only. Firmware notes: EPnR writes must set the opposite direction's CTR to 1 (write-1-no-effect) or a just-raised completion flag is cleared before its branch runs — this wedged multi-packet IN transfers. Polled IN loops must drain-first AND recheck after XFRC (a batch/interrupt boundary between the GRXSTSP load and its use otherwise exits with got=0 — proven by the HCD bring-up); byte counts come from HCTSIZ.XFRSIZ remaining, PKTSTS gates the drain. |

## Timers

| Unit | Level | Notes |
|---|---|---|
| TIM1–7 | Full | PSC/ARR/CNT with instruction-delta advance (no `ticks.min()` cap — ALL accumulated ticks processed per batch), PWM1/2 output compare (duty exposed via `pwm_duty()`, narrowed by BDTR dead-time on TIM1/TIM8 complementary channels), input capture, update events, UIE/CCIE interrupts, CCR1–4, DMA burst (DCR DBA/DBL window — each DMAR write lands in the next window register and wraps; DCR reprogram restarts). TIM6/7 basic timers included. |
| RTC | Full | Calendar registers, **alarm with IRQ** (custom `RTC_IRQHandler` works), BKP interface. |
| DAC | Full | DHR/DOR registers, output value readback. |

## Analog

| Unit | Level | Notes |
|---|---|---|
| ADC1–2 | Full | Real conversion state machine: per-sequence channels (SQR/JSQR), sample-time timing (`Tconv = SMP + 12.5` cycles, 1 instr = 1 cycle), EOC/STRT/JEOC/JSTRT flags, EOCS, AWD with HTR/LTR + AWDIE, CONT auto-restart, discontinuous chunks (DISCEN/DISCNUM, resume per trigger), JAUTO injected-after-regular, ADC1→DMA1 ch1 / ADC2→DMA1 ch2 requests. SQR1 length field corrected ([23:20]; multi-channel regular sequences previously ran as length-1). **Sources**: `adc_set_sim_value()` serves exact readings (legacy), `gpioSetAnalog()` wires a 12-bit pin voltage, `adcSetInternal(ch, v)` drives temp/VREFINT/VBAT (65535 clears to nominal), and enabled DAC channels drive their output pins (DAC1→PA4/ch4, DAC2→PA5/ch5) — real sources sample through an RC sample-and-hold (`adcSetRcTau`). **External triggers**: EXTTRIG/JEXTTRIG with EXTSEL/JEXTSEL sources TIM1_CC1/2/3, TIM1_TRGO, TIM2_CC2, TIM3_TRGO, TIM4_CC4 (+ injected TIM1_CC4/TIM2_TRGO/TIM2_CC2/TIM3_CC4/TIM4_TRGO) emitted from timer update (MMS=update) and compare events, and EXTI lines 11 (regular) / 15 (injected); a new conversion starts when a trigger arrives (ignored while busy). |
| DAC | Full | DHR12/8 (L/R/D) registers, dual channels, output registers DOR1/2 driven on writes. Enabled channels drive a 12-bit analog wire on their pins (F103: DAC1→PA4, DAC2→PA5) that ADC channels mapped to those pins sample via the RC path. |
| CRC | Full | CRC32 computation over written bytes, DR readback. |

## External devices (emulated bus devices)

These are *extra* peripherals the STM32 talks to over SPI/I2C — the "rest of the board".

| Device | Bus | Level | Notes |
|---|---|---|---|
| SPI NOR flash (e.g. W25Q) | SPI1/2 | Full | JEDEC ID (RDID/0x9F), manufacturer/device ID (0x90), status regs (WEL), page program, sector/subsector/bulk erase, continuous read, fast read (dummy byte), CS-gated command state machine. Backed by a JS `Uint8Array` file image. |
| EEPROM (e.g. 24Cxx) | I2C1/2 | Full | 7-bit addressing, byte/sequential read & write, page behavior, repeated START, address counter. File-backed. |
| OLED (e.g. SSD1306-style 128×64) | I2C | Full | Command + display-data state machine with a framebuffer; `add_i2c_oled` configures size. Framebuffer readback via `i2c_oled_fb('I2C1', 0x3C)` (128×64 bytes, 8 vertical pixels/byte) for page-side canvas rendering. |
| LCD (e.g. SPI TFT) | SPI | Partial | Framing protocol (`0xFB` session start at pixel 0 with no arg byte, `0xFC` end, neither stored; resync-safe, saturating cursor); pixel rendering is not provided — the 128×64 byte-per-pixel framebuffer is exposed via `lcd_fb('SPI1')` and the page renders it. |
| Resistive touchscreen (ADS7846) | SPI | Full | Command decoding (channels incl. pressure 0x94), 8/12-bit modes, **deferred reply** (reply arrives on the SPI transfer *after* the command, like the real part), touch injection via `touchscreen_set_touch()` + touch-detect GPIO line. |
| Software SPI (bit-banged GPIO) | GPIO | Full | `add_software_spi()`: CS/CLK/MISO/MOSI pins, emulated on GPIO transitions. |
| SD card (SDHC) + eMMC (block layer) | SDIO | Full | Block-addressed image (512 B sectors); CID/CSD/OCR/RCA derived deterministically, CSD capacity + EXT_CSD sector count from image size. File-backed via `add_sd_card('SDIO', data)`. |

## What is NOT emulated

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

- **372 unit tests** (`node tests/test_all.mjs`) — GPIO (incl. electrical model: pull-ups,
  open-drain, external-driver precedence, slew readback, pin-change events), USART, ADC (real conversion
  timing, RC sample-and-hold via gpioSetAnalog, DAC→ADC loopback, AWD IRQ, TIM1 TRGO /
  TIM1_CC1 / EXTI 11 external triggers), RCC, SysTick, TIM, IWDG, NVIC, CRC, SPI, I2C,
  RTC, PWR, FLASH, CAN, DMA, AFIO, EXTI, BKP, DAC, TIM6, RTC Alarm, UART RX, FSMC
  (MBKEN/WREN gating, byte/word access), deep-sleep gating (TIM frozen, RTC alive, resume
  without catch-up), fault escalation (CFSR/HFSR/BFAR, BusFault vs HardFault, IBUSERR,
  SHPR routing), SDIO (CMD0/8/55/41/2/3/7/9/16/17/24 init + block R/W, IRQ49, DMA2 CH4
  pump, no-card timeouts, F103-SVD registration).
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
