# Supported Boards & Chips

One peripheral map covers the whole family: GD32F103 is register-identical
to STM32F103 at everything modeled, so variants differ only in flash/RAM
sizes and the DBGMCU IDCODE (`pkg/emulator.js` CHIPS table — no SVD needed;
see `docs/USAGE.md`). Timing stays instruction-budget based on every chip.

| Board / chip | `chip` name | Flash/RAM | IDCODE | Notes |
|---|---|---|---|---|
| Blue Pill STM32F103C8 (default) | `stm32f103c8` | 64K/20K | `0x10016410` | reference target, 39/39 suite |
| Blue Pill STM32F103CB | `stm32f103cb` | 128K/20K | `0x10016410` | same map, bigger flash |
| Maple Mini | `maple_mini` | 128K/20K | `0x10016410` | own Arduino header map (LED D33=PB1); offset-vector boot (`vector_table`) for DFU layouts verified |
| Nucleo-F103RB | `nucleo_f103rb` | 128K/20K | `0x10016410` | Arduino headers D0-D51/A0-A8 (LED D13=PA5); `Serial` = USART2 |
| Generic F103RC (SKR Mini class) | `stm32f103rc` | 256K/48K | `0x10016410` | room for large firmware; printer hardware (TMC/TFT/heaters) NOT modeled |
| GD32F103C8 | `gd32f103c8` | 64K/20K | `0x2BA01477` | runs identical F103 binaries (clone contract) |
| GD32F103CB | `gd32f103cb` | 128K/20K | `0x2BA01477` | |
| GD32F103RB | `gd32f103rb` | 128K/20K | `0x2BA01477` | |
| STM32F105 (connectivity) | `{ name, svd }` | 256K/64K | `0x10016418` | separate SVD map: CAN2@0x40006800 |

## Per-board notes (audited)

Silicon peripheral set vs emulator coverage, wiring, demos and quirks —
one page per board:

- [Blue Pill](boards/blue-pill.md) — reference target, 29 presets
- [GD32F103C8](boards/gd32f103c8.md) — clone contract, 29 presets
- [Maple Mini](boards/maple-mini.md) — D33 LED, USB-DFU note, 29 presets
- [Nucleo-F103RB](boards/nucleo-f103rb.md) — USART2 Serial, Arduino headers, 29 presets
- [Generic F103RC](boards/f103rc.md) — high-density set (DAC/FSMC/ADC3/SDIO/UART4-5/SPI3), 30 presets
- [STM32F105](boards/f105.md) — CAN2 + OTG notes, SVD map, 26 presets

The page shows the live chip (`label · ID … · flash/RAM`) in the stats bar
after every load, and the GPIO grid renders Arduino aliases from
`site/board_pins.json` (extracted from the STM32duino variant files).

Each board has runnable demo firmware (`tests/arduino_board_demo/`, one
sketch compiled per FQBN, shipped as `site/arduino_board_*.elf`) with a
matching page preset that sets chip + ELF together
(`tests/test_board_demo.mjs` 8/8, browser presets, CI). The same ×4 pattern
covers UART echo (`arduino_board_echo`, native Serial per board),
the 7-peripheral showcase (`arduino_hw_showcase` + board banner/LED) and
the RTC clock (`arduino_rtc_clock` + board banner) — `test_board_echo.mjs`
8/8, `test_board_showcase.mjs` 12/12, `test_board_rtc.mjs` 20/20. High-density-only
peripherals get their own desk on the RC (`arduino_hd_fsmc`: FSMC NOR +
dual-DAC loopback, `test_hd_fsmc.mjs` 5/5); dual-CAN lives on the F105
(`arduino_can_dual`, `test_can_dual.mjs` 5/5).

The demo page filters the preset menu to the selected chip (board-only
demos hide on other chips; picking one auto-switches the chip), and the
rig SVG redraws per board (chip marking, LED pin, header pins). The UART
input box follows the board's native Serial port (`uartAddr`: USART2
`0x40004400` on Nucleo/RC).

## Verification matrix

- Bluepill-targeted periph39 firmware: **39/39 on all six F103-map chips**
  (f103c8, gd32c8/cb, maple_mini, nucleo_f103rb, f103rc).
- Real arduino-cli firmware per target boots + echoes: MAPLEMINI_F103CB on
  `maple_mini` (USART1); GENERIC_F103RCTX on `stm32f103rc` and NUCLEO_F103RB
  on `nucleo_f103rb` (USART2 via `uart_addr`).
- Offset-linked firmware (`flash_offset=0x5000`, DFU layout) boots + echoes
  on `maple_mini` with `vector_table: 0x08005000`.
- `tests/test_chips.mjs`: IDCODE per chip + GD32 UART echo (CI).

## Deliberately out of scope

- Maple DFU uploader / STLink probe emulation (flashing transports and
  debugger hardware — firmware loads directly; offset boot above covers
  the DFU-layout case).
- GD32 flash-timing / UID quirks (no observable surface at this level).
- Printer hardware for SKR-class boards (stepper drivers, TFT, heaters).
