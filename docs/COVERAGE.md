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
| TIM9/10/11 | APB2 | Full | IRQ + channels + trigger bases (were already present) |
| TIM12/13/14 | APB1 | Full (S) | Pinned in `timer_base()` |
| RTC | 0x40002800 | Full | Time + alarm + second + overflow IRQs, SECF/ALRF/OWF flags (RM0008 CRH bit order) |
| WWDG | 0x40002C00 | Full | Counter + reset + early-wakeup interrupt (EWI → IRQ0) |
| IWDG | 0x40003000 | Full | Down-counter + reset; runs through STOP/STANDBY |
| USART1-3, UART4/5 | APB1/2 | Full | Byte-time pacing, RXNE/TXE IRQs, DMA channels (incl. DMA2 for UART4/5) |
| I2C1/2 | APB1 | Full | Master TX/RX state machine; **no 10-bit addressing, no slave mode** (OAR registers store only) |
| USB (FS device) | 0x40005C00 | Full | EP0-7R toggle semantics, CNTR masks, ISTR (W0C flags; CTR/DIR/EP_ID derived), DADDR, BTABLE, 512 B PMA (byte-exact), RESET on FRES release, SETUP/OUT injection with DTOG sequencing, IN completion as `UsbIn` event + IRQ20. No SOF engine, suspend/resume, wakeup IRQ42, double-buffered endpoints |
| CAN1 (+CAN2 via F105 SVD) | APB1 | Full | Mailboxes, ID-list + mask filters, TX/RX IRQs, RX injection |
| BKP | 0x40006C00 | Full | Backup registers (RM0008 map) + tamper pin (TPE/TPAL, IRQ2, DR clear) |
| PWR | 0x40007000 | Full | Modes + STOP/STANDBY gating + PVD (fixed-supply model → EXTI16) |
| DAC | 0x40007400 | Full (S) | Both channels, DMA, →ADC loopback wire |
| AFIO / EXTI | APB2 | Full | Remap, 20 lines, SWIER, GPIO-edge fan-in |
| GPIOA-G | APB2 | Full | Electrical model (pull/open-drain/slew), pin events; E-G reachable via SVD (8-port backing) |
| ADC1/2 (+ADC3 S) | APB2 | Full | Real conversion timing, AWD, injected, EXTSEL triggers; **no dual (multi-ADC) mode** |
| SPI1/2 (+SPI3 S) | APB1/2 | Full | Master 8/16-bit, CPOL/CPHA, CRC registers store-only, I2S decodes |
| SDIO | 0x40018000 | Full | SDHC card model (see §4); **no MMC/eMMC identification** |
| DMA1 (7ch) + DMA2 (5ch, S) | AHB | Full | All directions, global completion streams 0-11, plan-based pump |
| RCC | 0x40021000 | Partial | All enable/reset bits + decoded SYSCLK/HCLK/PCLK query API; wall-clock conversions stay on the fixed 8 MHz instruction budget by decision |
| FLASH | 0x40022000 | Full | Unlock/program/erase, option bytes, status |
| CRC | 0x40023000 | Full | |
| FSMC | 0xA0000000 | Full (S) | 7 banks, MBKEN/WREN, all widths (no NAND ECC computation) |
| NVIC / STK / SCB | 0xE000Exxx | Full | Priority dispatch, SysTick debt, SHPR/SHCSR, faults, deep sleep |
| SCB_ACTRL | 0xE000E008 | Missing | Aux control (DISMCYCINT/DISFOLD — cycle-count subtilities only) |
| NVIC_STIR | 0xE000EF00 | Missing | Software-triggered IRQs (RTOS test code sometimes uses it) |
| MPU | 0xE000ED90 | Full | 8 regions, RNR/VALID/aliases, priority, subregions, AP/XN, background, PPB rules, MMFSR/MMFAR, MemManage/HardFault escalation (see docs/CPU.md) |
| DBG / DBGMCU | 0xE0042000 | Missing | Intentional: debug/trace has no headless meaning |
| ETHERNET_MAC/MMC/PTP/DMA | 0x40028xxx | Skipped | Correct: no F1 silicon has Ethernet (ST SVD quirk) |
| OTG_FS_* | 0x50000xxx | Skipped | Correct: F103 has FS-device USB only, no OTG (SVD quirk) |

Score: of ~40 real peripherals, **38 Full, 1 Partial, 0 Stubs**, 4 intentionally missing/skipped.

The remaining Partial is the RCC clock tree (all enable/reset bits work; the
MHz value behind the fixed instruction budget is now queryable via
`rcc_clocks()`/`rcc_sysclk_hz()`, but wall-clock conversions are not rescaled —
rescaling SysTick/USART to a 72 MHz budget would 9× every delay loop and break
all firmware instruction budgets, so the fixed budget stays by decision). Previous partials closed since the audit: WWDG EWI (was already
implemented — proven by test), PVD + EXTI16, RTC second/overflow + CRL flags
(which also fixed a mirrored CRH bit-order mistake shared with the test
firmware), BKP tamper + RM0008 register map, TIM9–11 bases (were already
present — the audit claim was wrong, caught by the compiler).

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

## 3. USB — closed this sprint

Implemented (`src/peripherals/usb.rs`, ~420 lines): EP0R-EP7R with hardware
toggle semantics, CNTR masks, ISTR event flags + derived CTR/DIR/EP_ID, DADDR,
BTABLE, 512 B packet memory with byte-exact sub-word access (PMA window
exempted from the bus word-lane logic), USB RESET on FRES release,
SETUP/OUT injection with DTOG sequencing and NAK-unless-armed, IN completion
drained as `UsbIn` (discriminant 18) + IRQ20, `onUsbIn` in `STM32F1`.
Still out: SOF engine, suspend/resume, wakeup IRQ42, double-buffered and
isochronous endpoints (treated as bulk).

## 4. Storage: SD card vs eMMC (status answer)

## 4. Storage: SD card vs eMMC (bridged this sprint)

The SDIO host now speaks both identification protocols behind one block
layer. SD mode is unchanged (CMD55+ACMD41, CSD v2.0). MMC mode adds:
CMD1 SEND_OP_COND with no APP latch (busy-first, R3 with sector-access bit),
EXT_CSD register read via CMD8 (revision, card type, sector count from the
image size), and erase commands CMD32/33/35/36/38 (fill 0xFF). The card mode
latches on whichever OP_COND completes first, so real probe order works:
CMD8 pre-init still echoes R7 (SD probing), then CMD1 switches the same card
to MMC. CID/RCA/block R/W are shared; 8-bit WIDBUS needs no modeling (the
FIFO is width-agnostic).

Still out (documented, no consumer): HS200/HS400 speed modes (need 1.8 V +
tuning the F1 lacks), RPMB authenticated access, boot partitions. Practical
note stands: eMMC on a Blue Pill needs an adapter breakout (BGA package) —
every real Blue Pill storage project uses SD.

## 5. Test coverage of the above

- `tests/test_all.mjs` (354): SDIO init/R/W/IRQ/DMA/no-card/SVD; DMA global
  streams; WWDG EWI; PVD edges; RTC second/overflow + flags; RCC clock decode;
  tamper; USB toggles/RESET/control/bulk/IRQ; everything in §1 marked Full
  has a group.
- `tests/canary.mjs` + 200M runs (both paths): 39/39 real-firmware checks.
- Deliberately untested: DBG/STIR/ACTRL (absent), eMMC path, USB SOF/
  suspend/double-buffer paths.
