# Blue Pill (STM32F103C8) — support notes

The reference target: medium-density STM32F103C8 (64K flash, 20K RAM,
LQFP48), DBGMCU IDCODE `0x10016410`. Every firmware suite runs here first.

## Silicon peripherals

GPIOA–D, USART1–3, SPI1–2, I2C1–2, CAN1, USB FS device, TIM1–4, ADC1–2,
RTC, BKP, PWR, IWDG, WWDG, CRC, FLASH, AFIO, EXTI, NVIC/SysTick/SCB —
all modeled. **Absent in silicon** (medium density): DAC, FSMC, ADC3,
SDIO, UART4/5, SPI3, TIM5+, CAN2, GPIOE–G. The emulator still models
them (harmless superset); run those demos on [Generic F103RC](f103rc.md)
or [STM32F105](f105.md) where they really exist.

## Board wiring (emulator rig)

- LED: **PC13** (active-low on real boards; `LED_BUILTIN`)
- `Serial` = **USART1** (PA9/PA10) — the page terminal shows it
- 8 MHz crystal, NRST + BOOT0 headers, USB device port

## Demos on this chip (29 presets)

All 24 shared presets (blink, echo, comprehensive, periph37, fade,
timer_uart, pwm_wave, servo, adc_uart, dac_sine, rtc_clock, stopwatch,
flash_demo, showcase, ws2812, i2c_scan, i2c_slave, can_chat, sd_logger,
usb_cdc, usb_serial, otg_cdc, otg_host, dfu, mini_rtos) plus the Blue Pill builds:
`board_pill`, `board_pill_echo`, `board_pill_showcase`, `board_pill_rtc`.
(USB/OTG presets show on every chip but only function where the
peripheral is mapped — see the matrix.)

## Verification

- periph39 firmware: **39/39** (`node tests/canary.mjs`)
- `tests/test_board_demo.mjs`, `test_board_echo.mjs`,
  `test_board_showcase.mjs`, `test_board_rtc.mjs` green on `stm32f103c8`

## Feature matrix (silicon vs emulator)

Silicon = real STM32F103C8 (medium-density, LQFP48). Emulator = this
project's model on the builtin F103 map. Statuses: **Full** =
register-level incl. IRQs/DMA/events; **Partial** = subset (see remark);
**Gap** = reads zero/lenient, no consumer; **Absent** = correctly
unmapped (no such silicon here). Other labels (Driver/IDCODE only, Out of scope,
Assumed) as marked. Depth details: [PERIPHERALS](../PERIPHERALS.md).

| Feature | Silicon | Emulator | Remark |
|---|---|---|---|
| Cortex-M3 Thumb-2 (UDIV/SDIV, LDREX/STREX, TBB/TBH, unaligned) | Yes | Full | differential-fuzzed vs Unicorn oracle, 0 divergences |
| MPU (8 regions, subregions, AP/XN) | Yes | Full | enforced live on every access |
| Faults (MemManage/BusFault/UsageFault→HardFault, CFSR/BFAR/HFSR) | Yes | Full | SHCSR-gated escalation |
| SVC / PendSV, SHPR priorities, EXC_RETURN, PSP | Yes | Full | mini_rtos proof (preemptive PSP tasks) |
| SysTick (24-bit, debt drain) | Yes | Full | phase-exact multi-period re-pend |
| DWT CYCCNT | Yes | Partial | CYCCNT +1/latency from FLASH ACR; data watchpoints live in the Debug slice (own row); ETM/TPIU absent (ITM stimulus is its own row) |
| ITM stimulus port 0 | Yes | Full | TER+TCR-gated printf bytes as `ItmByte` events |
| Debug (SWD/JTAG, ETM trace) | Yes | Full | DBGMCU readout; SWD DP + MEM-AP + DHCSR/DCRSR/DEMCR + 4 data watchpoints + JTAG TAP (transaction-level `swd_*` API, GDB Z0/Z2/Z3/Z4); pin/clock edges + ETM out of scope |
| RCC (HSI/HSE/PLL ×2–16, prescalers, CSS) | Yes | Full | clocks queryable (`rcc_clocks_hz`); MCO selection queryable (`rcc_mco_hz`, pin wave out of scope) |
| FLASH 64K (program/erase, WRPRTERR) | Yes | Full | OBR USER settable; WDG_SW clear runs IWDG from reset |
| PWR (PVD, Sleep/Stop/Standby) | Yes | Full | PVD PLS thresholds vs settable supply (`pwr_set_supply_mv`) →EXTI16; SLEEPDEEP freezes timers (RTC+IWDG keep running); standby wakes on WKUP/RTC only |
| BKP (10 regs, tamper, RTC cal) | Yes | Full | TPE/TPAL edges + IRQ, W1C |
| RTC (second/alarm/overflow, PRL) | Yes | Full | 1 Hz PRL model; LSE/LSI assumed running |
| CRC-32 | Yes | Full | known-answer vectors |
| IWDG | Yes | Full | reset-request path |
| WWDG + EWI | Yes | Full | early-wakeup IRQ proven |
| 96-bit UID @0x1FFFF7E8 | Yes | Full | fixed constant serial, writes ignored |
| GPIOA–D | Yes | Full | electrical model (pull-up/down, slew, open-drain, analog); LCKR lock sequence freezes nibbles |
| GPIOE–G | No (100-pin+) | Full (A–E) | harmless superset on this 48-pin part |
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
| LED / button / Serial / USB port | PC13 / — / USART1 / device | Full | aliases in `board_pins.json` |
| 8 MHz crystal, NRST + BOOT0 | Yes | Assumed | fixed instruction budget; no clock fault surface |
