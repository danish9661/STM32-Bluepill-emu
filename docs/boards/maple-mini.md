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
- Native USB port (Maple's USB DFU uploader is *not* emulated — flashing
  transports are out of scope; our [bootloader](../USAGE.md#system-memory-bootloader-an3155-usart-flow-no-hardware)
  speaks the USART AN3155 flow instead)

## Demos on this chip (25 presets)

All 21 portable demos plus the Maple builds: `board_maple`,
`board_maple_echo`, `board_maple_showcase`, `board_maple_rtc`.

## Verification

- periph39 firmware: **39/39** on `maple_mini`
- Real `MAPLEMINI_F103CB` arduino-cli firmware boots + USART1-echoes
- `tests/test_board_demo.mjs`, `test_board_echo.mjs`,
  `test_board_showcase.mjs`, `test_board_rtc.mjs` green on `maple_mini`
- DFU-layout offset-vector boot (`vector_table: 0x08005000`) proven on
  `maple_mini`
