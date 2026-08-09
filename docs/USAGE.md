# Usage — CLI, Library, Browser, Development

## Install

```bash
npm install @d7music/stm32-bluepill-emulator
```

The package ships everything as WASM: the CPU core is embedded in `unicorn_arm.cjs`
(self-contained binary, no external files) and peripherals in
`stm32_bluepill_wasm_bg.wasm`. It works in **Node** and in the **browser** (ES modules).

## Library API (browser / Node)

```js
import { createEmulator } from "@d7music/stm32-bluepill-emulator";

const emu = await createEmulator({
  firmware,            // Uint8Array of firmware (ELF, HEX, or raw BIN)
  flash_size: 0x10000, // Flash region size (64KB default)
  ram_size: 0x5000,    // SRAM size (20KB default)
  vector_table: 0x08000000, // Vector table base
  svd: null,           // SVD XML string (optional; defaults to hardcoded register map)
  uart_addr: 0x40013800,    // USART used by uartRx()
  verbose: false,
  ext_devices: {},     // see below
});

emu.step(maxInst);          // run ≤ maxInst instructions (loop this in your rAF)
emu.tick();                 // tick peripherals once (called inside step)
emu.setPc(pc); emu.getPc(); emu.getSp(); emu.getRegisters();
emu.resolveSymbol(addr);    // 'main+0x1e' style, needs symbols from ELF/map
emu.setSymbols(list);       // [{name, addr}] from .elf or .map
emu.getUartOutput();        // everything the firmware ever printed
emu.uartRx(byte); emu.uartRxBytes(bytes); emu.rxPending();
emu.canInjectMessage(addr, tir, tdtr, tdlr, tdhr);
emu.gpioReadOutput(port, pin); emu.gpioReadInput(port, pin); emu.gpioSetInput(...);
emu.gpioSetSlew(n);         // output slew delay in instructions (0 = instant) — IDR readback shows the old level until the transition settles
emu.pwmDuty(timerAddr, channel);    // duty 0-100, e.g. (0x40000000, 0) = TIM2 CH1
emu.setSimAdc(value);      // ADC conversion result firmware will read (conversion timing is real)
emu.setTouch(periphAddr, x, y, pressure);  // ADS7846 touch injection
emu.periphRead(addr, width); emu.periphWrite(addr, width, value); // raw register access
emu.memRead32(addr);       // read emulated RAM/Flash word (e.g. firmware flags)
```

### `ext_devices` — peripherals the STM32 talks to over SPI/I2C

```js
ext_devices: {
  spi_flash:   [{ peripheral, jedec_id, data, cs? }],            // e.g. {peripheral:0x40013000, jedec_id:0xEF4017, data:flashBuf}
  i2c_eeprom:  [{ peripheral, address, data }],                  // 7-bit address + backing Uint8Array (e.g. 64K)
  i2c_oled:    [{ peripheral, address, width, height }],         // e.g. SSD1306-style 128×64
  lcd:         [{ peripheral, cs }],                             // SPI TFT
  touchscreen: [{ peripheral, touch_detected_pin, cs }],         // ADS7846 (cs on the GPIO pin driving CS)
  software_spi:[{ name, cs, clk, miso, mosi }],                  // bit-banged SPI via GPIO transitions
  fsmc:        [{ name, data }],                                 // FSMC NOR/PSRAM: 'FSMC.BANK1..4' (NE1–4), 'FSMC.BANK5..6' (NAND), 'FSMC.BANK7' (PC Card); byte image
}
```

FSMC banks are read/written by the firmware at their memory windows (NE1–4 @ 0x6000_0000…,
NAND2/3 @ 0x7000_0000/0x8000_0000, PC Card @ 0x9000_0000) once the bank controller is
enabled (`BCR.MBKEN`; NOR writes also require `BCR.WREN`). Unmapped data accesses
(no bank backing or bank disabled) are raised as faults, and SVC/PendSV/hard-fault
handlers in firmware work normally (faults escalate to HardFault unless the SHCSR
enable bits are set via `SCB.SHCSR`).

## CLI (`pkg/cli.mjs`)

```bash
node pkg/cli.mjs <firmware> [options]
node pkg/cli.mjs --config=config.yaml <firmware>   # or --config=config.yaml with no position arg
```

Options:

| Flag | Meaning |
|---|---|
| `--config=<file>` | YAML config: registers (or SVD path), devices, memory regions, patches |
| `--max=<n>` | Run at most `n` instructions, then print `MAX_INST` summary and exit (e.g. `--max=200000000`). Default: run forever. |
| `--regs[=<file>]` | Dump register state to stdout/file and exit |
| `--uart=<file>` | Play a UART RX script file (bytes, fed via `uart_rx_byte`) |
| `--map=<file>` | Extra `.map` file for symbols (in addition to ELF symbols) |
| env `FIRMWARE`, `MAX_INST` | Same as the position arg / `--max` |

