# STM32F1 JavaScript API (`pkg/stm32f1.js`)

`stm32f1.js` is a thin, rp2040js/avr8js-style wrapper over the low-level
`pkg/emulator.js` WASM bridge. It adds **no emulation logic and no hot-path
overhead** — every method delegates to `emulator.js`, and the only thing it does
at the API boundary is translate calls and route peripheral events.

It exposes the emulator the way a board-level library (rp2040js, avr8js) does:

```js
import { STM32F1 } from './stm32f1.js';

const mcu = await STM32F1.fromELF(fs.readFileSync('./firmware.elf'));

// GPIO: subscribe to output-level changes, read, or drive an input (button)
mcu.gpio.pin('A', 5).on('change', (high) => console.log('PA5 =', high));
mcu.gpio.pin('B', 13).setInput(true);   // simulate a button press

// USART: TX arrives via onData; send() injects host -> MCU RX
mcu.usart1.onData = (b) => process.stdout.write(String.fromCharCode(b));
mcu.usart2.send('AT\r\n');

// SPI / I2C virtual peripherals (see "Virtual-peripheral events" below)
mcu.spi1.onTransfer = (ch, tx, rx) => console.log('SPI1 tx', tx, 'rx', rx);
mcu.i2c1.onStart = (addr) => console.log('I2C1 start', addr);

await mcu.execute(100_000);
```

## Construction

| Static factory | Meaning |
| --- | --- |
| `STM32F1.fromELF(buf, opts?)` | Load an ELF firmware image |
| `STM32F1.fromBin(buf, opts?)` | Load raw binary (flashed at `0x08000000`) |
| `STM32F1.fromHex(text, opts?)` | Load Intel HEX text |
| `STM32F1.create(opts)` | Pass `opts` straight to `createEmulator()` |

`opts` is the `createEmulator()` options object. To attach virtual devices, pass
`opts.ext_devices` exactly as `cli.mjs`/`emulator.js` expect it:

```js
const mcu = await STM32F1.fromELF(elf, {
  ext_devices: {
    i2c_eeprom:  [{ peripheral: 'I2C1', address: '0x50', data: eepromImage }],
    spi_flash:   [{ peripheral: 'SPI1', jedec_id: '0xEF4016', data: flashImage, cs: 'PA4' }],
    i2c_oled:    [{ peripheral: 'I2C1', address: '0x3C', width: 128, height: 64 }],
    lcd:         [{ peripheral: 'SPI1', cs: 'PA1' }],
    touchscreen: [{ peripheral: 'SPI1', touch_detected_pin: 'PA3', cs: 'PA2' }],
  },
});
```

Instance methods: `execute(cycles) -> {instCount, stopped}`, `step(cycles)`,
`stop()`, `close()`, `reset()`, `loadELF/loadBin/loadHex(buf)`, `uartRx(byte)`,
`uartOutput`, `onPeriphWrite(fn)`, `setSymbols(text)`, `resolveSymbol(pc)`.

## GPIO

- `mcu.gpio.pin(port, pin)` → `GPIOPin` (`port` is `'A'`/`'B'`/`'C'` or `0`/`1`/`2`).
- `pin.on('change', cb)` → unsubscribe fn; fires on **chip-driven output** level changes (same source as `onPinChange`).
- `pin.read()` → driven output level `0`/`1`; `pin.readInput()` → input level; `pin.setInput(high)` drives an external input; `pin.setAnalog(0..4095)`.

## USART

- `mcu.usart1` / `usart2` / `usart3` (also `mcu.usart[1..3]`).
- `usart.onData = (byte) => …` fires for every byte the MCU **transmits** (TX).
- `usart.send(data)` injects bytes into that USART's **RX** (host → MCU).
- `usart.output` is the accumulated TX string for that USART (the core's
  `getUartOutput()` is USART1-only; this per-USART buffer is the ergonomic fix).

## Virtual-peripheral events (Wokwi-style)

SPI/I2C/USART transactions are emitted by the core as a flat event queue
(`drain_events()` in Rust) and **drained automatically once per `execute`/`step`
batch** by the wrapper, which dispatches them to per-bus callbacks. This is the
model Wokwi-style virtual peripherals expect: your JS device reacts to bus
transactions and injects bytes back.

### SPI — `mcu.spi1` … `mcu.spi6` (also `mcu.spi[1..6]`)

```js
mcu.spi1.onTransfer = (channel, tx, rx) => {
  // tx / rx are number[] of the bytes written / read on this DR access
  console.log('SPI', channel, 'tx', tx, 'rx', rx);
};

// Inject MISO bytes the MCU will read on the next transfers (virtual device -> MCU):
mcu.spi1.injectMiso([0xAA, 0xBB]);
```

### I2C — `mcu.i2c1` … `mcu.i2c3` (also `mcu.i2c[1..3]`)

```js
mcu.i2c1.onStart = (addr) => console.log('START', addr);  // 7-bit address
mcu.i2c1.onWrite = (byte) => console.log('WRITE', byte);
mcu.i2c1.onRead  = () => console.log('READ');
mcu.i2c1.onStop  = () => console.log('STOP');

// Inject RX bytes the MCU reads during master-receiver transactions:
mcu.i2c1.injectRx([0x12, 0x34]);
```

### USART — `mcu.usartN.onData`

