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

## Current Status (commit `561a856`)
### Test suite: `node tests/test_all.mjs`
**157/157 unit tests PASS** (GPIO, USART, ADC, RCC, SysTick, TIM, IWDG, NVIC, CRC, SPI, I2C, RTC, PWR, FLASH, CAN, DMA, AFIO, EXTI, BKP, DAC, TIM6, RTC Alarm, UART RX).

### Firmware test — `tests/arduino_periph_test/` (21-peripheral Arduino sketch)
```
echo "AB" | node pkg/cli.mjs --config=tests/arduino_periph_test/config.yaml --max=50000000
```
- **PASS (17)**: GPIO, USART TX, UART Loopback, RCC, FLASH, PWR, BKP, IWDG, WWDG, RTC, CRC, DAC, ADC, AFIO, EXTI reg, CAN, SPI Flash
- **PASS (2)**: I2C OLED, I2C OLED write
- **FAIL (1)**: `[I2C] FAIL: eeprom=1 oled=1 val=FF` — EEPROM write works (0x00,0x00,0x42), read returns 0xFF
- **NOT REACHED:** Touchscreen, LCD, and the async section (DMA TX/RX, UART RX, TIM2, EXTI0, SysTick)
- `A` (0x41) piped via stdin stays queued for the DMA RX test; `B` is the UART RX byte. Stdin is drained into `uart_rx_byte()` each batch.

## What We Did — Latest Sprint (commit `561a856`)
### I2C peripheral fixes (`src/peripherals/i2c.rs`)
- **`fire_interrupts` CR2 bit mapping corrected**: ITERREN=bit8, ITEVTEN=bit9, ITBUFEN=bit10 (was cyclically shifted → wrong IRQs fired)
- **`fire_interrupts` moved inside `if self.sr1_addr_flag` in SR2 read** — previously every SR2 read could re-pend the EV interrupt → infinite ISR loop (this was the original "I2C ISR storm" bug)
- **CR2 write** enables NVIC IRQ31/32 (via new `nvic.enable_irq()`) and fires interrupts on ITEVTEN bits; BUF-IT-disable sets BTF (SR1 bit 2)
- **TRA bit fixed**: SR2 TRA is bit 2, not bit 4 (caused ISR to think it was transmitter when receiver)
- **SR2 read** transitions `AddrSent{is_read}` → `Active{is_read}` on ADDR-clear; sets RXNE (bit 6) + clears TRA for reads, TXE (bit 7) + sets TRA for writes
- **DR read (0x10)** in `Active{is_read}` auto-preloads next byte from device and re-asserts RXNE
- **DR write (0x10)**: StartSent → sends address (devices matched by 7-bit addr), preloads first byte on read; Active{is_read:false} → pushes byte to device; NACK path sets SR1 bit 10 (AF)
- **reset()** clears SR1/SR2/state on SW reset / PE=0 → keeps `hi2c` RAM struct untouched (firmware-side issue, see workarounds)

### SPI flash (`src/ext_devices/spi_flash.rs`)
- Page-program write support: WREN/WRDI, status register returns real WEL bit (0x02), page address+data latching, CS-gated
- ReadData (4-byte: 3 address bytes latched) and FastRead (5-byte: 1 dummy + 3 addr) address-latch fixes
- `SpiFlashConfig.cs` now accepted; `cli.mjs` passes `d.cs` through

### USART (`src/peripherals/usart.rs`)
- HDSEL loopback fix: `read_dr()` returns looped TX byte from `self.dr` instead of a queued external byte; external stdin bytes stay queued for the UART RX test

### NVIC (`src/peripherals/nvic.rs`)
- Added `enable_irq()`, `is_enabled()`, `is_pending()` helpers; debug log in `find_highest_pending()` (see below)

### WASM JS bridge (`src/lib.rs`, `pkg/cli.mjs`, `pkg/emulator.js`)
- **Debug log buffer**: `static I2C_LOG` + `i2c_log()` + `drain_i2c_log()` export — used via `[I2C]`/`[NVIC]` trace lines in the firmware run
- SysTick: reads of 0xE000100 (reports `instCount & 0xFFFFFFFF`)
- **ELF LMA fix** (`emulator.js` `parseElf`): also loads the LMA (p_paddr) copy of each segment — firmware's startup code copies `.data` from its load address; without both, .data is zero
- `i2c_init` firmware patch + Mode patching in `cli.mjs` (see Workarounds)

