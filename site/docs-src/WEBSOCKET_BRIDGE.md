# WebSocket Bridge — Headless Emulator + Browser Viewer

Run the STM32F1 emulator headlessly on Node, stream all peripheral events to a
browser over WebSocket in real time, and send inputs (UART, GPIO, CAN) back.

```
┌──────────────────────────────┐          ┌──────────────────────────────┐
│  Node (ws-server.mjs)        │  JSON    │  Browser (ws-viewer.html)    │
│                              │  over    │                              │
│  createEmulator(firmware)    │  WS      │  Decode 17 event types       │
│  emu.step(20000) @ ~60fps   │ ───────→ │  UART terminal               │
│  drainEvents() + pins        │          │  GPIO pin grid (click→input) │
│                              │ ←─────── │  Event log                   │
│                              │  cmds    │  FPS / instruction counter   │
└──────────────────────────────┘          └──────────────────────────────┘
```

## Quick start

```bash
# 1. Install the runtime dependency (one-time)
npm install ws

# 2. Start the server with any firmware
node pkg/ws-server.mjs site/arduino_periph_test.elf

# 3. Open the viewer in any browser
open http://localhost:8080/ws-viewer.html
```

You'll see UART output streaming in the terminal panel, pin activity lighting
up the GPIO grid, and event types logged in the event log.

## Server — `pkg/ws-server.mjs`

### CLI flags

| Flag | Default | Meaning |
|------|---------|---------|
| `<firmware>` | *(required)* | Path to ELF, HEX, or raw BIN firmware image |
| `--port=N` | `8080` | HTTP + WebSocket port |
| `--max=N` | 0 (run forever) | Stop after N instructions |

### What it does

1. **Loads firmware** via `createEmulator({ firmware })` — same entry point as the
   browser demo and the `STM32F1` wrapper.
2. **Runs `emu.step(20000)`** every 16 ms (~60 fps). Each step advances up to 20K
   instructions (the same batch size the browser demo uses).
3. **Drains events** after each step: `emu.drainEvents()` returns all peripheral
   transactions (SPI, I2C, USART, EXTI, ADC, TIM, DAC, CRC, RTC, watchdog, CAN,
   TIM capture, FSMC) as a flat i32 array; `emu.takePinEvents()` returns GPIO
   pin-change triplets `[port, pin, level, ...]`.
4. **Broadcasts** the frame as JSON to every connected WebSocket client:
   ```json
   { "e": [6,1,65, 1,1,3,3, ...], "p": [0,5,1, 1,13,0], "fps": 60, "t": 2000000 }
   ```
5. **Idles** when no clients are connected (zero CPU usage).
6. **Stops** at `--max` instructions if given, then exits.

### HTTP server

The same Node process also serves `site/` over HTTP, so the viewer is available
at `http://localhost:8080/ws-viewer.html` without a separate file server.

### Client commands

When a browser sends a JSON message, the server dispatches it to the emulator:

| Command | Fields | Emulator call |
|---------|--------|---------------|
| `uart_rx` | `byte` (0–255), `addr` (optional, default `0x40013800` = USART1) | `emu.uartRxAddr(addr, byte)` |
| `gpio_set` | `port` (0=A, 1=B, 2=C), `pin` (0–15), `high` (bool) | `emu.gpioSetInput(port, pin, high)` |
| `can_inject` | `addr`, `tir`, `tdtr`, `tdlr`, `tdhr` | `emu.canInjectMessage(addr, tir, tdtr, tdlr, tdhr)` |

## Viewer — `site/ws-viewer.html`

A standalone HTML page — no build step, no dependencies, works in any modern
browser.

### Features

- **UART terminal** — MCU-transmitted bytes (USART1) appear as green text;
  typing in the input box sends bytes to the MCU's USART1 RX.
- **GPIO pin grid** — 48 pins (ports A–C, pins 0–15); click any pin to toggle
  its external input level (sends `gpio_set` to the server). Active (driven)
  pins glow amber.
- **Event log** — every peripheral event is decoded and appended (SPI transfers,
  I2C transactions, EXTI edges, ADC conversions, timer updates, DAC writes,
  CRC results, RTC alarms, watchdog resets, CAN frames, TIM captures, FSMC
  accesses).
- **FPS + instruction counter** — live badges at the top.
- **Reconnects** automatically on disconnect (2 s backoff).

### Architecture

The viewer is a pure WebSocket client:

```
ws-viewer.html
  → new WebSocket('ws://localhost:8080')
  → onmessage: JSON.parse → decodeEvents(msg.e) + updatePins(msg.p)
  → send: { type:'uart_rx', byte:65 } / { type:'gpio_set', port, pin, high }
```

## WebSocket protocol

### Server → Client (broadcast)

Every frame the server sends a JSON object:

```json
{
  "e":   [ ... ],     // flat i32 array: event discriminants + payloads
  "p":   [ ... ],     // pin-change triplets: [port, pin, level, ...]
  "fps": 60,          // frames per second (step calls / sec)
  "t":   2000000      // total instructions executed so far
}
```

#### Event array format (`e`)

The array is a sequence of tagged events. Each event starts with a type
discriminant (1–17), followed by its fields. The special value `0` terminates
the sequence.