As above; every transmitted byte is delivered here, regardless of which USART
(the core captures all of them, not just USART1).

## System-level events

Beyond the bus transactions above, the core also emits chip-level events:

```js
mcu.onExtiEdge  = (line)    => console.log('EXTI', line);   // GPIO input edge -> EXTI line 0..15
mcu.onAdcDone   = (adc, ch) => console.log('ADC', adc, 'chan', ch); // conversion complete (adc=1/2, ch=0..17)
mcu.onTimUpdate = (tim)     => console.log('TIM', tim, 'update');   // timer overflow/update (tim=1..8)
```

- `onExtiEdge(line)` fires when a GPIO input pin changes level and the EXTI line
  is unmasked + edge-configured (`exti.rs` `gpio_pin_changed`). This is what
  `attachInterrupt()` + an external edge (e.g. a button via `pin.setInput`) sees.
- `onAdcDone(adc, ch)` fires on every ADC EOC (regular + injected) in `adc.rs`.
- `onTimUpdate(tim)` fires on every timer update event (UIF / overflow) in `tim.rs`.

These are top-level callbacks on the `STM32F1` instance (assigned directly, like
`usart1.onData`); they are dispatched from the same per-batch `drain_events()`
that serves the bus events.

## More system / peripheral events

```js
mcu.onDacWrite   = (chan, value)   => ...; // DAC channel 1/2 output a new 12-bit value
mcu.onCrcResult  = (value)         => ...; // CRC_DR read -> computed 32-bit result
mcu.onRtcAlarm   = (alarm)         => ...; // RTC alarm time reached (crl CNF matched)
mcu.onWdogReset  = (which)         => ...; // 1 = IWDG, 2 = WWDG reset requested
mcu.onCanTx      = (can, id, len, data) => ...; // CAN frame queued for transmit (data[8])
mcu.onCanRx      = (can, id, len, data) => ...; // CAN frame received into a FIFO (data[8])
```

- `onDacWrite(chan, value)` fires on every DAC DHR write (`dac.rs`), `value` is the
  12-bit `DOR` driven on the pin (chan 1 = PA4, chan 2 = PA5).
- `onCrcResult(value)` fires when `CRC_DR` is read (`crc.rs`) — `value` is the
  accumulated 32-bit result.
- `onRtcAlarm(alarm)` fires when the RTC counter crosses the alarm value with the
  alarm interrupt enabled (`rtc.rs`).
- `onWdogReset(which)` fires when IWDG (`which=1`) or WWDG (`which=2`) rolls over
  and requests a core reset (`iwdg.rs` / `wwdg.rs`).
- `onCanTx` / `onCanRx` fire on CAN mailbox submission / reception (`can.rs`).
  `can` is 1 (CAN1) or 2 (CAN2); `id` is the 11-bit STDID or 29-bit EXTID; `len`
  is the DLC and `data` is an 8-byte array. These mirror Wokwi's CAN bus model.

## Timer input capture and FSMC transactions

```js
mcu.onTimCapture = (tim, ch, value) => ...; // input-capture latch (value = captured CNT)
mcu.onFsmcAccess = (bank, offset, write, size, value) => ...; // FSMC memory transaction
```

- `onTimCapture(tim, ch, value)` fires when a TIM channel configured for **input
  capture** (CCMR `CCxS != 0`) sees an edge matching its polarity on the source
  pin. `value` is the CNT latched into `CCRx`, `tim` is the timer number (1..14),
  `ch` the channel (0..3). This is real input capture: the timer samples the
  channel pin each batch and latches on the matching edge (see `tim.rs`
  `sample_input_capture`). The default pin mapping per timer is used (no AFIO
  remap), e.g. TIM2_CH1 = PA0.
- `onFsmcAccess(bank, offset, write, size, value)` fires on every FSMC memory
  access (`fsmc.rs`): `bank` is 1..7 (BANK1..7), `offset` the address within the
  bank, `write` true for writes, `size` the access width in bytes, and `value`
  the read result or written value. Useful for Wokwi-style memory-mapped
  peripherals (displays, etc.).

These are encoded as flat `drain_events()` discriminants 16 (`TimCapture`) and
17 (`FsmcAccess`).

## Implementation notes

- The event queue lives on `WasmSystem` (`src/system.rs`, `VmEvent` enum). The
  core pushes `SpiTransfer` (in `spi.rs` DR write), `I2cStart/Write/Read/Stop`
  (in `i2c.rs`), and `UartTx` (in `usart.rs` `write_dr`). `drain_events()`
  (`src/lib.rs`) flattens them to an `i32[]` consumed by `STM32F1._drain_events()`.
- Injection buffers (`spi_inject_miso`, `i2c_inject_rx`) are consumed in the
  peripheral read paths, overriding the attached device for that byte.
- `emulator.js` exposes `drainEvents()`, `spiInjectMiso(ch, bytes)`,
  `i2cInjectRx(ch, bytes)`, `uartRxAddr(addr, byte)`; the wrapper builds on these.
- Rebuild after Rust changes with the pinned toolchain and re-sync `pkg/` →
  `site/` (CI byte-exact guard):
  `PATH=binaryen-version_132/bin:$PATH RUSTFLAGS="--remap-path-prefix=$HOME=/build" wasm-pack build --target web`
