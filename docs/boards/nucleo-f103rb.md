# Nucleo-F103RB — support notes

ST Nucleo-64 board with medium-density STM32F103RB (128K flash, 20K RAM,
LQFP64): same peripheral set as the [Blue Pill](blue-pill.md).
DBGMCU IDCODE `0x10016410`. Arduino header aliases (`D0`–`D51`/`A0`–`A8`)
live in `site/board_pins.json` (`nucleo_f103rb` key).

## Board wiring (emulator rig)

- LED: **PA5** (Arduino D13 / green LD2, `LED_BUILTIN` = `LED_GREEN`)
- Button: **PC13** (blue B1, `USER_BTN`)
- `Serial` = **USART2** (ST-Link VCP) — banner and echo traffic leave on
  USART2; the page merges all USARTs into one terminal, and the input box
  follows via `uartAddr`, so typing still echoes
- ST-Link USB (debug probe hardware is out of scope — use the
  [GDB stub](../GDB.md) instead)

## Demos on this chip (29 presets)

All 25 shared presets plus the Nucleo builds: `board_nucleo`,
`board_nucleo_echo`, `board_nucleo_showcase`, `board_nucleo_rtc`.
(USB/OTG presets show on every chip but only function where the
peripheral is mapped — see the matrix.)

## Verification

- periph39 firmware: **39/39** on `nucleo_f103rb`
- Real `NUCLEO_F103RB` arduino-cli firmware boots + USART2-echoes
- `tests/test_board_demo.mjs`, `test_board_echo.mjs`,
  `test_board_showcase.mjs`, `test_board_rtc.mjs` green on `nucleo_f103rb`

## Feature matrix (silicon vs emulator)

Silicon = ST Nucleo-64 (STM32F103RB, medium-density, 128K flash, 20K
RAM, LQFP64) — same peripheral set as the [Blue Pill](blue-pill.md).
Emulator = this project's model on the builtin F103 map. Statuses:
**Full** = register-level incl. IRQs/DMA/events; **Partial** = subset
(see remark); **Gap** = reads zero/lenient, no consumer; **Absent** =
correctly unmapped.
Other labels (Driver/IDCODE only, Out of scope) as marked.
Depth: [PERIPHERALS](../PERIPHERALS.md).

| Feature | Silicon | Emulator | Remark |
|---|---|---|---|
| Cortex-M3 Thumb-2 (UDIV/SDIV, LDREX/STREX, TBB/TBH, unaligned) | Yes | Full | differential-fuzzed vs Unicorn oracle, 0 divergences |
| MPU (8 regions, subregions, AP/XN) | Yes | Full | enforced live on every access |
| Faults (MemManage/BusFault/UsageFault→HardFault, CFSR/BFAR/HFSR) | Yes | Full | SHCSR-gated escalation |
| SVC / PendSV, SHPR priorities, EXC_RETURN, PSP | Yes | Full | mini_rtos proof (preemptive PSP tasks) |
| SysTick (24-bit, debt drain) | Yes | Full | phase-exact multi-period re-pend |
| DWT CYCCNT | Yes | Partial | CYCCNT +1/latency from FLASH ACR; data watchpoints live in the Debug slice (own row); ETM/TPIU absent (ITM stimulus is its own row) |
| ITM stimulus port 0 | Yes | Full | TER+TCR-gated printf bytes as `ItmByte` events |
| Debug (SWD/JTAG, ETM trace) | Yes | Full | DBGMCU `0x10016410`; SWD DP + MEM-AP + DHCSR/DCRSR/DEMCR + 4 data watchpoints + JTAG TAP (transaction-level `swd_*` API, GDB Z0/Z2/Z3/Z4); pin/clock edges + ETM out of scope |
| ST-Link VCP / debug probe | Yes (on-board) | Out of scope | probe hardware not emulated; `Serial` traffic still flows on USART2 |
| RCC (HSI/HSE/PLL ×2–16, prescalers, CSS) | Yes | Full | clocks queryable (`rcc_clocks_hz`); MCO selection queryable (`rcc_mco_hz`, pin wave out of scope) |
| FLASH 128K (program/erase, WRPRTERR) | Yes | Full | OBR USER settable; WDG_SW clear runs IWDG from reset |
| PWR (PVD, Sleep/Stop/Standby) | Yes | Full | PVD PLS thresholds vs settable supply (`pwr_set_supply_mv`) →EXTI16; SLEEPDEEP freezes timers (RTC+IWDG keep running); standby wakes on WKUP/RTC only |
| BKP (10 regs, tamper, RTC cal) | Yes | Full | TPE/TPAL edges + IRQ, W1C |
| RTC (second/alarm/overflow, PRL) | Yes | Full | 1 Hz PRL model; LSE/LSI assumed running |
| CRC-32 | Yes | Full | known-answer vectors |
| IWDG | Yes | Full | reset-request path |
| WWDG + EWI | Yes | Full | early-wakeup IRQ proven |
| 96-bit UID @0x1FFFF7E8 | Yes | Full | fixed constant serial, writes ignored |
| GPIOA–D (+Arduino D0–D51/A0–A8) | Yes | Full | electrical model; aliases in `board_pins.json` (`nucleo_f103rb` key); LCKR lock sequence freezes nibbles |
| GPIOE–G | E partial (64-pin) | Full (A–E) | superset for the missing pins |
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
| LED / button / Serial | PA5=D13 / PC13 / USART2 VCP | Full | input box follows via `uartAddr`; page merges all USARTs into one terminal |
