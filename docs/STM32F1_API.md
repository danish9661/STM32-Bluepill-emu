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

`opts` is the `createEmulator()` options object (`pkg/emulator.js:205`). `batch_size` (`pkg/emulator.js:217`) is `20000` by default or adaptive `20K` (IRQ/DMA pending) / `50K` idle when omitted — pass `batch_size: 50000` to force a fixed batch, see `site/worker.js:130` / `pkg/cli.mjs:632`. To attach virtual devices, pass
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

Instance methods: `execute(cycles) -> {instCount, stopped}`, `step(cycles)` (respects `batch_size` / adaptive 20K/50K, `pkg/emulator.js:698`),
`stop()`, `close()`, `reset()`, `loadELF/loadBin/loadHex(buf)`, `uartRx(byte)`,
`uartOutput`, `onPeriphWrite(fn)`, `setSymbols(text)`, `resolveSymbol(pc)`.

## GPIO

- `mcu.gpio.pin(port, pin)` → `GPIOPin` (`port` is `'A'`/`'B'`/`'C'` or `0`/`1`/`2`).
- `pin.on('change', cb)` → unsubscribe fn; fires on **chip-driven output** level changes (same source as `onPinChange`).
- `pin.read()` → driven output level `0`/`1`; `pin.readInput()` → input level; `pin.setInput(high)` drives an external input; `pin.setAnalog(0..4095)`.

```js
// Subscribe to PC13 output changes (LED blink)
const unsub = mcu.gpio.pin('C', 13).on('change', (high) => {
  console.log('LED:', high ? 'ON' : 'OFF');
});

// Simulate a button press on PB13 (EXTI13)
mcu.gpio.pin('B', 13).setInput(true);   // press
mcu.gpio.pin('B', 13).setInput(false);  // release

// Read current state
const pa5Level = mcu.gpio.pin('A', 5).read();        // output level
const pa0Input = mcu.gpio.pin('A', 0).readInput();   // input level
```

## USART

- `mcu.usart1` / `usart2` / `usart3` (also `mcu.usart[1..3]`).
- `usart.onData = (byte) => …` fires for every byte the MCU **transmits** (TX).
- `usart.send(data)` injects bytes into that USART's **RX** (host → MCU).
- `usart.output` is the accumulated TX string for that USART (the core's
  `getUartOutput()` is USART1-only; this per-USART buffer is the ergonomic fix).

```js
// Echo every byte the MCU sends on USART1
mcu.usart1.onData = (b) => {
  process.stdout.write(String.fromCharCode(b));
};

// Send a command to USART2 (e.g. AT firmware)
mcu.usart2.send('AT+RST\r\n');

// Read all USART1 output accumulated so far
console.log(mcu.usart1.output);
```

## Virtual-peripheral events (Wokwi-style)

SPI/I2C/USART transactions are emitted by the core as a flat event queue
(`drain_events()` in Rust) and **drained automatically once per `execute`/`step`
batch** by the wrapper, which dispatches them to per-bus callbacks. This is the
model Wokwi-style virtual peripherals expect: your JS device reacts to bus
transactions and injects bytes back.

### SPI — `mcu.spi1` … `mcu.spi6` (also `mcu.spi[1..6]`)

```js
// Observe SPI1 transfers (fires on every DR write)
mcu.spi1.onTransfer = (channel, tx, rx) => {
  // tx / rx are number[] of the bytes written / read on this DR access
  console.log('SPI', channel, 'tx', tx, 'rx', rx);
};

// Inject MISO bytes the MCU will read on the next transfers (virtual device -> MCU):
mcu.spi1.injectMiso([0xAA, 0xBB]);
```

### I2C — `mcu.i2c1` … `mcu.i2c3` (also `mcu.i2c[1..3]`)

```js
// Observe I2C1 transaction edges
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
mcu.onUsbIn = (ep, data) => ...; // USB IN completion (device -> host bytes)
```

- `onTimCapture(tim, ch, value)` fires when a TIM channel configured for **input
  capture** (CCMR `CCxS != 0`) sees an edge matching its polarity on the source
  pin. `value` is the CNT latched into `CCRx`, `tim` is the timer number (1..14),
  `ch` the channel (0..3). This is real input capture: the timer samples the
  channel pin each batch and latches on the matching edge (see `tim.rs`
  `sample_input_capture`). The pin mapping honors the AFIO MAPR remap for
  TIM2/TIM3/TIM4 (e.g. TIM2_REMAP=01 moves CH2 from PA1 to PB3), default mapping
  otherwise.
- `onFsmcAccess(bank, offset, write, size, value)` fires on every FSMC memory
  access (`fsmc.rs`): `bank` is 1..7 (BANK1..7), `offset` the address within the
  bank, `write` true for writes, `size` the access width in bytes, and `value`
  the read result or written value. Observation-only. To feed data **back** to the
  MCU (so the MCU reads what the virtual device provides), use
  `mcu.fsmcWriteByte(name, offset, byte)` / `mcu.fsmcReadByte(name, offset)`
  to read/write the FSMC backing image directly (no bus side effects, no events).
  The FSMC bank must be registered via `ext_devices.fsmc_bank` in the options
  passed to `fromELF` / `fromHex` / `create`.

