# STM32 Bluepill WASM Emulation — Context File

## Project Overview
Full-system emulation of an STM32F103C8 (Bluepill) microcontroller running Arduino firmware. Two WASM modules bridge through JavaScript:
1. **Unicorn ARM** (`pkg/unicorn_arm.cjs`/`.js`) — ARM Cortex-M3 CPU emulator (native C + WASM-compiled)
2. **Rust Peripherals** (`pkg/stm32_bluepill_wasm_bg.wasm`) — All STM32 peripherals (GPIO, USART, TIM, SPI, I2C, DMA, RTC, CRC, CAN, NVIC, etc.)

## Architecture
```
┌─────────────────────────── JS (cli.mjs) ───────────────────────────┐
│                                                                     │
│  codeHook (per-instruction) → counts instructions (no WASM calls)  │
│  memReadHook / memWriteHook → periph_read / periph_write  [JIT]   │
│                                                                     │
│  Loop:                                                              │
│    1. emu_start(maxBatch=100K)   ← Unicorn runs N instructions      │
│    2. step_batch(count)          ← Rust ticks all peripherals      │
│    3. processDma()               ← DMA data movement via Unicorn   │
│    4. processInterrupts()        ← NVIC→Unicorn IRQ injection      │
│                                                                     │
│  DMA data movement crosses WASM boundary:                           │
│    Rust queues DmaTransfer → JS reads via dma_get_pending()         │
│    → uc.mem_read/mem_write to move data → dma_set_completed()       │
└─────────────────────────────────────────────────────────────────────┘
```

## Current Performance
- **5.1M IPS** real-world (20M instructions in ~3.9s)
- `step_batch()` gave **3.15× speedup** over per-instruction `step()`
- `tick_indices` Vec and single `AtomicU32` DMA bitmask gave minor gains
- **Bottleneck**: Memory hooks (periph_read/periph_write) are JS callbacks — every peripheral register access crosses WASM→JS→WASM boundary

## Source Layout

### Rust (wasm-pack build → pkg/)
- `src/lib.rs` — WASM bindings: init, periph_read/write, tick, step, step_batch, DMA, GPIO, UART
- `src/system.rs` — WasmSystem: tick() iterates tick_indices, DMA completion tracking via AtomicU32
- `src/peripherals/mod.rs` — Peripheral trait, Peripherals struct, SVD-based construction, tick_indices registration
- `src/peripherals/*.rs` — One file per peripheral (dma, gpio, usart, tim, rtc, crc, can, nvic, spi, i2c, etc.)
- `src/ext_devices/` — External devices: SPI flash, I2C EEPROM, LCD, I2C OLED, touchscreen

### JS
- `pkg/cli.mjs` — Main emulation runner: Unicorn setup, event loop, DMA/interrupt processing
- `pkg/unicorn_arm.cjs` + `unicorn_arm.js` — Unicorn ARM WASM module (binary, native Node binding)
- `pkg/index.html` — Browser demo (python -m http.server)

### Tests
- `test_all.mjs` — 157 unit tests (GPIO, USART, ADC, RCC, SysTick, TIM, IWDG, NVIC, CRC, SPI, I2C, RTC, PWR, FLASH, CAN, DMA, AFIO, EXTI, BKP, DAC, TIM6, RTC Alarm, UART RX). Run: `node test_all.mjs`
- `comprehensive_test/` — 21-peripheral firmware test (Arduino sketch → bin). Run: `node pkg/cli.mjs --config=comprehensive_test/config.yaml`
- `bench.mjs` — Microbenchmarks comparing tick() vs step() performance
- `pkg/test_unicorn.cjs` — Simple Unicorn smoke test

### Config
- `Cargo.toml` — Rust deps: wasm-bindgen, svd-parser, serde, log, regex
- Build: `wasm-pack build --target web` (outputs to pkg/)

## What We Did (Optimization Sprint)

