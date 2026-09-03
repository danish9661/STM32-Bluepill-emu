# Coverage Audit — Emulator vs Real STM32F103 Silicon

Method: census of all 67 `<peripheral>` blocks in `svd/STM32F103.svd`
(including `derivedFrom` entries), cross-checked against `build_peripheral()`
+ the hardcoded map in `src/peripherals/mod.rs` and the depth notes in
`docs/PERIPHERALS.md`. "Family" = what the SVD describes (medium density and
up); the Blue Pill's STM32F103C8 is a medium-density subset — peripherals the
C8 lacks but we model anyway (DAC, FSMC, DMA2, ADC3, TIM5+, UART4/5, CAN2,
SPI3, GPIOE-G) are harmless supersets, marked *(S)* below.

## 1. Peripheral census (SVD → emulator status)

| SVD peripheral(s) | Base(s) | Status | Notes |
|---|---|---|---|
| TIM2/3/4/5/6/7 | APB1 | Full | PWM, input capture, TRGO Triggers, slave modes, DMA requests |
| TIM1/8 | APB2 | Full | + RCR/BDTR registers decode |
| TIM9/10/11 | APB2 | Partial | IRQ + channels work; `timer_base()` lacks 9/10/11, so TRGO fan-out from them is degraded |
| TIM12/13/14 | APB1 | Full (S) | Pinned in `timer_base()` |
| RTC | 0x40002800 | Full | Time + **alarm**; second IRQ and tamper not modeled |
| WWDG | 0x40002C00 | Partial | Counter + reset; early-wakeup interrupt (EWI) not modeled |
| IWDG | 0x40003000 | Full | Down-counter + reset; runs through STOP/STANDBY |
| USART1-3, UART4/5 | APB1/2 | Full | Byte-time pacing, RXNE/TXE IRQs, DMA channels (incl. DMA2 for UART4/5) |
| I2C1/2 | APB1 | Full | Master TX/RX state machine; **no 10-bit addressing, no slave mode** (OAR registers store only) |
| USB (FS device) | 0x40005C00 | **Stub** | Reads 0 / writes ignored — the single biggest peripheral gap (see §3) |
| CAN1 (+CAN2 via F105 SVD) | APB1 | Full | Mailboxes, ID-list + mask filters, TX/RX IRQs, RX injection |
| BKP | 0x40006C00 | Full | Backup registers + tamper |
| PWR | 0x40007000 | Partial | Modes + STOP/STANDBY gating; **PVD (voltage detector) not modeled** |
| DAC | 0x40007400 | Full (S) | Both channels, DMA, →ADC loopback wire |
| AFIO / EXTI | APB2 | Full | Remap, 20 lines, SWIER, GPIO-edge fan-in |
| GPIOA-G | APB2 | Full | Electrical model (pull/open-drain/slew), pin events; E-G reachable via SVD (8-port backing) |
| ADC1/2 (+ADC3 S) | APB2 | Full | Real conversion timing, AWD, injected, EXTSEL triggers; **no dual (multi-ADC) mode** |
| SPI1/2 (+SPI3 S) | APB1/2 | Full | Master 8/16-bit, CPOL/CPHA, CRC registers store-only, I2S decodes |
| SDIO | 0x40018000 | Full | SDHC card model (see §4); **no MMC/eMMC identification** |
| DMA1 (7ch) + DMA2 (5ch, S) | AHB | Full | All directions, global completion streams 0-11, plan-based pump |
| RCC | 0x40021000 | Partial | All enable/reset bits; clock TREE not modeled (fixed 8 MHz instruction budget — PLL/HSE/CSS accepted, no effect) |
| FLASH | 0x40022000 | Full | Unlock/program/erase, option bytes, status |
| CRC | 0x40023000 | Full | |
| FSMC | 0xA0000000 | Full (S) | 7 banks, MBKEN/WREN, all widths (no NAND ECC computation) |
| NVIC / STK / SCB | 0xE000Exxx | Full | Priority dispatch, SysTick debt, SHPR/SHCSR, faults, deep sleep |
| SCB_ACTRL | 0xE000E008 | Missing | Aux control (DISMCYCINT/DISFOLD — cycle-count subtilities only) |
| NVIC_STIR | 0xE000EF00 | Missing | Software-triggered IRQs (RTOS test code sometimes uses it) |
| MPU | 0xE000ED90 | Missing | Intentional: MPU-off firmware (the default) is unaffected |
| DBG / DBGMCU | 0xE0042000 | Missing | Intentional: debug/trace has no headless meaning |
| ETHERNET_MAC/MMC/PTP/DMA | 0x40028xxx | Skipped | Correct: no F1 silicon has Ethernet (ST SVD quirk) |
| OTG_FS_* | 0x50000xxx | Skipped | Correct: F103 has FS-device USB only, no OTG (SVD quirk) |

