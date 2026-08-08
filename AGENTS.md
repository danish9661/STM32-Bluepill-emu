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
- ~5.1M IPS real-world (200M instructions in ~39s; 100M in ~25s)
- `step_batch()` gave 3.15× speedup over per-instruction `step()`
- `has_tick` flag: 69% tick speedup; `tick_indices` Vec + `AtomicU32` DMA bitmask: minor gains
- **instCount as plain number, not BigInt** (cli.mjs + pkg/emulator.js codeHook): ~19% faster full run (48.3s → 39.1s); BigInt ops per instruction were measurable at 5M instr/sec. `maxInst` compare + `step_batch` arg are now numbers too. Same change in emulator.js lifted the browser demo from 3.8M → 5.0M Avg IPS (~30%)
- **Site runLoop**: batch ~5× `step(100000)` per rAF frame (80ms budget), one UI pass per frame — UART TXE pacing needs 100K batches; Speed stat divides real frame instructions, not a fixed 500K
- **Bottleneck**: memory hooks (periph_read/periph_write) are JS callbacks — every peripheral register access crosses WASM→JS→WASM. `node --cpu-prof` shows ~55% of time inside wasm functions (Rust peripheral dispatch), 3.1% in wasm-to-js glue, ~4.5% JS `get` — per-access Uint8Array reuse was neutral (binding reallocates anyway)
- **Regression canary**: `node tests/canary.mjs` (or `node tests/canary.mjs <maxInstr>` default 100M) — runs firmware, asserts exit 0, no FAIL lines, SUMMARY pass=37 fail=0, ~25s. Faster than the full 200M run.

## Current Status (uncommitted WIP — see "What We Did — Current Sprint" below)
### Test suite: `node tests/test_all.mjs`
**158/158 unit tests PASS** (GPIO, USART, ADC, RCC, SysTick, TIM, IWDG, NVIC, CRC, SPI, I2C, RTC, PWR, FLASH, CAN, DMA, AFIO, EXTI, BKP, DAC, TIM6, RTC Alarm, UART RX).

### Firmware test — `tests/arduino_periph_test/` (24-peripheral Arduino sketch, 37 checks)
```
echo -n "AB" | node pkg/cli.mjs --config=tests/arduino_periph_test/config.yaml --max=200000000     # ~40s, 2000 steps
```
- **PASS (37/37)**: sync section (GPIO, USART TX, UART Loopback, RCC, FLASH, PWR, BKP, IWDG, WWDG, RTC, CRC, DAC, ADC, AFIO, EXTI reg, CAN, SPI Flash, I2C EEPROM, I2C OLED, touchscreen, LCD, I2C2 EEPROM, SPI2 Flash, USART2 Loopback) + async section (DMA TX/RX, UART RX, TIM2, EXTI0, EXTI1, EXTI13, CAN RX, SysTick, TIM3 PWM, TIM4, RTC Alarm IRQ)
- **CAN RX injection**: cli.mjs polls the firmware's `canRxArmed` RAM global (symbol from ELF), then calls `can_inject_message(0x40006400, 0<<21, 2, 0xDEAD, 0)`. Note the firmware's filter bank 0 is ID-list mode (FS1R=1, FM1R=0, F0=0) → only STDID **0** matches — inject ID 0, not 0x123.
- **Batch-boundary timing**: emulator ticks peripherals only in `step_batch()` between `emu_start` batches — never mid-batch. Any test that reads CNT/SR/IRQ flags after a `spin()` must be async-style (arm once, poll across batches), else it sees CNT=0.
- **Important**: 50M instr cap stops mid-print (not a deadlock); use `--max=200000000` for the full run. `A` (0x41) is reserved for the DMA RX test; `B` is the UART RX byte. `uart_rx_pending()` gate in cli.mjs prevents `A` from being consumed by the UART RX test.
- **USART TX test notes**: firmware test polls SR TXE up to 2M iterations. In emulator TXE re-asserts at batch boundaries, DRW per ISR run → ~1 byte / 100K-instr batch (not byte_time at 6250). Poll of 100K iters previously failed because the 22-byte drain needs ~2.2M instructions. Real HW at 115200: ~1.9ms drain, well under 2M-iteration budget.
- **I2C2/SPI2 devices**: `build/eeprom2.bin` (0x51, 64K) + `build/spi_flash2.bin` (JEDEC `0xEF4017`, CS PB12) — both must be re-created after an arduino-cli rebuild (build dir gets wiped):
  `node -e "const fs=require('fs'); const e2=Buffer.alloc(65536); e2[0]=0x42; e2[1]=0x24; fs.writeFileSync('tests/arduino_periph_test/build/eeprom2.bin', e2); fs.writeFileSync('tests/arduino_periph_test/build/spi_flash2.bin', Buffer.alloc(65536));"`

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
### New firmware tests + cli CAN RX injection (this sprint)
- `tests/arduino_periph_test/arduino_periph_test.ino`: added `testI2C2()` (register-level I2C2 master on 0x40005800 → EEPROM 0x51 round-trip via repeated START), `testSPI2()` (register-level SPI2 master 0x40003800, JEDEC `EF 40 17` + PP/readback on PB12), `testUSART2()` (HDSEL loopback), plus async TIM3 PWM (CC1IF via OC1M=PWM1), TIM4 CNT, RTC alarm IRQ (custom `extern "C" RTC_IRQHandler`, IRQ3), EXTI1 (PB1) + EXTI13 (PB13) via SWIER, and CAN RX (firmware sets `canRxArmed` RAM flag → cli injects once → firmware reads RFIFO0)
### cli.mjs perf (`pkg/cli.mjs`)
- **instCount/batchInstCount as plain numbers** (was BigInt): ~19% faster full run (48.3s → 39.1s at 200M); codeHook increments are the hottest JS path
- CAN RX injection: `can_inject_message` import; main loop polls the ELF symbol `canRxArmed` via `uc.mem_read` and injects a single CAN frame `(ID=0, DLC=2, data=0xDEAD)` — 37/37 firmware checks PASS
- Regression canary: `tests/canary.mjs` (spawns cli with `--max=100000000`, asserts `SUMMARY pass=37 fail=0`, prints `CANARY PASS`) — ~25s, replaces slower full-run checks
### WASM abort investigation (this sprint)
- `Fatal: undefined Stack: undefined` at ~35M instr (seen once) — stress-tested: 7 runs totalling ~2.5B instructions (200M×3, 400M×2, 600M, 300M with `usr/bin/time -v`) — **zero aborts**, max RSS 155MB stable (no leak). Not reproducible; monitor on any re-occurrence

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
node tests/canary.mjs                # regression canary: 37/37 firmware checks, ~25s
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

### Immediate (ALL PASS as of this sprint; re-check after any change)
1. **Verify nothing regressed** — rerun `tests/test_all.mjs` (158) + canary (`node tests/canary.mjs`, 37/37) after any edit to `src/` or `pkg/cli.mjs`

### Known issue (monitor only)
- WASM abort (`Fatal: undefined Stack: undefined`) at ~35M+ instructions — seen once, **not reproducible** across ~2.5B stress instructions (7 runs). Re-investigate if it re-occurs

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