### Completed
1. **`has_tick` flag on peripherals** → 69% tick speedup (10.7M→18.2M IPS). Only tick peripherals that implement `tick()`.
2. **`tick_indices` Vec** → ~3% gain over iterating all peripherals checking has_tick.
3. **Single `AtomicU32` DMA bitmask** → ~8% gain, replacing `[AtomicBool; 8]` with bit operations.
4. **`step_batch(count)`** → 3.15× real-world speedup (13.79s→3.9s / 20M instr). Process N instructions in one WASM call.
5. **Removed `clock_enabled()` gate from `read()`/`write()`** — tests assume direct register access without RCC setup.
6. **CRC fix**: Non-reflected `0x04C11DB7` polynomial (removed bogus data reflection).
7. **DMA fix**: Restore `ndtr = 0` after `do_xfer()` on EN bit.
8. **RTC alarm**: `cnt >= alarm` instead of `cnt == alarm` (handles multi-tick jumps).
9. **RTC CRL**: RTOFF at bit 5 (was bit 0).
10. **RTC/FLASH register offsets** match STM32F103 datasheet.
11. **SPI test fix**: Read SR before DR (DR read clears RXNE).
12. **cli.mjs**: Removed stale `initSync(...)` call (module auto-initializes).

### All 157 unit tests + 21 comprehensive tests pass
Commit: `c5f9dec` (10 files, +283/-78)

## Current Limitations / Architecture Issues
- **Two separate WASM modules** bridged through JS → memory hooks are JS callbacks, WASM→JS→WASM round-trip on every peripheral register access
- **DMA data movement**: Rust queues transfer info → JS reads it → JS calls uc.mem_read/write → JS calls dma_set_completed(). 7 JS calls per DMA transfer.
- **Unicorn ARM module** (`unicorn_arm.cjs`): appears to be a binary Node native addon (not WASM). Cannot be modified without compiling from source.
- **Interrupt delivery**: JS reads NVIC state from Rust after every batch, injects into Unicorn. Could be latency up to 100K instructions.

## Next Steps (on Linux)

### Phase 1: Compile Unicorn + Rust into a single WASM module
This eliminates all JS boundary crossings for memory hooks and DMA.

**Required on Linux:**
1. **Emscripten SDK** — to compile Unicorn C source to WASM
2. **Unicorn Engine source** — user has it. Need `unicorn/` directory with CMakeLists.txt
3. **Rust nightly + `wasm32-unknown-emscripten` target** — to compile Rust peripheral code as a static lib linkable with Emscripten
4. **Python3 + CMake** — build toolchain

**Build approach:**
```
emcc -s WASM=1 -s TOTAL_MEMORY=512MB \
     -s EXPORTED_FUNCTIONS=['_uc_open','_uc_close','_uc_mem_map',...] \
     -s EXPORTED_RUNTIME_METHODS=['ccall','cwrap'] \
     unicorn/*.c rust_periph.a \
     -o unicorn_periph.js
```

### Phase 2: Replace memory hooks with direct memory access
Once both are in the same WASM instance:
- Map a section of WASM linear memory as the STM32 peripheral address space (0x40000000–0xB0000000)
- `uc_mem_map_ptr(0x40000000, size, PROT_ALL, wasm_memory_ptr)`
- Rust peripherals read/write the same memory region — no hooks needed
- DMA data movement becomes a direct memory copy in Rust

### Phase 3: Move DMA + interrupt processing into Rust
- DMA: instead of queueing transfer info for JS, do `memcpy` directly in Rust
- Interrupts: Rust directly calls Unicorn's `uc_intr` or sets NVIC pending and stops execution

### Alternative: Pure Rust ARM emulator
Instead of porting Unicorn, investigate existing pure-Rust Cortex-M emulators:
- `cargo-cortex-m` / `mdl` — may be simpler than porting Unicorn C to WASM
- Tradeoff: would need to implement all ARM Thumb instructions, but no JS boundary at all
- Check: `cortex-m-emulator` crate, `mdl` (Micro-Debug Lab), or `qemu` in Rust bindings

### Fallback: Optimize within current architecture
If WASM port is too complex:
- Merge DMA processing into Rust (avoid JS round-trip for data movement)
- Use `SharedArrayBuffer` for zero-copy state sharing between WASM modules
- Pre-allocate and batch DMA transfer processing

## Files Most Relevant for Next Phase
- `src/lib.rs` — WASM API boundary; add DMA-processing functions
- `src/system.rs` — DMA queue + completion tracking; can be extended for direct data movement
- `src/peripherals/dma.rs` — DMA peripheral implementation; transfer logic here
- `pkg/cli.mjs` — Caller; rewrite to remove DMA/interrupt JS bridge
- `pkg/unicorn_arm.cjs` / `.js` — Binary; need source to recompile
- `Cargo.toml` — Add `wasm32-unknown-emscripten` target support

## Build Commands (Windows, for reference)
```bash
wasm-pack build --target web    # builds Rust → pkg/
node test_all.mjs               # 157 unit tests
node bench.mjs                  # benchmarks
node pkg/cli.mjs firmware.bin   # run firmware
```
