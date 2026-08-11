# STM32 Bluepill Emulator (STM32F103C8) — run Arduino firmware in the browser

[![Live Demo](https://img.shields.io/badge/live%20demo-github%20pages-38bdf8)](https://danish9661.github.io/STM32-Bluepill-emu/)
[![About](https://img.shields.io/badge/about-github%20pages-7dd3fc)](https://danish9661.github.io/STM32-Bluepill-emu/about.html)
[![License: MIT](https://img.shields.io/badge/license-MIT-yellow.svg)](LICENSE)

A **full-system emulator for the STM32F103C8 "Blue Pill" microcontroller** that runs
**real, unmodified Arduino firmware** (STM32duino) — as well as libopencm3 and STM32Cube
HAL programs — in **Node.js or the browser**. Powered by the **Unicorn ARM Cortex-M3 CPU
emulator compiled to WebAssembly (WASM)** + a **Rust peripheral emulator** implementing
the chip's registers with realistic timing, interrupts, and electrical behavior.

- **Try it live**: [danish9661.github.io/STM32-Bluepill-emu](https://danish9661.github.io/STM32-Bluepill-emu/) — 9 demo presets with a board + LED visualization and a UART terminal, or upload your own `.bin`/`.hex`/`.elf`
- **~5.1M instructions/sec** in the browser, **~22M instructions/sec** headless (200M instructions in ~9 s)
- Runs firmware compiled for the real chip: Arduino core (STM32duino), libopencm3, STM32Cube HAL
- Verified: `Serial.println()`, `digitalWrite()`, `millis()`, SPI, I2C, CAN, DMA, ADC, timers and interrupts all work
- **Cycle-accurate timing**: 1 instruction = 1 CPU cycle on a 72 MHz-equivalent time base (SysTick/timer/UART timing matches real silicon); wall speed scales with host throughput, so a "1 s" timer demo runs slower than one line per wall second (~0.2 s of chip time per second of wall time headless)
- **Demo presets**: blink, comprehensive test (21 peripherals), peripheral test (39 automated checks), UART echo, TIM2 PWM fade, TIM2-on-UART, ADC-on-UART, SPI flash + I2C EEPROM/OLED (JEDEC ID, EEPROM write/readback, device scan), **peripheral showcase** (I2C OLED + SPI LCD framebuffers, 74HC595 7-seg, RGB PWM, buzzer, EXTI13 button — all as JS-side drivers watching the buses)

## Features

- **Full peripheral set** — GPIO (A–D) with an electrical model (pull-ups, open-drain, output slew), USART1–3, SPI1–2, I2C1–2, TIM1–14 (PWM, input capture, external ADC triggers), ADC1–2 (real conversion timing + RC sample-and-hold, DAC→ADC loopback, external triggers), DAC1–2, DMA1 (7 channels), CAN1 (RX injection), RTC alarm, CRC, NVIC priority dispatch, SysTick, EXTI, AFIO, BKP, WWDG, IWDG, PWR, FLASH, FSMC (NOR/NAND/PC-Card)
- **Realistic analog & interrupts** — SVC/PendSV/fault escalation, deep-sleep gating, timer-driven ADC triggers (TIM1–4 TRGO/CC events, EXTI 11/15)
- **224/224 unit tests** and a **39/39 integration canary** running a real 24-peripheral Arduino sketch (~9 s)
- **rp2040js-style bus** — peripherals register on a runtime bus (`src/bus.rs`); custom JS peripherals (`emu.addJsPeripheral`) and multi-chip support (STM32F103C8 builtin, any F1-family SVD like STM32F105 with CAN2)
- **Library + CLI + browser demo** — embed it, script it, or demo it

## Install

```bash
npm install stm32-bluepill-emu
```

## Library usage

```js
import { createEmulator } from 'stm32-bluepill-emu';
import { readFileSync } from 'fs';

const emu = await createEmulator({
  firmware: readFileSync('blink.bin'),       // raw .bin, vector table at 0x08000000
  ext_devices: {
    i2c_eeprom: [{ peripheral: 'I2C1', address: 0x50, data: new Uint8Array(1024) }],
    spi_flash:  [{ peripheral: 'SPI1', jedec_id: 0xEF4016, data: new Uint8Array(65536) }],
    i2c_oled:   [{ peripheral: 'I2C1', address: 0x3C, width: 128, height: 64 }],
  },
});

const result = emu.run(10_000_000);          // run up to 10M instructions
console.log(result.instCount, 'instructions executed');

console.log(emu.getUartOutput());            // everything Serial.print'ed
console.log(emu.gpioReadOutput(2, 13));      // PC13 LED state
emu.uartRxBytes([0x31, 0x32]);               // inject RX bytes
console.log(emu.getRegisters());             // { R0..R12, SP, LR, PC, xPSR }
emu.close();
```

### Options

| Option | Default | Description |
|---|---|---|
| `firmware` | empty | Raw firmware binary |
| `flash_size` | `0x10000` | Flash region size |
| `ram_size` | `0x5000` | SRAM size |
| `vector_table` | `0x08000000` | Vector table base |
| `svd` | hardcoded map | SVD XML string for peripheral layout |
| `uart_addr` | `0x40013800` | USART base used by `uartRx()` |
| `ext_devices` | `{}` | SPI flash / I2C EEPROM / OLED / LCD / touchscreen / software SPI |
| `verbose` | `false` | Print SP/PC at boot |

### Emulator API

- `run(maxInstructions)` → `{ totalSteps, instCount, stopped }`
- `step(maxBatch)` → `{ pc, instCount, stopped }`
- `stop()`, `close()`
- `getRegisters()`, `getPc()`, `getSp()`, `setPc(pc)`
- `getUartOutput()`, `uartRx(byte)`, `uartRxBytes([...])`
- `gpioReadOutput(port, pin)`, `gpioReadInput(port, pin)`, `gpioSetInput(port, pin, val)`
- `canInjectMessage(addr, tir, tdtr, tdlr, tdhr)`
- `setSimAdc(value)`, `setTouch(peripheral, x, y, pressure)`
- `periphRead(addr, width)`, `periphWrite(addr, width, value)` (low-level registers)
- `read32(addr)`, `write32(addr, value)` (Unicorn memory)
- `hasPendingInterrupt()`, `getNextPendingInterrupt()`, `setIntrMasks(pm, bp)`

## CLI usage

```bash
npx bluepill-emu firmware.bin [max_instructions]
npx bluepill-emu --config=config.yaml
```

- `--regs` — dump CPU registers at exit
- `--uart=0x40013800` — UART base for stdin RX injection
- Config YAML supports regions, SVD path, patches, and external devices

## Firmware formats

- **`.bin`** — raw binary, loaded at the flash base (default `0x08000000`). Works everywhere (CLI, library, demo site).
- **`.hex`** — Intel HEX (Arduino/STM32duino output). Auto-detected (starts with `:`) by the CLI and `createEmulator()`, and by the demo site for `.hex` uploads. `--config` regions load `.hex` files too.
- **`.elf`** — full ELF32 executable: auto-detected by magic bytes; all `PT_LOAD` segments are written to their link addresses (flash + RAM init data), and symbols are extracted automatically for `resolveSymbol()`.
- **`.map`** — GNU ld linker map; **not executable**. Pair it with the firmware for symbol names: `cli.mjs --map=build/app.ino.map`, or upload it in the demo site after the firmware. The terminal/register panel then shows `PC → HAL_Delay+0x14` instead of a bare address. `resolveSymbol(addr)` is also available on the library emulator object (`emu.setSymbols(...)`, or auto from `.elf`).

## Browser usage

```html
<script src="unicorn_arm.js"></script>
<script type="module">
  import { createEmulator } from 'stm32-bluepill-emu';
  const emu = await createEmulator({ firmware: await (await fetch('blink.bin')).arrayBuffer() });
  emu.run(1_000_000);
</script>
```

A full demo site (board + LED visualization, UART terminal, run/step controls) and an [About page](https://danish9661.github.io/STM32-Bluepill-emu/about.html) live in `site/` and are deployed to GitHub Pages via `.github/workflows/pages.yml`.

## License

MIT — see [LICENSE](LICENSE).

## Development

```bash
wasm-pack build --target web --out-dir pkg   # rebuild Rust peripherals
node tests/test_all.mjs                      # 224 unit tests
node tests/bench.mjs                         # benchmarks
node pkg/cli.mjs firmware.bin                # run firmware
```

## Supported peripherals

Fully implemented: GPIO (A–D), USART1–3, SPI1–2, I2C1–2, TIM1–7 (PWM, input capture, interrupts), ADC1–2, DMA1 (7 channels), CAN1 (mailboxes + filters), RTC (alarm), CRC, NVIC (priority dispatch), SysTick, SCB, EXTI, AFIO, BKP, DAC, WWDG, IWDG, PWR, FLASH, FSMC.

**USB is a stub** (register reads return 0). Firmware using USB will not work.

## Architecture

```
┌──────────── JS driver (cli.mjs / emulator.js) ────────────┐
│  Loop:                                                     │
│   1. uc.emu_start(maxBatch=20K)  ← Unicorn WASM          │
│   2. step_batch(count)            ← Rust ticks peripherals│
│   3. processDma()                 ← DMA data movement     │
│   4. processInterrupts()          ← NVIC → IRQ injection  │
└────────────────────────────────────────────────────────────┘
```

See `docs/NEXT_PHASE.md` for the roadmap (the historical "merge WASM modules for 5–10×"
goal was retired — the JS bridge was never the bottleneck; ~22M IPS measured today).
