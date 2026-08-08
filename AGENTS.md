# STM32 Bluepill WASM Emulation — Context File

## Project Overview
Full-system emulation of an STM32F103C8 (Bluepill) microcontroller running real Arduino firmware. Two modules bridge through JavaScript:
1. **Unicorn ARM** (`pkg/unicorn_arm.cjs`) — ARM Cortex-M3 CPU emulator (binary Node addon, unmodifiable)
2. **Rust Peripherals** WASM (`pkg/stm32_bluepill_wasm_bg.wasm`) — GPIO, USART, TIM, SPI, I2C, DMA, RTC, CRC, CAN, NVIC, EXTI, ADC, DAC, FLASH, PWR, BKP, IWDG, WWDG, etc.

## Architecture & Emulation Loop
```
┌─────────────────────────── JS (pkg/cli.mjs) ─────────────────────────┐
│                                                                      │
│  codeHook (per-instruction) → counts instructions (no WASM calls)   │
│  memReadHook / memWriteHook → periph_read / periph_write  [JIT]     │
│                                                                      │
│  Loop (each iteration = 1 batch):                                    │
│    1. pump stdin → uart_rx_byte()                                    │
│    2. processDma()            ← move queued DMA data via Unicorn    │
│    3. uc.emu_start(pc|1, 0, 0, maxBatch=100K)                       │
│    4. step_batch(batchInstCount)   ← Rust ticks peripherals         │
│       - status==1 → watchdog reset requested → stop                 │
│    5. processDma()                                                  │
│    6. processInterrupts()  ← up to 16 IRQs per batch                │
│    7. is_watchdog_reset_requested() check                           │
│                                                                      │
│  DMA crosses the WASM boundary:                                      │
│    Rust queues DmaTransfer → JS dma_get_all_pending() →              │
│    uc.mem_read/mem_write to move data → dma_set_completed_many()    │
└──────────────────────────────────────────────────────────────────────┘
```

### Performance
- ~5.1M IPS real-world (20M instructions in ~3.9s)
- `step_batch()` gave 3.15× speedup over per-instruction `step()`
- `has_tick` flag: 69% tick speedup; `tick_indices` Vec + `AtomicU32` DMA bitmask: minor gains
- **Bottleneck**: memory hooks (periph_read/periph_write) are JS callbacks — every peripheral register access crosses WASM→JS→WASM

## Current Status (uncommitted WIP — see "What We Did — Current Sprint" below)
### Test suite: `node tests/test_all.mjs`
**158/158 unit tests PASS** (GPIO, USART, ADC, RCC, SysTick, TIM, IWDG, NVIC, CRC, SPI, I2C, RTC, PWR, FLASH, CAN, DMA, AFIO, EXTI, BKP, DAC, TIM6, RTC Alarm, UART RX).

### Firmware test — `tests/arduino_periph_test/` (21-peripheral Arduino sketch)
```
echo -n "AB" | node pkg/cli.mjs --config=tests/arduino_periph_test/config.yaml --max=200000000     # ~79s, 2000 steps
```
- **PASS (28/28)**: all tests green — sync section (GPIO, USART TX, UART Loopback, RCC, FLASH, PWR, BKP, IWDG, WWDG, RTC, CRC, DAC, ADC, AFIO, EXTI reg, CAN, SPI Flash, I2C EEPROM, I2C OLED, touchscreen, LCD) + async section (DMA TX/RX, UART RX, TIM2, EXTI0, SysTick)
- **Important**: 50M instr cap stops mid-print (not a deadlock); use `--max=200000000` for the full run. `A` (0x41) is reserved for the DMA RX test; `B` is the UART RX byte. `uart_rx_pending()` gate in cli.mjs prevents `A` from being consumed by the UART RX test.
- **USART TX test notes**: firmware test polls SR TXE up to 2M iterations. In emulator TXE re-asserts at batch boundaries, DRW per ISR run → ~1 byte / 100K-instr batch (not byte_time at 6250). Poll of 100K iters previously failed because the 22-byte drain needs ~2.2M instructions. Real HW at 115200: ~1.9ms drain, well under 2M-iteration budget.

## What We Did — Current Sprint (uncommitted WIP)
### USART byte-time TXE pacing (`src/peripherals/usart.rs`)
- `write_dr()` non-loopback now clears TXE and sets `txe_clear_until = instr + byte_time()` (BRR*p10); `tick()` re-asserts TXE + fires pending at byte-time edge — prevents back-to-back TXE interrupt storms
- `refresh_txe()` on SR/DR reads, `rx_pending()` for the DMA gate
### SPI device selection (`src/peripherals/spi.rs`, `src/peripherals/gpio.rs`)
- `active_device()` now uses new `gpio.read_pin_effective()` (read callback, else driven output state) — previously `read_port` ignored output-only pins, so the flash CS settled non-selected; touchscreen CS (PA2) now correctly selects the ADS7846
### Touchscreen ADS7846 protocol (`src/ext_devices/touchscreen.rs`)
- `deferred_reply` — the emulator returns the reply one SPI transfer (8 more clocks) after the command byte, matching the real part; cmd `0x94` (pressure/slave select) mapped to touch_pressure
### Exti / Tim (`src/peripherals/exti.rs`, `src/peripherals/tim.rs`)
- EXTI IMR writes and TIM DIER UIE writes now call `nvic.enable_irq()` for the mapped IRQ (sync section IRQs were never enabled → EXTI0/TIM2 IRQs could not fire)
### DMA (`src/peripherals/dma.rs`, `src/system.rs`)
- **`channels[ch].ndtr = 0` after `do_xfer()` removed** (async semantics: CNDTR holds until `tick` + JS-side `dma_set_completed_many`). The old zero-on-enable line made firmware re-arms immediately → second DMA transfer consumed the next stdin byte in `[DMAP]` (bug: UART RX FAIL)
- `queue_dma_transfer` dedupes on `stream_idx` (re-armed channel while a transfer is still queued is ignored — same root cause vs THE DUPLICATE)
### `src/peripherals/nvic.rs` / `src/lib.rs` cleanups
- Removed all `i2c_log`/`debug_*`/`I2C_LOG`/`flash_debug` instrumentation (see Active Workarounds) — machine-parseable `[name] PASS/FAIL` only
### Unit tests
- `tests/test_all.mjs`: UART/DMA tests now use `dma_get_pending_count()`/`dma_set_completed_many()` + `tick()` to model the async bridge — was 157, now **158/158 pass**

