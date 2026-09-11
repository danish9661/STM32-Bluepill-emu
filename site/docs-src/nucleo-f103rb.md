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

## Demos on this chip (25 presets)

All 21 portable demos plus the Nucleo builds: `board_nucleo`,
`board_nucleo_echo`, `board_nucleo_showcase`, `board_nucleo_rtc`.

## Verification

- periph39 firmware: **39/39** on `nucleo_f103rb`
- Real `NUCLEO_F103RB` arduino-cli firmware boots + USART2-echoes
- `tests/test_board_demo.mjs`, `test_board_echo.mjs`,
  `test_board_showcase.mjs`, `test_board_rtc.mjs` green on `nucleo_f103rb`
