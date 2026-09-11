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

## Demos on this chip (25 presets)

All 21 portable demos (blink, echo, comprehensive, periph37, fade,
timer_uart, pwm_wave, servo, adc_uart, dac_sine, rtc_clock, stopwatch,
flash_demo, showcase, ws2812, i2c_scan, i2c_slave, can_chat, sd_logger,
usb_cdc, mini_rtos) plus the Blue Pill builds: `board_pill`,
`board_pill_echo`, `board_pill_showcase`, `board_pill_rtc`.

## Verification

- periph39 firmware: **39/39** (`node tests/canary.mjs`)
- `tests/test_board_demo.mjs`, `test_board_echo.mjs`,
  `test_board_showcase.mjs`, `test_board_rtc.mjs` green on `stm32f103c8`
