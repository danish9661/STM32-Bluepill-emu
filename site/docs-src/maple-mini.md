# Maple Mini (STM32F103CB) — support notes

LeafLabs Maple Mini: medium-density STM32F103CB (128K flash, 20K RAM,
LQFP48) — same peripheral set as the [Blue Pill](blue-pill.md), double
the flash. DBGMCU IDCODE `0x10016410`.

## Silicon peripherals

Same as Blue Pill (no DAC/FSMC/ADC3/SDIO/UART4-5/SPI3/TIM5/CAN2).
Arduino aliases for this board live in `site/board_pins.json`
(`maple_mini` key) and render in the page GPIO grid.

## Board wiring (emulator rig)

- LED: **PB1** (Arduino D33, `LED_BUILTIN` = `LED_GREEN`)
- BUT button: **PB8** (physical; the Arduino variant leaves `USER_BTN`
  undefined)
- `Serial` = **USART1** — the page terminal shows it
- Native USB port (Maple-style DFU download works end to end — see the
  `dfu` preset; the factory ROM binary itself is not emulated, and our
  [bootloader](../USAGE.md#system-memory-bootloader-an3155-usart-flow-no-hardware)
  speaks the USART AN3155 flow instead)

## Demos on this chip (29 presets)

All 25 shared presets plus the Maple builds: `board_maple`,
`board_maple_echo`, `board_maple_showcase`, `board_maple_rtc`.
(USB/OTG presets show on every chip but only function where the
peripheral is mapped — see the matrix.)

## Verification

- periph39 firmware: **39/39** on `maple_mini`
- Real `MAPLEMINI_F103CB` arduino-cli firmware boots + USART1-echoes
- `tests/test_board_demo.mjs`, `test_board_echo.mjs`,
  `test_board_showcase.mjs`, `test_board_rtc.mjs` green on `maple_mini`
- DFU-layout offset-vector boot (`vector_table: 0x08005000`) proven on
  `maple_mini`

## Feature matrix (silicon vs emulator)

Silicon = LeafLabs Maple Mini (STM32F103CB, medium-density, 128K flash,
20K RAM, LQFP48) — same peripheral set as the [Blue Pill](blue-pill.md),
double the flash. Emulator = this project's model on the builtin F103
map. Statuses: **Full** = register-level incl. IRQs/DMA/events;
**Partial** = subset (see remark); **Gap** = reads zero/lenient, no
consumer; **Absent** = correctly unmapped. Other labels (Driver/IDCODE only, Out of scope)
as marked. Depth: [PERIPHERALS](../PERIPHERALS.md).

| Feature | Silicon | Emulator | Remark |
|---|---|---|---|
| Cortex-M3 Thumb-2 (UDIV/SDIV, LDREX/STREX, TBB/TBH, unaligned) | Yes | Full | differential-fuzzed vs Unicorn oracle, 0 divergences |
| MPU (8 regions, subregions, AP/XN) | Yes | Full | enforced live on every access |
| Faults (MemManage/BusFault/UsageFault→HardFault, CFSR/BFAR/HFSR) | Yes | Full | SHCSR-gated escalation |
| SVC / PendSV, SHPR priorities, EXC_RETURN, PSP | Yes | Full | mini_rtos proof (preemptive PSP tasks) |
| SysTick (24-bit, debt drain) | Yes | Full | phase-exact multi-period re-pend |
| DWT CYCCNT | Yes | Partial | CYCCNT +1/latency from FLASH ACR; ITM/ETM/TPIU absent |
| ITM stimulus port 0 | Yes | Full | TER+TCR-gated printf bytes as `ItmByte` events |
| Debug (SWD/JTAG, ETM trace) | Yes | IDCODE only | DBGMCU `0x10016410`; use the GDB stub instead |
| USB DFU uploader (factory bootloader) | Yes (ROM) | Full workflow | the ROM binary itself isn't emulated, but the DFU download ritual is real end-to-end: `arduino_dfu` firmware speaks DFU DNLOAD/manifest over EP0 with true flash unlock/program sequencing (`tests/test_dfu.mjs` 51/51), and the page plays host (file or pattern → manifest). Offset-vector boot covers DFU layouts; USART AN3155 bootloader IS modeled |
| RCC (HSI/HSE/PLL ×2–16, prescalers, CSS) | Yes | Full | clocks queryable (`rcc_clocks_hz`); MCO selection queryable (`rcc_mco_hz`, pin wave out of scope) |
| FLASH 128K (program/erase, WRPRTERR) | Yes | Full | OBR USER settable; WDG_SW clear runs IWDG from reset |
| PWR (PVD, Sleep/Stop/Standby) | Yes | Full | PVD PLS thresholds vs settable supply (`pwr_set_supply_mv`) →EXTI16; SLEEPDEEP freezes timers (RTC+IWDG keep running); standby wakes on WKUP/RTC only |
| BKP (10 regs, tamper, RTC cal) | Yes | Full | TPE/TPAL edges + IRQ, W1C |
| RTC (second/alarm/overflow, PRL) | Yes | Full | 1 Hz PRL model; LSE/LSI assumed running |
| CRC-32 | Yes | Full | known-answer vectors |
| IWDG | Yes | Full | reset-request path |
| WWDG + EWI | Yes | Full | early-wakeup IRQ proven |
| 96-bit UID @0x1FFFF7E8 | Yes | Full | fixed constant serial, writes ignored |
| GPIOA–D | Yes | Full | electrical model (pull-up/down, slew, open-drain, analog); LCKR lock sequence freezes nibbles |
| GPIOE–G | No (100-pin+) | Full (A–E) | harmless superset |
| AFIO (remap, EXTI select) | Yes | Full | MAPR incl. TIM2/3/4 + CAN fixes; SWJ_CFG reserves debug pins until released |
| EXTI 0–15, 16/PVD, 17/RTC, 18/USB | Yes | Full | edge IRQs + ADC triggers; no line 19 (no ETH on F1); standby wakes on WKUP/RTC only |
| DMA1 (7 ch, all requests) | Yes | Full | full request matrix (TIM/ADC/DAC/USART/SPI/I2C); circular reload + HTIF per pass |
| DMA2 (5 ch) | No (HD/CL only) | Full | superset: mapped and working here too |
| ADC1/ADC2 (SMP timing, AWD, temp sensor) | Yes | Full | Tconv-accurate, dual mode, injected + ext triggers; discontinuous chunks + JAUTO; SQR length field corrected |
| ADC3 | No (HD/XL) | Full | on the builtin map (HD/CL silicon has it) |
| DAC 2ch + loopback | No (HD only) | Full | superset: PA4/PA5 wires + ADC loopback |
| TIM1 (advanced, BDTR/break) | Yes | Full | MOE/BKIN/LOCK; DTG narrows duty on complementary channels |
| TIM2–4 (GP, capture, DMA burst, slave) | Yes | Full | input capture + AFIO remap, burst window, slave modes |
| TIM5 | No (HD/CL) | Full | on the builtin map (IRQ 50) |
| TIM6/7 (basic) | Yes | Full | update IRQ + DMA requests |
| TIM8–14 | No F1 silicon | Driver only | instantiable for custom maps; out of scope |
| USART1–3 (TX pacing, LIN, errors) | Yes | Full | ORE SR→DR recovery, LIN SBK/LBD |
| UART4/5 | No (HD/CL) | Full | on the builtin map (IRQ 52/53, DMA2) |
| IrDA / smartcard / HDSEL | USARTs have it | Full | transfers verified byte-identical (no START/STOP framing or pulse surface at this abstraction) |
| SPI1/2 (+CRC-8/16) | Yes | Full | CRCNEXT phase + CRCERR; NSS hardware output (SSOE) |
| SPI3 | No (HD/CL) | Full | superset on this chip |
| TI frame format (FRF) | SPIs have it | Full | decoded; data path shared with Motorola by construction (NSS phasing only) |
| I2S audio (via SPI2/3) | Yes | Full | I2SCFGR/I2SPR + audio-gen model |
| I2C1/2 (master/slave/10-bit/PEC/GENCALL) | Yes | Full | slave inject API; stretch-equivalent NACKs |
| SMBus ALERT | Yes | Full | both directions (CR1 drive + inject); ARP out of scope |
| CAN1 (loopback/silent, filters, TTCM) | Yes | Full | TX edge-trigger IRQ, timestamps; silent + silent-loopback complete without bus disturbance |
| CAN2 | No (CL only) | Full | on the builtin map (shares CAN1 banks); F105 proof applies |
| USB FS device (EP0–7, SOF, suspend) | Yes | Full | BTABLE-16/PMA-1K/double-buffer/ISO/HP/PDWN/detach/DADDR; ESOF on detach |
| USB OTG_FS (device + host) | No (CL only) | Full | on the builtin map (same core as F105; 119 unit + HCD e2e) |
| FSMC (NOR/NAND/PC-card) | No (HD 100-pin+) | Full | superset (needs image); MBKEN/WREN + ECC accum |
| SDIO (+MMC, DMA2) | No (HD only) | Full | superset (needs image); SDHC block + SDSC byte addressing (CSD v1) |
| LED / button / Serial | PB1=D33 / PB8 / USART1 | Full | aliases in `board_pins.json` (`maple_mini` key) |
| Native USB port (DFU) | Yes | Full | `dfu` preset: page downloads firmware images into the bootloader live |
