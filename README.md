# STM32 Bluepill Emulator

[![npm version](https://img.shields.io/npm/v/stm32f1-emu.svg?color=cb3837)](https://www.npmjs.com/package/stm32f1-emu)
[![npm downloads](https://img.shields.io/npm/dm/stm32f1-emu.svg)](https://www.npmjs.com/package/stm32f1-emu)
[![Live Demo](https://img.shields.io/badge/live%20demo-github%20pages-38bdf8)](https://danish9661.github.io/STM32-Bluepill-emu/)
[![License: MIT](https://img.shields.io/badge/license-MIT-yellow.svg)](LICENSE)

A full-system emulator for the **STM32F1 family** (STM32F103C8 "Blue Pill", STM32F105, etc.)
that runs **real, unmodified Arduino / STM32Cube firmware** in Node.js or the browser.

**~21.8M instructions/sec** headless (`50M` in `2.3s`, `pure b .` `26.5`), **~8-9M instr/sec** in the browser (`periph_test 40M` `8.6` headed, `4.5` headless `Worker` `4.5` `SAB OFF` `→` `9.2` `SAB ON`).

---

## Install

```bash
npm install stm32f1-emu
```

Requires **Node.js 18+** (ESM). In the browser, load via a `<script>` tag or bundler.

---

## Quick Start

### Node.js — High-Level API (recommended)

```js
import { STM32F1 } from 'stm32f1-emu';
import { readFileSync } from 'fs';

// Load an ELF, BIN, or Intel HEX
const mcu = await STM32F1.fromELF(readFileSync('firmware.elf'));

// Or from raw binary / Intel HEX text
// const mcu = await STM32F1.fromBin(readFileSync('firmware.bin'));
// const mcu = await STM32F1.fromHex(readFileSync('firmware.hex'), 'utf8');

// Subscribe to USART TX (MCU → host)
mcu.usart1.onData = (byte) => process.stdout.write(String.fromCharCode(byte));

// Subscribe to GPIO changes
mcu.gpio.pin('C', 13).on('change', (high) => {
  console.log('PC13 LED:', high ? 'ON' : 'OFF');
});

// Run 1 million instructions (auto-drains events after each batch)
const result = await mcu.execute(1_000_000);
console.log(result.instCount, 'instructions executed');

// Inject bytes into UART RX (host → MCU)
mcu.uartRx(0x41); // send 'A'

// Read UART output
console.log(mcu.uartOutput);

// Clean up
mcu.close();
```

### Node.js — Low-Level API

The low-level emulator is also available via the `./emulator` sub-export:

```js
import { createEmulator } from 'stm32f1-emu/emulator';
import { readFileSync } from 'fs';

const emu = await createEmulator({
  firmware: readFileSync('firmware.elf'),  // Uint8Array, string (HEX), or ArrayBuffer
});

const result = emu.run(10_000_000);  // run up to 10M instructions
console.log(result.instCount, 'instructions executed');

console.log(emu.getUartOutput());          // USART1 TX output
console.log(emu.gpioReadOutput(2, 13));    // PC13 level (port 2 = C)
emu.uartRxBytes([0x31, 0x32]);             // inject RX bytes
console.log(emu.getRegisters());           // { R0..R12, SP, LR, PC, xPSR }

emu.close();
```

### Browser

```html
<script src="unicorn_arm.js"></script>
<script type="module">
  import { STM32F1 } from 'stm32f1-emu';

  const mcu = await STM32F1.fromELF(
    await (await fetch('firmware.elf')).arrayBuffer()
  );

  mcu.usart1.onData = (b) => {
    document.getElementById('terminal').textContent += String.fromCharCode(b);
  };
  mcu.gpio.pin('C', 13).on('change', (high) => {
    document.getElementById('led').style.background = high ? '#f00' : '#300';
  });

  // Run 20K instructions per frame (~60 fps)
  function loop() {
    mcu.step(20_000);
    requestAnimationFrame(loop);
  }
  loop();
</script>
```

---

## STM32F1 Wrapper API

The high-level `STM32F1` class wraps the low-level emulator with a Wokwi-style
event-driven API. All bus transactions (SPI, I2C, USART, ADC, TIM, etc.) are
decoded into callbacks you subscribe to.

### Creating an Instance

```js
const mcu = await STM32F1.create({ firmware: buf, chip: 'stm32f103c8' });
const mcu = await STM32F1.fromELF(buf, opts?);
const mcu = await STM32F1.fromBin(buf, opts?);
const mcu = await STM32F1.fromHex(hexText, opts?);
```

| Option | Default | Description |
|---|---|---|
| `firmware` | empty | `Uint8Array`, `ArrayBuffer`, or Intel HEX string |
| `chip` | `'stm32f103c8'` | Chip ID or `{ name, svd }` for SVD-based layout |
| `svd` | `null` | SVD XML string (overrides `chip`) |
| `flash_size` | `0x10000` | Flash region size (bytes) |
| `ram_size` | `0x5000` | SRAM size (bytes) |
| `ext_devices` | `{}` | External devices (see below) |

**ext_devices:**

```js
{
  spi_flash:     [{ peripheral: 'SPI1', jedec_id: 0xEF4016, data: new Uint8Array(65536) }],
  i2c_eeprom:    [{ peripheral: 'I2C1', address: 0x50, data: new Uint8Array(1024) }],
  i2c_oled:      [{ peripheral: 'I2C1', address: 0x3C, width: 128, height: 64 }],
  lcd:           [{ peripheral: 'SPI1', cs: 'PA8' }],
  touchscreen:   [{ peripheral: 'SPI1', cs: 'PA1', touch_detected_pin: 'PC5' }],
  software_spi:  [{ name: 'FLASH', cs: 'PB12', clk: 'PB13', miso: 'PB14', mosi: 'PB15' }],
  fsmc_bank:     [{ name: 'FSMC.BANK1', data: new Uint8Array(65536) }],
}
```

### Execution

| Method | Returns | Description |
|---|---|---|
| `execute(cycles)` | `{ instCount, stopped }` | Run N instructions + auto-drain events |
| `step(cycles)` | `{ pc, instCount, stopped }` | Single batch + auto-drain events |
| `stop()` | `void` | Request stop of a running `execute()` loop |
| `close()` | `void` | Unsubscribe listeners + tear down |
| `reset()` | `Promise<STM32F1>` | Recreate emulator from scratch (reloads firmware) |

### GPIO

```js
const pin = mcu.gpio.pin('A', 5);  // or mcu.gpio.pin(0, 5)
```

| Method | Returns | Description |
|---|---|---|
| `pin.on('change', cb)` | `() => void` | Subscribe to output-level changes; returns unsubscribe |
| `pin.read()` | `0 \| 1` | Driven output level |
| `pin.readInput()` | `0 \| 1` | Input level |
| `pin.setInput(high)` | `void` | Drive external input (e.g. button press) |
| `pin.setAnalog(val)` | `void` | Set analog value (0–4095) |

### USART

| Property / Method | Description |
|---|---|
| `mcu.usart1` / `usart2` / `usart3` | USART wrappers |
| `usart.onData = (byte) => {}` | TX callback (MCU → host) |
| `usart.send(string \| number[])` | Inject bytes into MCU RX |
| `usart.output` | Accumulated TX string (getter) |

### SPI

| Property / Method | Description |
|---|---|
| `mcu.spi1` – `mcu.spi6` | SPI wrappers |
| `spi.onTransfer = (ch, tx, rx) => {}` | DR write callback |
| `spi.injectMiso([0xFF, ...])` | Queue MISO bytes for next transfer |

### I2C

| Property / Method | Description |
|---|---|
| `mcu.i2c1` / `i2c2` / `i2c3` | I2C wrappers |
| `i2c.onStart = (addr) => {}` | Start condition callback |
| `i2c.onWrite = (byte) => {}` | Byte write callback |
| `i2c.onRead = () => {}` | Read request callback |
| `i2c.onStop = () => {}` | Stop condition callback |
| `i2c.injectRx([0x55, ...])` | Queue RX bytes for next read |

### Virtual-Peripheral Events

These callbacks fire on specific hardware events:

| Callback | Signature | Description |
|---|---|---|
| `onExtiEdge` | `(line) => void` | EXTI external interrupt edge detected |
| `onAdcDone` | `(adc, chan) => void` | ADC conversion complete |
| `onTimUpdate` | `(tim) => void` | Timer overflow (update event) |
| `onTimCapture` | `(tim, ch, value) => void` | TIM input capture |
| `onDacWrite` | `(chan, value) => void` | DAC output written |
| `onCrcResult` | `(value) => void` | CRC calculation result read |
| `onRtcAlarm` | `(alarm) => void` | RTC alarm triggered |
| `onWdogReset` | `(which) => void` | Watchdog reset requested (1=IWDG, 2=WWDG) |
| `onCanTx` | `(can, id, len, data[8]) => void` | CAN message transmitted |
| `onCanRx` | `(can, id, len, data[8]) => void` | CAN message received |
| `onFsmcAccess` | `(bank, offset, write, size, value) => void` | FSMC bus transaction |

### Display Framebuffers

```js
const oledFb = mcu._emu.i2cOledFb('I2C1', 0x3C);  // Uint8Array (page-major)
const lcdFb  = mcu._emu.lcdFb('SPI1');              // Uint8Array (128×64, 1B/pixel)
```

### Symbol Resolution

```js
mcu.setSymbols(mapText);              // load GNU ld .map text
console.log(mcu.resolveSymbol(pc));   // e.g. "main+0x1e" or null
```

---

## Low-Level API

The `createEmulator()` function returns a `Promise<BluepillEmulator>`.

```js
import { createEmulator } from 'stm32f1-emu';

const emu = await createEmulator({
  firmware: readFileSync('firmware.elf'),
  flash_size: 0x10000,
  ram_size: 0x5000,
  vector_table: 0x08000000,
  chip: 'stm32f103c8',
  ext_devices: {},
  verbose: false,
});
```

### Execution

| Method | Returns | Description |
|---|---|---|
| `run(maxInstructions?)` | `{ totalSteps, instCount, stopped }` | Run up to N instructions (0 = forever) |
| `step(maxBatch?)` | `{ pc, instCount, stopped }` | Run one batch (default 20K instructions) |
| `stop()` | `void` | Request stop |
| `close()` | `void` | Tear down Unicorn instance |

### Registers & Memory

| Method | Returns | Description |
|---|---|---|
| `getRegisters()` | `{ R0..R12, SP, LR, PC, xPSR }` | All ARM registers |
| `getPc()` | `number` | Current program counter |
| `getSp()` | `number` | Current stack pointer |
| `setPc(pc)` | `void` | Set PC (auto-ORs with 1 for Thumb) |
| `read32(addr)` | `number` | Read 32-bit word from any address |
| `write32(addr, val)` | `void` | Write 32-bit word to any address |

### UART

| Method | Description |
|---|---|
| `getUartOutput()` | Accumulated USART1 TX output (string) |
| `uartRx(byte)` | Inject one byte into USART1 RX |
| `uartRxBytes([...])` | Inject multiple bytes |
| `uartRxAddr(addr, byte)` | Inject into a specific USART by base address |
| `rxPending()` | Number of unread bytes in UART RX buffer |

### GPIO

| Method | Description |
|---|---|
| `gpioReadOutput(port, pin)` | Read driven output level (port: 0=A, 1=B, 2=C) |
| `gpioReadInput(port, pin)` | Read input level |
| `gpioSetInput(port, pin, value)` | Drive external input |
| `gpioSetAnalog(port, pin, level)` | Set analog value (0–4095) |

### ADC / PWM / CAN

| Method | Description |
|---|---|
| `setSimAdc(value)` | Set simulated ADC value |
| `pwmDuty(addr, channel?)` | PWM duty (0–100) of a timer channel |
| `canInjectMessage(addr, tir, tdtr, tdlr, tdhr)` | Inject CAN message |

### Peripheral Bus

| Method | Description |
|---|---|
| `periphRead(addr, width?)` | Raw peripheral register read |
| `periphWrite(addr, width, value)` | Raw peripheral register write |

### Bus Watchers / Events

| Method | Returns | Description |
|---|---|---|
| `onPeriphWrite(fn)` | `() => void` | Subscribe to ALL peripheral writes; returns unsubscribe |
| `onPinChange(fn)` | `() => void` | Subscribe to chip-driven GPIO changes; returns unsubscribe |
| `drainEvents()` | `number[]` | Drain virtual-peripheral transaction events (flat i32 array) |
| `takePinEvents()` | `number[]` | Drain buffered pin-change events |

### Virtual Device Injection

| Method | Description |
|---|---|
| `spiInjectMiso(channel, bytes)` | Queue MISO bytes for a SPI channel |
| `i2cInjectRx(channel, bytes)` | Queue RX bytes for an I2C channel |
| `addJsPeripheral(base, size, read, write)` | Register a custom peripheral on the bus |

### Symbol Resolution

| Method | Description |
|---|---|
| `setSymbols(list)` | Set symbol table `[{name, addr}]` |
| `resolveSymbol(addr)` | Resolve address → symbol name (e.g. `"main+0x1e"`) |
| `getSymbolCount()` | Number of loaded symbols |

---

## CLI Usage

```bash
# Run a raw binary
npx stm32f1-emu firmware.bin [max_instructions]

# Run with config (YAML)
npx bluepill-emu --config=config.yaml

# Options
--regs              # Dump CPU registers at exit
--uart=0x40013800   # UART base address for stdin RX injection
--map=firmware.map  # Load symbol map for PC resolution
--verbose           # Print SP/PC at boot
--max=200000000     # Max instructions (default: 100M)
```

**Config YAML example:**

```yaml
flash: 0x08000000
ram: 0x20000000
regions:
  - start: 0x08000000
    file: firmware.hex
ext_devices:
  spi_flash:
    - peripheral: SPI1
      jedec_id: 0xEF4016
      data: flash.bin
  i2c_eeprom:
    - peripheral: I2C1
      address: 0x50
      data: eeprom.bin
```

---

## Firmware Formats

| Format | Description | How to Load |
|---|---|---|
| `.bin` | Raw binary (vector table at 0x08000000) | CLI, library, demo site |
| `.hex` | Intel HEX (Arduino/STM32duino output) | Auto-detected (starts with `:`) |
| `.elf` | ELF32 executable (segments + symbols) | Auto-detected by magic bytes |
| `.map` | GNU ld linker map (not executable) | Pair with `--map` flag for symbol names |

---

## WebSocket Bridge (headless Node + browser viewer)

```bash
# Start the server (runs the emulation loop, streams events over WebSocket)
node node_modules/stm32f1-emu/pkg/ws-server.mjs firmware.elf --port=8080

# Open the viewer in a browser
# http://localhost:8080/ws-viewer.html
```

The viewer renders a UART terminal, GPIO grid (click to toggle inputs), event log,
and FPS counter. The server streams all virtual-peripheral events as JSON.

---

## Development

```bash
# Rebuild Rust peripherals → WASM
PATH=/tmp/binaryen-version_132/bin:$PATH \
RUSTFLAGS="--remap-path-prefix=$HOME=/build" \
wasm-pack build --target web --out-dir pkg

# Run tests
node tests/test_all.mjs              # 236 unit tests
node tests/canary.mjs                # 39/39 firmware checks (~25s)
node tests/test_emulator_js.mjs      # browser run-loop path (200M, 39/39)
node tests/test_browser.mjs          # Playwright browser tests

# Run firmware directly
node pkg/cli.mjs firmware.elf
echo -n "AB" | node pkg/cli.mjs --config=config.yaml --max=200000000
```

---

## Supported Peripherals

GPIO (A–D) with electrical model, USART1–3, SPI1–2, I2C1–2, TIM1–14
(PWM, input capture, external triggers, slave modes, DMA requests), ADC1–2
(real conversion timing, RC sample-and-hold, DAC→ADC loopback, external triggers),
DAC1–2, DMA1 (7ch) + DMA2 (5ch), CAN1 (RX injection + filters), RTC (alarm),
CRC, NVIC (priority dispatch + 64-IRQ budget), SysTick, SCB (deep sleep, SHPR
routing, fault escalation), EXTI, AFIO (pin remap), BKP, WWDG, IWDG, PWR, FLASH,
FSMC (NOR/NAND/PC-Card), **USB is a stub**.

---

## License

MIT — see [LICENSE](LICENSE).

## Acknowledgements

- [AlexAltea/unicorn.js](https://github.com/AlexAltea/unicorn.js) — JavaScript / WebAssembly port of the Unicorn ARM CPU emulator.
- [Unicorn Engine](https://www.unicorn-engine.org/) — Multi-architecture CPU emulator framework based on QEMU.