These are encoded as flat `drain_events()` discriminants 16 (`TimCapture`) and
17 (`FsmcAccess`).

## USB device events

```js
mcu.onUsbIn = (ep, data) => ...; // IN transfer completed: `data` bytes for the host
```

- `onUsbIn(ep, data)` fires when firmware arms an IN transfer (STAT_TX → VALID)
  and the packet moves to the host (`usb.rs` `complete_in`). Host → device
  traffic goes the other way via injection: `mcu._emu` exposes
  `usbInjectSetup(bytes8)` / `usbInjectOut(ep, bytes)` (queued through
  `usb_inject_setup` / `usb_inject_out`; NAKed unless the endpoint is armed
  VALID). Encoded as flat discriminant 18 (`UsbIn`: `[ep, len, bytes...]`).

## Complete worked examples

### Virtual I2C EEPROM (write-back store)

Build a virtual I2C EEPROM from scratch — the MCU firmware writes bytes and
reads them back, all driven by the event queue:

```js
import { STM32F1 } from './stm32f1.js';
import { readFileSync } from 'fs';

const mcu = await STM32F1.fromELF(readFileSync('./firmware.elf'));
const store = new Uint8Array(256); // 256-byte EEPROM image

let addrBytes = 0, memAddr = 0;

mcu.i2c1.onStart = (addr) => {
  addrBytes = 0;
  console.log(`I2C START → 0x${addr.toString(16)}`);
};
mcu.i2c1.onWrite = (byte) => {
  if (addrBytes < 2) {
    memAddr = (memAddr << 8) | byte;  // 2-byte address
    addrBytes++;
  } else {
    store[memAddr++] = byte;          // data byte → write to store
    console.log(`EEPROM write [0x${(memAddr - 1).toString(16)}] = 0x${byte.toString(16)}`);
  }
};
mcu.i2c1.onRead = () => {
  const val = store[memAddr++];
  mcu.i2c1.injectRx([val]);           // push next byte for MCU to read
  console.log(`EEPROM read [0x${(memAddr - 1).toString(16)}] = 0x${val.toString(16)}`);
};
mcu.i2c1.onStop = () => console.log('I2C STOP');

await mcu.execute(2_000_000);
```

### Virtual SPI SD card

Intercept SPI transactions to implement a minimal SD card protocol:

```js
const mcu = await STM32F1.fromELF(readFileSync('./firmware.elf'));
let csHigh = true;
let cmdBuf = [];

mcu.gpio.pin('A', 4).on('change', (high) => {
  csHigh = high;
  if (high) { cmdBuf = []; }  // CS deasserted → reset command buffer
});

mcu.spi1.onTransfer = (ch, tx, rx) => {
  if (csHigh) return;  // chip not selected
  for (const b of tx) {
    cmdBuf.push(b);
    if (cmdBuf.length >= 6) {
      const cmd = cmdBuf[1] & 0x3F;
      console.log(`SD CMD${cmd} args=${cmdBuf.slice(2, 6).map(x => x.toString(16).padStart(2, '0')).join('')}`);
      // Respond with R1 (0x00 = idle, 0x01 = not idle)
      mcu.spi1.injectRx([cmd === 0 ? 0x01 : 0x00]);
      cmdBuf = [];
    }
  }
};

await mcu.execute(5_000_000);
```

### Timer input capture (frequency counter)

Measure an external signal frequency using TIM2 input capture:

```js
const mcu = await STM32F1.fromELF(readFileSync('./firmware.elf'));
const periods = [];

mcu.onTimCapture = (tim, ch, value) => {
  if (tim === 2 && ch === 0) {
    periods.push(value);
    if (periods.length >= 2) {
      const delta = periods[periods.length - 1] - periods[periods.length - 2];
      const freq = 72_000_000 / delta;  // assuming 72 MHz timer clock
      console.log(`TIM2 CH1 capture: CNT=${value}, period=${delta} ticks, freq=${freq.toFixed(0)} Hz`);
    }
  }
};

// Simulate a 1 kHz square wave on PA0 (TIM2_CH1)
let t = 0;
const simulate = () => {
  mcu.gpio.pin('A', 0).setInput(true);
  setTimeout(() => mcu.gpio.pin('A', 0).setInput(false), 500);
  t += 1000;
  if (t < 10_000) setTimeout(simulate, 1000);  // 10 cycles
};
simulate();
await mcu.execute(1_000_000);
```

### CAN bus bridge (forward CAN1 → CAN2)

Monitor CAN1 traffic and re-inject it on CAN2:

```js
const mcu = await STM32F1.fromELF(readFileSync('./can_bridge.elf'));

mcu.onCanRx = (can, id, len, data) => {
  console.log(`CAN${can} RX: id=0x${id.toString(16)} len=${len} data=`,
    data.slice(0, len).map(b => b.toString(16).padStart(2, '0')).join(' '));
};

mcu.onCanTx = (can, id, len, data) => {
  console.log(`CAN${can} TX: id=0x${id.toString(16)} len=${len}`);
};

await mcu.execute(10_000_000);
```