| Disc | Name | Fields | Description |
|------|------|--------|-------------|
| 1 | `SpiTransfer` | `channel, txLen, rxLen, txBytes..., rxBytes...` | SPI DR write (full-duplex transfer) |
| 2 | `I2cStart` | `channel, addr` | I2C START condition (7-bit address) |
| 3 | `I2cWrite` | `channel, byte` | I2C byte written by master |
| 4 | `I2cRead` | `channel` | I2C read request (master requests a byte) |
| 5 | `I2cStop` | `channel` | I2C STOP condition |
| 6 | `UartTx` | `usart, byte` | USART byte transmitted (usart = 1/2/3) |
| 7 | `ExtiEdge` | `line` | GPIO edge detected on EXTI line 0–15 |
| 8 | `AdcDone` | `adc, chan` | ADC conversion complete (adc = 1/2, chan = 0–17) |
| 9 | `TimUpdate` | `tim` | Timer update/overflow event (tim = 1–14) |
| 10 | `DacWrite` | `chan, value` | DAC output updated (chan = 1/2, value = 12-bit) |
| 11 | `CrcResult` | `value` | CRC result read (32-bit) |
| 12 | `RtcAlarm` | `alarm` | RTC alarm value reached |
| 13 | `WdogReset` | `which` | Watchdog reset requested (1 = IWDG, 2 = WWDG) |
| 14 | `CanTx` | `can, id, len, d0..d7` | CAN frame submitted to TX mailbox |
| 15 | `CanRx` | `can, id, len, d0..d7` | CAN frame received into FIFO |
| 16 | `TimCapture` | `tim, ch, value` | TIM input-capture latch (captured CNT) |
| 17 | `FsmcAccess` | `bank, offset, write, size, value` | FSMC memory read/write |

#### Pin-change array format (`p`)

Triplets of `(port, pin, level)` where `port` is 0=A/1=B/2=C, `pin` is 0–15,
and `level` is 0 (low) or 1 (high). Only chips that have been driven to a
**new** level appear here — not every batch.

### Client → Server (commands)

Send any of these as a JSON string over the WebSocket:

```json
{ "type": "uart_rx",    "byte": 65 }
{ "type": "uart_rx",    "addr": 4194304, "byte": 65 }
{ "type": "gpio_set",   "port": 1, "pin": 13, "high": true }
{ "type": "can_inject", "addr": 4194400, "tir": 0, "tdtr": 2, "tdlr": 357913941, "tdhr": 0 }
```

## Writing a custom client

Any WebSocket client that speaks JSON works. Here's a minimal Node example:

```js
import { WebSocket } from 'ws';

const ws = new WebSocket('ws://localhost:8080');

ws.on('open', () => {
  // Send a UART byte to the MCU
  ws.send(JSON.stringify({ type: 'uart_rx', byte: 0x41 }));
});

ws.on('message', (data) => {
  const msg = JSON.parse(String(data));
  if (msg.e) {
    // Parse the flat event array (see protocol above)
    let i = 0;
    while (i < msg.e.length) {
      const type = msg.e[i++];
      if (type === 0) break;
      if (type === 6) {  // UartTx
        const usart = msg.e[i++], byte = msg.e[i++];
        process.stdout.write(String.fromCharCode(byte));
      } else {
        // Skip other event types (see Disc column above for field counts)
        break;  // implement parsers for the events you care about
      }
    }
  }
  if (msg.p) {
    // Pin changes: triplets of [port, pin, level]
    for (let j = 0; j < msg.p.length; j += 3) {
      console.log(`Pin ${String.fromCharCode(65 + msg.p[j])}${msg.p[j + 1]} = ${msg.p[j + 2]}`);
    }
  }
});
```

### Browser example

```js
const ws = new WebSocket('ws://localhost:8080');
ws.onmessage = (e) => {
  const { e: events, p: pins, fps, t } = JSON.parse(e.data);
  // events: flat i32 array (discriminants 1–17)
  // pins:   port/pin/level triplets
  // fps:    frames per second
  // t:      total instructions
};
```

## Running with different firmware

```bash
# The 24-peripheral test firmware (39 checks, UART RX "AB")
echo -n "AB" | node pkg/ws-server.mjs site/arduino_periph_test.elf --max=200000000

# The WS2812 LED strip demo (SPI + DMA)
node pkg/ws-server.mjs site/arduino_ws2812.elf

# The hardware showcase (7 peripherals: OLED, LCD, 7-seg, RGB, buzzer, button)
node pkg/ws-server.mjs site/arduino_hw_showcase.elf

# Custom firmware
node pkg/ws-server.mjs path/to/firmware.elf --port=9090
```

## Implementation notes

- **Typed-array serialization**: WASM's `drain_events()` returns an `Int32Array`.
  `JSON.stringify` serializes typed arrays as `{"0":val,"1":val,...}` objects,
  not `[val,val,...]` arrays. The server wraps results in `Array.from()` before
  serialization — this is required for correct protocol framing.
- **Batch size**: 20K instructions per step matches the browser demo. This gives
  ~1.1 ms IRQ latency at real speed. Smaller batches improve responsiveness but
  increase frame overhead.
- **Idle when no clients**: the `setInterval` tick short-circuits when
  `clients.size === 0`, so the server consumes zero CPU when the browser is
  closed.
- **HTTP + WS on one port**: the `ws` library's `WebSocketServer` accepts an
  existing `http.Server`, so both the static file server and WebSocket endpoint
  share a single TCP port.
- **No firmware modification needed**: the bridge uses the same `createEmulator`
  API as the CLI and browser demo. Firmware that runs in the browser demo or
  under `cli.mjs` will stream identically over WebSocket.

## Related docs

- [STM32F1 JavaScript API](STM32F1_API.md) — full event callback reference
- [USAGE.md](USAGE.md) — CLI flags, library API, `ext_devices`, config.yaml
- [PERIPHERALS.md](PERIPHERALS.md) — what's emulated at register level