Score: of ~40 real peripherals, **33 Full, 5 Partial, 1 Stub (USB), 4 intentionally missing/skipped**.

## 2. Depth gaps inside "Full" peripherals (all minor)

- TIM: DMA-burst (DMAR/DCR store only), complementary dead-time shaping.
- ADC: dual simultaneous mode; channels 16-18 nominal internal values.
- SPI: CRC values not computed (registers store); TI frame format.
- I2C: SMBus/PEC, general-call responses.
- USART: LIN/IrDA/smartcard modes (registers decode).
- CAN: time-triggered (TTCM bit accepted).
- FSMC: NAND ECC bytes not computed; fixed access timing.
- RTC: second tick IRQ, tamper pin.
- FLASH: write-protection enforcement (unlock model is permissive, like RCC gating).
- GPIO: C8 exposes only A/B/C.13-15/D.0-1 — full ports are a superset.
- RCC/PWR: no PVD, no CSS failure injection, no stop-mode clock switch.

None of these affect the 39/39 firmware suite or any shipped demo; they matter
only to firmware that specifically exercises them (which then sees lenient
reads instead of faults — the emulator's standing philosophy).

## 3. USB — the one real gap

`src/peripherals/usb.rs` is a 15-line stub: every register reads 0. Any USB
firmware (STM32duino USB-CDC serial, custom HID) silently sees "no USB clock /
no events" and spins or drops the interface. A real model needs: CNTR/ISTR
event flags + IRQ, endpoint registers EP0-7R with DTOG bits, the 512 B packet
memory (BTABLE), reset/enumeration state machine, and SETUP/IN/OUT transaction
pumps with a JS-side byte transport (the `drain_events()` queue pattern used
by every other bus). Estimate: the largest single peripheral left, on the
order of the SDIO work × 2–3 (control transfers + descriptors + class drivers
are a protocol stack, not just registers).

## 4. Storage: SD card vs eMMC (status answer)

The SDIO host speaks the **SD protocol completely for block I/O**: CMD0/8/55/
ACMD41 init with busy polling, SDHC CSD v2.0 capacity, CMD17/18/24/25 data at
any block, DMA + polled paths. Any FAT stack (SdFat, FatFs over STM32SD) that
talks SD will work, at any image size.

**eMMC/MMC is not modeled.** Concretely missing: CMD1 SEND_OP_COND (MMC uses
it instead of CMD55+ACMD41), OCR semantics without HCS/CCS negotiation,
EXT_CSD (the MMC CMD8 is a 512-byte register read, unrelated to SD's CMD8
voltage check), CID/CSD layout differences, 8-bit WIDBUS, erase-group/TRIM
commands (CMD32/33/35/36/38 are absent on the SD path too), RPMB and boot
partitions (out of scope regardless). A card strapped as eMMC would fail at
identification (no response to CMD1 → timeout) and never reach block I/O.

Practical note: eMMC on a Blue Pill is essentially nonexistent (BGA153/169
package, 8 data lines, 1.8/3.3 V rails) — every real Blue Pill storage project
uses SD over SPI or SDIO. If an eMMC firmware ever shows up, the cheap bridge
is a CMD1 → OCR-ready fallback (~30 lines) treating the image as block storage;
full EXT_CSD/boot/RPMB fidelity is not worth building without a consumer.

## 5. Test coverage of the above

- `tests/test_all.mjs` (277): SDIO init/R/W/IRQ/DMA/no-card/SVD; DMA global
  streams; everything in §1 marked Full has a group.
- `tests/canary.mjs` + 200M runs (both paths): 39/39 real-firmware checks.
- Deliberately untested: USB (stub), MPU/DBG/STIR/ACTRL (absent), eMMC path.