The CLI prints machine-parseable lines: `[name] PASS/FAIL` per test check and a final
`SUMMARY pass=N fail=N` (see the firmware test below). Firmware can be **ELF** (symbols
auto-loaded), **HEX**, or **raw BIN**.

### config.yaml

```yaml
cpu:
  vector_table: 0x08000000
  svd: path/to/file.svd       # optional; default = hardcoded register map
  use_hardcoded: true
regions:
  - start: 0x08000000
    size: 0x10000
    load: firmware.hex        # or bin/elf
devices:                      # same shape as library ext_devices
  spi_flash: [{ peripheral: 0x40013000, jedec_id: "0xEF4017", file: build/spi_flash.bin }]
  i2c_eeprom: [{ peripheral: 0x40005400, address: 0x51, file: build/eeprom.bin }]
patches:
  - addr: 0x08001BBC           # instruction-level patches (see workarounds)
```

### Firmware test (39 checks, the canary)

```bash
echo -n "AB" | node pkg/cli.mjs --config=tests/arduino_periph_test/config.yaml --max=200000000
node tests/canary.mjs          # same thing with --max=100000000, asserts 39/39, ~25s
```

- `A` (0x41) is reserved for the DMA RX test; `B` is the UART RX byte.
- CAN RX injection: cli polls the firmware's `canRxArmed` RAM global, then calls
  `can_inject_message(0x40006400, 0<<21, 2, 0xDEAD, 0)`. The firmware's filter bank 0 is
  ID-list mode → only STDID **0** matches — inject ID 0, not 0x123.
- **Batch-boundary timing**: peripherals tick only in `step_batch()` between `emu_start`
  batches — never mid-batch. Tests that read CNT/SR/IRQ flags after a `spin()` must be
  async-style (arm once, poll across batches). The one exception: `svc` fires
  synchronously inside a batch (the hook jumps straight to `SVC_Handler`), so a
  synchronous SVC check works in `setup()`.
- Use `--max=200000000` for the full run (50M stops mid-print, which is not a deadlock).

## Browser demo

```bash
python -m http.server -d pkg    # open http://localhost:8000
```

The demo runs the periph39 firmware live with a run loop that batches
`step(20000)` per rAF frame, renders UART output per frame, and reports real IPS.
Full periph39 run: ~0.5 s wall in-browser.

## Development

```bash
cargo check                          # Rust sanity (fast)
wasm-pack build --target web         # rebuild pkg/stm32_bluepill_wasm_bg* (Rust → wasm)
node tests/test_all.mjs              # 189 unit tests
node tests/canary.mjs                # regression canary: 39/39 firmware checks, ~25s
node tests/bench.mjs                 # benchmarks
```

Firmware rebuild (Linux, arduino-cli installed locally):

```bash
/home/danish1075/bin/arduino-cli compile --fqbn STMicroelectronics:stm32:GenF1:pnum=BLUEPILL_F103C8 \
  --build-path tests/arduino_periph_test/build tests/arduino_periph_test
# NOTE: --build-path wipes eeprom.bin/spi_flash.bin — restore from site/ after compiling:
node -e "const fs=require('fs'); const e2=Buffer.alloc(65536); e2[0]=0x42; e2[1]=0x24;
fs.writeFileSync('tests/arduino_periph_test/build/eeprom2.bin', e2);
fs.writeFileSync('tests/arduino_periph_test/build/spi_flash2.bin', Buffer.alloc(65536));"
```

> **Staging rule**: always `git add -A` or stage BOTH `pkg/` and `site/` together —
> CI compares `pkg/emulator.js` against `site/emulator.js`. `scripts/sync-site.sh`
> copies fresh artifacts (`pkg/*.js`, `pkg/*.wasm`, `pkg/emulator.js`, `pkg/index.html`,
> `pkg/*.cjs`) into `site/`.

Disassembly for ISR debugging (Windows PowerShell):
`arm-none-eabi-objdump -d tests/arduino_periph_test/build/arduino_periph_test.ino.elf > isr.asm`

## Performance notes

- ~22M instructions/sec in Node; the demo runs the whole 24-peripheral firmware in
  ~0.5 s wall.
- Batch size 20K → IRQ latency ≈ 1.1 ms at real speed.
- RSS stable at ~150 MB regardless of instruction count (no leaks — stress-verified
  over ~2.5B instructions).