## Active Workarounds (temporary, remove or upstream later)
1. **`mrs rX, msp` → `mov rX, sp`** (cli.mjs `patchMrsMsp`, ~line 19): Unicorn cannot decode Thumb `mrs`; newlib `_sbrk` uses it; rewrite to 4-byte equivalent + nop (same footprint)
2. **i2c_init NVIC patch** (cli.mjs ~line 252): patch offset 0x8001BBC (block 0x8001BBC–0x8001BDB) replaced with inline ISER0/ISER1 writes + preserved `SetPriority` calls (Unicorn skips the two `bl HAL_NVIC_EnableIRQ`)
3. **hi2c->Mode patch** (cli.mjs `memWriteHook`): when `0x40005410` (I2C1 DR) is written with the R-bit set, patch RAM `*(0x200002d8)+0x3D` to 0x22 — HAL I2C1 ISR requires `hi2c->Mode == 0x22` (MASTER_RX) before reading DR
4. **Interrupt frame saved in JS closure variables** (not memory): stack frames get clobbered by handler PUSH; save R0-R3,R12,LR,PC,xPSR in JS locals, restore after handler
5. **16-IRQ loop in `processInterrupts()`**: prevents starvation when high-priority IRQ re-pends itself (e.g. CAN TX IRQ37 prio16 vs I2C EV IRQ31 prio32)
6. **DMA**: batched `dma_get_all_pending()` / `dma_set_completed_many()` — one WASM call instead of 7; JS-side `[DMAP]` direction decode: 0=periph→mem (periph_read), 1=mem→periph (periph_write), 2=mem→mem

## To Run / Rebuild
```bash
cargo check                          # Rust sanity (fast)
wasm-pack build --target web         # rebuild pkg/stm32_bluepill_wasm_bg* (Rust → wasm)
node tests/test_all.mjs              # 158 unit tests
node tests/bench.mjs                 # benchmarks
node pkg/cli.mjs tests/arduino_periph_test/build/arduino_periph_test.ino.elf   # run firmware
echo -n "AB" | node pkg/cli.mjs --config=tests/arduino_periph_test/config.yaml --max=200000000
# rebuild firmware (Linux, arduino-cli installed locally):
/home/danish1075/bin/arduino-cli compile --fqbn STMicroelectronics:stm32:GenF1:pnum=BLUEPILL_F103C8 --build-path tests/arduino_periph_test/build tests/arduino_periph_test
# NOTE: --build-path wipes eeprom.bin/spi_flash.bin — restore from site/ after compile
# browser demo:
python -m http.server -d pkg   # then open localhost:8000
```

### Disassembly for ISR debugging (Windows PowerShell)
```
arm-none-eabi-objdump -d tests/arduino_periph_test/build/arduino_periph_test.ino.elf > isr.asm
# find HAL_I2C_EV_IRQHandler / HAL_I2C_Master_*_IT symbols
```

## Next Phase — What's Left

### Immediate (get 21/21 — ALL PASS as of this sprint; re-check after any change)
1. **Verify nothing regressed after instrumentation removal** — rerun `tests/test_all.mjs` (158) + firmware 200M run (28/28) after any edit to `src/` or `pkg/cli.mjs`

### Known issue
- WASM abort (`Fatal: undefined Stack: undefined`) at ~35M+ instructions (seen once) — investigate if it re-occurs after other fixes

## Next Phase — Long-term Optimizations
1. **Single WASM module** (Emscripten): compile Rust peripheral code + Unicorn C into one `emcc` output (Linux toolchain; `wasm32-unknown-emscripten` target). Recommended-free approach elsewhere in docs
2. **Replace mem hooks with shared linear memory**: `uc_mem_map_ptr(mem, periph_range)` → Rust reads/writes same region, zero crossing
3. **DMA + interrupts fully in Rust** (no JS round-trip; `uc_intr` or stop+re-exec)
4. **Alternative: pure-Rust Cortex-M emulator** (cargo-cortex-m / mdl) — evaluate vs porting Unicorn

## Files Most Relevant
- `src/peripherals/i2c.rs` — I2C state machine
- `src/lib.rs` — WASM API; new exports
- `pkg/cli.mjs` — DR Mode patch, workarounds, loop
- `src/peripherals/usart.rs` — TXE byte-time pacing, `rx_pending()`
- `src/ext_devices/spi_flash.rs`, `src/peripherals/spi.rs`, `src/ext_devices/touchscreen.rs` — touchscreen SPI reads (deferred_reply)
- `src/peripherals/gpio.rs` — `read_pin_effective()`
- `tests/arduino_periph_test/` — the 21-test firmware + config