### ADC + DAC loopback

Read ADC1 channel 0 (PA0) and output on DAC1 (PA4), monitoring both:

```js
const mcu = await STM32F1.fromELF(readFileSync('./adc_dac.elf'));

mcu.onAdcDone = (adc, ch) => {
  console.log(`ADC${adc} ch${ch} conversion complete`);
};

mcu.onDacWrite = (chan, value) => {
  console.log(`DAC${chan} output: ${value} (${(value * 3.3 / 4095).toFixed(2)}V)`);
};

// Drive PA0 with an analog value (simulates a sensor)
mcu.gpio.pin('A', 0).setAnalog(2048);  // ~1.65V (half VDDA)

await mcu.execute(1_000_000);
```

### FSMC-backed LCD display

Drive an FSMC-connected LCD and observe the frame buffer:

```js
const mcu = await STM32F1.fromELF(readFileSync('./lcd_demo.elf'), {
  ext_devices: {
    fsmc_bank: [{ name: 'FSMC.BANK1', size: 256 * 1024 }],
  },
});

mcu.onFsmcAccess = (bank, offset, write, size, value) => {
  if (bank === 1) {
    const rs = (offset >> 16) & 1;  // A16 = RS pin
    if (write) {
      console.log(`LCD ${rs ? 'DATA' : 'CMD'}: 0x${value.toString(16).padStart(2, '0')}`);
    }
  }
};

await mcu.execute(2_000_000);
```

### WebSocket bridge (headless Node + browser viewer)

`pkg/ws-server.mjs` runs the emulator headlessly and streams all 17 event
types to connected browser clients over WebSocket. `site/ws-viewer.html`
provides a ready-made viewer with UART terminal, GPIO pin grid, event log,
and FPS counter. Any WebSocket client can connect and exchange JSON messages.

```bash
npm install ws                                # runtime dependency
node pkg/ws-server.mjs firmware.elf           # --port=8080 --max=200000000
# open http://localhost:8080/ws-viewer.html
```

Client → server commands: `uart_rx` (inject UART byte), `gpio_set` (drive
an input pin), `can_inject` (inject a CAN frame). Server → client broadcast:
`{ e: [...], p: [...], fps, t }` — flat event array, pin changes, FPS, total
instructions.

Full protocol reference, event-type field layout, and custom-client examples:
**[docs/WEBSOCKET_BRIDGE.md](WEBSOCKET_BRIDGE.md)**

## Event-type reference table

| Disc | Event | Fields | Source |
|------|-------|--------|--------|
| 1 | `SpiTransfer` | `[channel, txLen, rxLen, tx..., rx...]` | `spi.rs` DR write |
| 2 | `I2cStart` | `[channel, addr]` | `i2c.rs` DR write (StartSent) |
| 3 | `I2cWrite` | `[channel, byte]` | `i2c.rs` DR write (Active TX) |
| 4 | `I2cRead` | `[channel]` | `i2c.rs` DR read (Active RX) |
| 5 | `I2cStop` | `[channel]` | `i2c.rs` CR1 STOP generation |
| 6 | `UartTx` | `[usart, byte]` | `usart.rs` DR write |
| 7 | `ExtiEdge` | `[line]` | `exti.rs` `gpio_pin_changed` |
| 8 | `AdcDone` | `[adc, chan]` | `adc.rs` EOC/JEOC |
| 9 | `TimUpdate` | `[tim]` | `tim.rs` UIF / overflow |
| 10 | `DacWrite` | `[chan, value]` | `dac.rs` DHR write |
| 11 | `CrcResult` | `[value]` | `crc.rs` DR read |
| 12 | `RtcAlarm` | `[alarm]` | `rtc.rs` alarm crossed |
| 13 | `WdogReset` | `[which]` | `iwdg.rs` / `wwdg.rs` reset request |
| 14 | `CanTx` | `[can, id, len, d0..d7]` | `can.rs` mailbox submit |
| 15 | `CanRx` | `[can, id, len, d0..d7]` | `can.rs` inject_message |
| 16 | `TimCapture` | `[tim, ch, value]` | `tim.rs` `sample_input_capture` |
| 17 | `FsmcAccess` | `[bank, offset, write, size, value]` | `fsmc.rs` read_sized/write_sized |
| 18 | `UsbIn` | `[ep, len, bytes...]` | `usb.rs` `complete_in` |

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

## I2C clock stretching note

Clock stretching (`stretch_until` in `i2c.rs`) defers `fire_interrupts()` for a
configurable number of instructions after a byte transfer, simulating the slave
device holding SCL low during internal write cycles. This is only applied by the
slave device's callback, **not** unconditionally on every DR write — the latter
would deadlock the ISR-driven `HAL_I2C_Master_Transmit_IT` path (the TXE
interrupt that drives subsequent byte writes would be deferred, stalling the
transfer after the first byte).