## Active Workarounds (temporary, remove or upstream later)
1. **`mrs rX, msp` → `mov rX, sp`** (cli.mjs `patchMrsMsp`, ~line 19): Unicorn cannot decode Thumb `mrs`; newlib `_sbrk` uses it; rewrite to 4-byte equivalent + nop (same footprint)
2. **i2c_init NVIC patch** (cli.mjs ~line 252): Unicorn skips the two `bl HAL_NVIC_EnableIRQ` in `i2c_init` → replace the block at 0x8001bb0–0x8001bcf with inline ISER0/ISER1 writes + preserved `SetPriority` calls
3. **hi2c->Mode patch** (cli.mjs `memWriteHook`, ~line 313): HAL I2C ISR requires `hi2c->Mode == 0x22` (MASTER_RX) to read DR; Wire calls a transmit-HAL which leaves 0x21 — when DR is written with R-bit set (0x40005410), patch RAM `*(0x20000228)+0x3D` to 0x22
4. **Interrupt frame saved in JS closure variabless** (not memory): stack frames get clobbered by handler PUSH; save R0-R3,R12,LR,PC,xPSR in JS locals, restore after handler
5. **16-IRQ loop in `processInterrupts()`**: prevents starvation when high-priority IRQ re-pends itself (e.g. CAN TX IRQ37 prio16 vs I2C EV IRQ31 prio32)
6. **DMA**: batched `dma_get_all_pending()` / `dma_set_completed_many()` — one WASM call instead of 7

## Firmware Analysis Notes (HAL ISR / disassembly)
- EEPROM uses I2C1 addr 0x50 on SMBUS-compatible driver; internal debug log trace confirmed: write = START→ADDR(0x50|W)→bytes 0x00 0x00 0x42→BTF→STOP (OK); read = START→ADDR(0x50|R)→clears ADDR→RXNE→**STOP without DR read** → root cause is `hi2c->Mode` check at ISR 0x80034c2
- `[I2C]` trace shows full state machine transitions; `[NVIC]` trace shows IRQ31/IRQ37 pending/prio race

## To Run / Rebuild
```bash
cargo check                          # Rust sanity (fast)
cargo wasmpack build --target web    # rebuild pkg/stm32_bluepill_wasm_bg* (Rust → wasm)
node tests/test_all.mjs              # 157 unit tests
node tests/bench.mjs                 # benchmarks
node pkg/cli.mjs tests/arduino_periph_test/build/arduino_periph_test.ino.elf   # run firmware
echo "AB" | node pkg/cli.mjs --config=tests/arduino_periph_test/config.yaml --max=50000000
# rebuild firmware (Windows, if arduino-cli):
arduino-cli compile --fqbn STMicroelectronics:stm32:GenF1:pnum=BLUEPILL_F103C8 --build-path tests/arduino_periph_test/build tests/arduino_periph_test
# browser demo:
python -m http.server -d pkg   # then open localhost:8000
```

### Disassembly for ISR debugging (Windows PowerShell)
```
arm-none-eabi-objdump -d tests/arduino_periph_test/build/arduino_periph_test.ino.elf > isr.asm
# find HAL_I2C_EV_IRQHandler / HAL_I2C_Master_*_IT symbols
```

## Next Phase — What's Left

### Immediate (get 21/21)
1. **Finish I2C EEPROM read** — the `Mode` patch (#3 above) was committed but **not yet re-verified after the last edit** (removed `size===1` check on the DR-write hook). Re-run test first; if FF persists, verify with logs: (a) `hi2c->XferOptions`/`XferCount`/`XferSize`/`pBuffPtr` (offset 0x50/0x2A/0x28/0x24) — HAL ISR may STOP because count==0; (b) `PreviousState`/`State` transitions
2. **Touchscreen test** (currently not reached, expected SPI values wrong: `x=2992 y=4095 p=0`) — after I2C passes, debug SPI touch readings
3. **Async section**: DMA TX/RX, UART RX, TIM2, EXTI0, SysTick — after the above; `A` byte is reserved for DMA RX in this section
4. **Remove all debug instrumentation** before finishing: `i2clog!`/`[I2C]`/`[NVIC]` logs, `I2C_LOG/i2c_log/drain_i2c_log`, `flash_debug/flash_cs_count/flash_trace`, cli.mjs `[IRQ]` prints, and the `nvics` `debug_log` — keep machine-parseable `[name] PASS/FAIL`

### Known issue
- WASM abort (`Fatal: undefined Stack: undefined`) at ~35M+ instructions (seen once) — investigate if it re-occurs after other fixes

## Next Phase — Long-term Optimizations
1. **Single WASM module** (Emscripten): compile Rust peripheral code + Unicorn C into one `emcc` output (Linux toolchain; `wasm32-unknown-emscripten` target). Recommended-free approach elsewhere in docs
2. **Replace mem hooks with shared linear memory**: `uc_mem_map_ptr(mem, periph_range)` → Rust reads/writes same region, zero crossing
3. **DMA + interrupts fully in Rust** (no JS round-trip; `uc_intr` or stop+re-exec)
4. **Alternative: pure-Rust Cortex-M emulator** (cargo-cortex-m / mdl) — evaluate vs porting Unicorn

## Files Most Relevant
- `src/peripherals/i2c.rs` — I2C state machine (the failing read)
- `src/lib.rs` — WASM API; debug log; new exports
- `pkg/cli.mjs` — DR Mode patch, workarounds, loop
- `src/ext_devices/spi_flash.rs`, `src/peripherals/spi.rs` — touchscreen SPI reads
- `tests/arduino_periph_test/` — the 21-test firmware + config