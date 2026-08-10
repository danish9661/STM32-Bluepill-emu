# STM32 Bluepill WASM Emulation — Context File

## Project Overview
Full-system emulation of an STM32F103C8 (Bluepill) microcontroller running real Arduino firmware. Two modules bridge through JavaScript:
1. **Unicorn ARM** (`pkg/unicorn_arm.cjs`) — ARM Cortex-M3 CPU emulator (binary Node addon, unmodifiable)
2. **Rust Peripherals** WASM (`pkg/stm32_bluepill_wasm_bg.wasm`) — GPIO, USART, TIM, SPI, I2C, DMA, RTC, CRC, CAN, NVIC, EXTI, ADC, DAC, FLASH, PWR, BKP, IWDG, WWDG, etc.

> **Staging rule (CI incident 2026-08-09):** always `git add -A` or stage BOTH `pkg/` and `site/` together. Commit `7040bd0` staged only `site/` (fresh `pkg/emulator.js` stayed uncommitted at 100K batches) — CI's `cmp pkg/emulator.js site/emulator.js` guard failed on the next commit and caught it. Local working trees mask this; fresh checkouts don't.

> **SVD vs hardcoded layout (verified 2026-08-09):** `Peripherals::from_svd()` (via `init_svd`, used by cli.mjs's config.yaml) registers REAL STM32F103 addresses (DMA1@0x40020000, CAN1@0x40006400). The fallback `Peripherals::new()` (via `init()`, used by emulator.js when no `svd` option is passed) uses the quirky hardcoded table in `src/peripherals/mod.rs` where **DMA1 = 0x40006000** (the real CAN1 address!) and CAN1 = 0x40006400. Real-address firmware (arduino_periph_test writes DMA1_B=0x40020000) FAILS its DMA tests under the no-SVD map — writes silently dropped. site/index.html fetches STM32F103.svd and passes it, so the browser demo is fine; any standalone `createEmulator()` smoke test must pass `svd` too.

## Architecture & Emulation Loop
```
┌─────────────────────────── JS (pkg/cli.mjs) ─────────────────────────┐
│                                                                      │
│  HOOKLESS instruction counting — emu_start(begin,0,0,maxBatch)      │
│  stops exactly at maxBatch (faults: ~0.01% of batches, skip+credit) │
│  memReadHook / memWriteHook → periph_read / periph_write  [JIT]     │
│                                                                      │
│  Loop (each iteration = 1 batch):                                    │
│    1. pump stdin → uart_rx_byte()                                    │
│    2. processDma()            ← move queued DMA data via Unicorn    │
│    3. uc.emu_start(pc|1, 0, 0, maxBatch=20K)                        │
│    4. step_batch(batchInstCount)   ← Rust ticks peripherals         │
│       - status==1 → watchdog reset requested → stop                 │
│    5. processDma()                                                  │
│    6. processInterrupts()  ← up to 16 IRQs per batch                │
│    7. is_watchdog_reset_requested() check                           │
│                                                                      │
│  DMA crosses the WASM boundary:                                      │
│    Rust queues DmaTransfer → JS dma_get_all_pending() →              │
│    periph bytes pumped in Rust (dma_absorb_periph / dma_push_periph),│
│    RAM moves via uc.mem_read/mem_write → dma_set_completed_many()    │
└──────────────────────────────────────────────────────────────────────┘
```

### Performance
- ~22M IPS real-world (200M instructions in ~9.0s; browser periph39 full run: 0.5s wall)
- **step_batch ticks once per batch, not per instruction** (`src/lib.rs`): all peripheral `tick()`s are instruction-delta based, so advancing INSTRUCTION_COUNT by `count` + one `sys.tick()` is equivalent but ~100K× cheaper — was ~55% of runtime (wasm-function[36]/[364] under `step_batch` in cpu-prof); **3.8× speedup** (21.2s → 5.6s for 100M). Requires per-batch tickers to process ALL accumulated ticks — `tim.rs advance()` had a `ticks.min(1000)` cap that dropped timer events (TIM2 IRQ never fired: CNT stuck at 12K of ARR=36K); removed.
- Peripheral access hooks are NOT a bottleneck anymore: measured 0.001 accesses/instruction (~27K per 50M instr) for the periph37 firmware
- `step_batch()` gave 3.15× speedup over per-instruction `step()`
- `has_tick` flag: 69% tick speedup; `tick_indices` Vec + `AtomicU32` DMA bitmask: minor gains
- **instCount as plain number, not BigInt** (cli.mjs + pkg/emulator.js): ~19% faster full run (48.3s → 39.1s); BigInt ops per instruction were measurable at 5M instr/sec. `maxInst` compare + `step_batch` arg are now numbers too. Same change in emulator.js lifted the browser demo from 3.8M → 5.0M Avg IPS (~30%)
- **Hookless instruction counting** (cli.mjs + pkg/emulator.js): the per-instruction JS codeHook (2 increments) cost ~20% of runtime — measured by running 200M with the hook removed (10.86s → 8.7–9.1s; ~18.5 → ~22M IPS). Since `emu_start(begin,0,0,maxBatch)` stops exactly at maxBatch, each batch is credited in full: exact for normal batches; a faulted batch (unmapped access, ~0.01% of batches — 1 in 9988 measured) is skipped (PC+2) and credited full anyway, overcounting <1 batch — invisible. Handler runs inside `processInterrupts` are not credited (instruction-delta peripherals self-correct; canary stays 39/39). This also settles the "single WASM module / C-level codeHook" idea: the JS boundary was the whole cost, and it's now gone without any rebuild
- **Batch size 20K** (cli.mjs, pkg/emulator.js DEFAULT_MAX_BATCH, site/index.html runLoop): was 100K (legacy from the slow-tick era). Per-batch tick is now cheap, so 5× smaller batches cut IRQ/interrupt delivery latency (~5.4ms → ~1.1ms) at zero measurable cost — 200M run 10.86s vs 10.8s baseline; canary still 39/39. More batch crossings = better UART RX/TIM/EXTI response in the browser demo (live per-frame UART render)
- **Site runLoop**: batch ~4× `step(20000)` per rAF frame (80ms budget), one UI pass per frame; Speed stat divides real frame instructions, not a fixed 500K
- **Regression canary**: `node tests/canary.mjs` (or `node tests/canary.mjs <maxInstr>` default 100M) — runs firmware, asserts exit 0, no FAIL lines, SUMMARY pass=39 fail=0, ~25s. Faster than the full 200M run.

## Current Status (uncommitted WIP — see "What We Did — Current Sprint" below)
### Test suite: `node tests/test_all.mjs`
**210/210 unit tests PASS** (GPIO incl. electrical model, USART, ADC incl. RC sample-and-hold / DAC loopback / external triggers / AWD IRQ, RCC, SysTick, TIM, IWDG, NVIC, CRC, SPI, I2C, RTC, PWR, FLASH, CAN, DMA, AFIO, EXTI, BKP, DAC, TIM6, RTC Alarm, UART RX, FSMC, deep-sleep gating, fault escalation).

### Firmware test — `tests/arduino_periph_test/` (24-peripheral Arduino sketch, 39 checks)
```
echo -n "AB" | node pkg/cli.mjs --config=tests/arduino_periph_test/config.yaml --max=200000000     # ~9s, 10000 steps
```
- **PASS (39/39)**: sync section (GPIO, USART TX, UART Loopback, RCC, FLASH, PWR, BKP, IWDG, WWDG, RTC, CRC, DAC, ADC, AFIO, EXTI reg, CAN, SPI Flash, I2C EEPROM, I2C OLED, touchscreen, LCD, I2C2 EEPROM, SPI2 Flash, USART2 Loopback, SVC) + async section (DMA TX/RX, UART RX, TIM2, EXTI0, EXTI1, EXTI13, CAN RX, SysTick, TIM3 PWM, TIM4, RTC Alarm IRQ, PendSV)
- **SVC + PendSV test**: `testSVC()` in setup() does `__asm volatile("svc #2")` (fires synchronously mid-batch via the JS INTR hook), sets SHPR3 (SVCall=0x40, PendSV=0x80), then pendSVC via ICSR PENDSVSET; PendSV fires at the next batch boundary.
- **CAN RX injection**: cli.mjs polls the firmware's `canRxArmed` RAM global (symbol from ELF), then calls `can_inject_message(0x40006400, 0<<21, 2, 0xDEAD, 0)`. Note the firmware's filter bank 0 is ID-list mode (FS1R=1, FM1R=0, F0=0) → only STDID **0** matches — inject ID 0, not 0x123.
- **Batch-boundary timing**: emulator ticks peripherals only in `step_batch()` between `emu_start` batches — never mid-batch. Any test that reads CNT/SR/IRQ flags after a `spin()` must be async-style (arm once, poll across batches), else it sees CNT=0. Exception: `svc` fires synchronously inside a batch.
- **Important**: 50M instr cap stops mid-print (not a deadlock); use `--max=200000000` for the full run. `A` (0x41) is reserved for the DMA RX test; `B` is the UART RX byte. `uart_rx_pending()` gate in cli.mjs prevents `A` from being consumed by the UART RX test.
- **USART TX test notes**: firmware test polls SR TXE up to 2M iterations. In emulator TXE re-asserts at batch boundaries, DRW per ISR run → ~1 byte / 100K-instr batch (not byte_time at 6250). Poll of 100K iters previously failed because the 22-byte drain needs ~2.2M instructions. Real HW at 115200: ~1.9ms drain, well under 2M-iteration budget.
- **I2C2/SPI2 devices**: `build/eeprom2.bin` (0x51, 64K) + `build/spi_flash2.bin` (JEDEC `0xEF4017`, CS PB12) — both must be re-created after an arduino-cli rebuild (build dir gets wiped):
  `node -e "const fs=require('fs'); const e2=Buffer.alloc(65536); e2[0]=0x42; e2[1]=0x24; fs.writeFileSync('tests/arduino_periph_test/build/eeprom2.bin', e2); fs.writeFileSync('tests/arduino_periph_test/build/spi_flash2.bin', Buffer.alloc(65536));"`

## What We Did — Current Sprint (uncommitted WIP)
### 1. Real GPIO/electrical behaviour (`src/peripherals/gpio.rs`)
- **Pin-level electrical model** for IDR readback: input pull-up/down (CNF=01, ODR bit selects direction), floating input (external driver or 0), push-pull output readback (slew-aware), open-drain (low driven; high released → external pull or 0), external drivers (JS read callbacks) win over driven state, analog → 0.
- **Slew** (`GPIO_SLEW` + `gpio_set_slew(n)`): output transitions land in `pending_transitions` (pin, settle_at, old_level); IDR shows the old level until settle. Open-drain driven-low ignores external drivers (`driven_pin_level`).
- **ODR/BSRR/BRR write the full register** (input pins use ODR for pull selection — the old `output_mask` filter silently broke INPUT_PULLUP); output-mode side effects (device callbacks, EXTI) still only fire for output pins.
### 2. Sleep state timing (`src/system.rs`, `src/peripherals/scb.rs`, `src/peripherals/mod.rs`, `src/peripherals/tim.rs`)
- `SCR.SLEEPDEEP` (SCB 0xE000ED10 bit 2) → deep sleep: `system.tick()` calls `tick_frozen()` on every peripheral except RTC (0x40002800) + IWDG (0x40003000), and skips SysTick accrual.
- **New trait method `tick_frozen()`** (default no-op): instruction-delta peripherals advance their delta base WITHOUT processing state. TIM overrides it — without this, frozen timers CATCH UP on wake (a 200-tick sleep produced a +220 CNT jump).
- Wake is immediate: UART RX pends from JS at the next batch boundary.
### 3. Exceptions other than IRQs (SVC, PendSV, faults) (`src/peripherals/nvic.rs`, `src/peripherals/scb.rs`, `pkg/cli.mjs`, `pkg/emulator.js`)
- **Unicorn probe**: `svc` fires HOOK_INTR intno 2 (execution continues after svc if not redirected); `bx lr` to 0xFFFFFFF9 THROWS UC_ERR_FETCH_UNMAPPED (no hook); MODE_BIG → UC_ERR_ARCH (must use THUMB|LITTLE_ENDIAN). The old `intno === 8` INTR branch is dead code (kept intact).
- **SVC**: JS stacks a 32-byte frame (xPSR, PC+2, LR, R12, R0-3) — written to the real stack AND mirrored in a JS `svcStack` — sets LR = EXC_RETURN (0xFFFFFFF9, or 0xFFFFFFFD when `CONTROL.SPSEL`), PC = SVCall vector (exception 11 → vector_table + 44). Return: the main-loop catch sees PC in 0xFFFFFFF0..0xFFFFFFFF with svcStack non-empty → pops the mirror. Depth capped at 8.
- **PendSV**: `ICSR.PENDSVSET` (SCB write) → NVIC pending → dispatched by the normal `processInterrupts` path.
- **Faults**: unmapped faults are now REAL except the known Unicorn `bl` artifact at `HAL_NVIC_EnableIRQ` (resolved via ELF symbols; skip PC+2). `raise_fault(kind, addr)` (Rust export) sets CFSR (IBUSERR/PRECISERR/UNDEFINSTR), BFAR+BFARVALID, HFSR FORCED, and pends BusFault (-11)/UsageFault (-10) if the SHCSR enable bit is set, else **escalates to HardFault (-13)**.
- **System-handler priorities**: SCB SHPR1-3 writes route to the NVIC `sys_handler_priority[16]` (default 0x80; fixed: NMI 0, HardFault 0, MemManage 1, BusFault 2, UsageFault 3). SHCSR write mask fixed (`& 0xFFFF` dropped bit 18).
- **Fault dispatch caveat**: STM32duino's default fault handler is `while(1)` — a genuinely faulting firmware hangs the run (realistic; the artifact skip + symbol gating keeps periph39 clean). No symbol table (hex-only browser firmware) → legacy tolerant skip.
### 4. Real ADC conversion (`src/peripherals/adc.rs`)
- Full rewrite: conversion state machine with real timing (`Tconv = SMP + 12.5` cycles, 1 instr = 1 ADC cycle; SMP codes 0-7 → 14/20/26/41/54/68/84/252), per-sequence channels (SQ1-16 from SQR3/2/1, JSQ1-4), `end_at`-based completion in `tick()`, EOC per conversion unless EOCS (CR2 bit 10), STRT at sequence start, AWD vs HTR/LTR, CONT (bit 16) auto-restart, SWSTART (bit 22)/JSWSTART (bit 21), CAL/RSTCAL self-clear, ADC1→DMA1 ch1 / ADC2→DMA1 ch2 requests via the new `dma_request()` trait method.
- ADC unit test now waits `step_batch(14)` after SWSTART (was: instant EOC).
### 5. DAC→ADC loopback + ADC external triggers (`src/peripherals/dac.rs`, `src/peripherals/adc.rs`, `src/peripherals/tim.rs`, `src/peripherals/exti.rs`)
- **DAC wires its pins**: enabled DAC channels drive a 12-bit analog wire (DAC1→PA4/ch4, DAC2→PA5/ch5); `Peripherals::dac_output()` (trait method `dac_output`, default None) consulted by `Adc::channel_voltage()` with the source resolver: wired GPIO (manual) > DAC output > nominal internal > sim value.
- **External trigger machinery**: `adc_timer_trigger(sys, tim_base, ch)` (ch 4 = TRGO) + `adc_exti_trigger(sys, line)` trait methods, fanned out from `Peripherals` to every ADC (0x40012400/0x40012800). TIM emits TRGO on update when MMS=010 and CC triggers on compare matches; EXTI emits on lines 11/15 rising edges. ADC gates on EXTTRIG (CR2 bit 20) / JEXTTRIG (bit 15) and EXTSEL (bits 17-19) / JEXTSEL (bits 12-14) tables (TIM1_CC1..TIM1_TRGO, TIM2_CC2, TIM3_TRGO, TIM4_CC4; injected TIM1_CC4/TIM2_TRGO/TIM2_CC2/TIM3_CC4/TIM4_TRGO, EXTI15). New conversion only when idle.
- Timer: `base` address field added (from name, `timer_base()`); triggers fire inside `advance()`/`generate_update()` mid-batch — conversion `end_at` completes at the NEXT step_batch (conversion runs with the batch-boundary ticker: same semantics as SWSTART).
- **Fixes**: double-RefCell panic (GPIO write → EXTI → `channel_voltage` while `gpio.borrow_mut()` held) avoided by `gpio.try_borrow()` in `channel_voltage` (source is re-read at sample completion anyway); test bug (`EXTSEL=7 TIM1_TRGO` needs TIM1 CR2 MMS=010 → value 0x20 not 0x10).
### 6. Full FSMC (`src/peripherals/fsmc.rs`, `src/ext_devices/fsmc_nor.rs`)
- All 7 external-memory banks: NE1-4 @ 0x6000_0000/0x6400_0000/0x6800_0000/0x6C00_0000 (NOR), NAND2/3 @ 0x7000_0000/0x8000_0000, PC-Card @ 0x9000_0000. BCR1-4 @ 0xA000_0000.. (BTR/BWTR at +8/+10/+18), PCR/PMEM/PATT 2-4 at +0x60/+0x80/+0xA0 (+8 stride).
- NOR banks gate on BCR MBKEN (bit 0); NOR writes also need WREN (bit 1). NAND/PC always enabled. `read_sized`/`write_sized` assemble bytes per access width.
- Backing: `FsmcNor` ext device with a JS `Uint8Array` image; `add_fsmc_bank('FSMC.BANK1', data)` (must precede init()); ext_devices lookup in `find_mem_device` matches `fsmc_nors` by name (`FSMC.BANK1..7`).
### 7. IRQ delivery correctness (`pkg/cli.mjs`, `pkg/emulator.js`, `src/peripherals/nvic.rs`) [commits add9fe2, 9626233]
- **xPSR restore**: `processInterrupts` saved R0-R3,R12,LR,PC,xPSR but cli.mjs never wrote xPSR back (emulator.js's frame read-back did). A handler's emu_start clobbers APSR, so a cmp/beq pair split across a batch boundary (e.g. timer demo guard `cmp` @0x8000222, `beq` @0x8000224) evaluated with the HANDLER's flags: TIM2's ISR landing exactly there fell through a guard that should have skipped → the print body ran twice per second with a stale `now` + fresh `CNT` → every `t=Ns cnt=N+1` line duplicated. Fix: `uc.reg_write_i32(ARM_REG_XPSR, savedXPSR)` before restoring PC. This was why cli runs were dirty while emulator.js probes stayed clean.
- **SysTick debt drain**: SysTick is delivered as `irq=-1` (const `SYSTICK: i32 = -1` in nvic.rs — exception #15 via `vector_table + 4*(16+irq)`), but both JS paths drained the re-pend debt with a dead `if (irq === 15)` check — `nvic_systick_take()` never ran, so a multi-period elapsed (large SysTick debt) delivered only ONE of the owed ticks.
- **Debt accounting**: wiring the drain to `irq === -1` alone double-delivered: `maybe_set_systick_intr_pending` sets the pending bit AND adds `ticks` to debt, then `systick_take()` re-pends the same tick again (~150 deliveries/6M vs 75 = 2× fast millis). Fixed in nvic.rs: the pending IRQ covers the FIRST tick, debt holds only the remainder (`ticks.saturating_sub(1)`, or full `ticks` when a SysTick was already pending); steady state = 1 delivery per period (75/6M ≈ 83 expected at 72000-instr 1ms).
### Unit tests / firmware / misc
- `tests/test_all.mjs`: 189 → **210 PASS** (DAC→ADC loopback via DOR1/2, TIM1 TRGO/CC1 + EXTI11 external triggers, EXTI 11 → ADC without SWSTART, DMA pump exports `dma_absorb_periph`/`dma_push_periph`; AWD IRQ needs ISER enable: `can_fire` requires the IRQ enabled in the NVIC, real hardware semantics — pending without enable stays pending).
- Firmware: +SVC (synchronous, `svc #2` in setup()) +PendSV (ICSR-pended, fires next batch) → **39/39**; canary asserts 39.
- cli.mjs/emulator.js: SVC hook, EXC_RETURN pop, symbol-gated fault raise; emulator.js `faultSym` merged into the existing `resolveSymbol` path (`resolveSym` shared helper, `setSymbols` resets it).
- site/: synced (emulator.js + wasm + unicorn_arm.js), refreshed `arduino_periph_test.elf` (39 checks) + eeprom2/spi_flash2 images.
- docs/: PERIPHERALS.md (FSMC/ADC Full, GPIO electrical, sleep, exceptions, 189 tests, 39 checks), ARCHITECTURE.md (Exceptions/Sleep/GPIO/FSMC sections), USAGE.md (gpioSetSlew + fsmc ext_devices + 39), README links.
- 200M full run: 39/39 in **9.00s** (~22M IPS).

## Active Workarounds (temporary, remove or upstream later)
1. **`mrs rX, msp` → `mov rX, sp`** (cli.mjs `patchMrsMsp`, ~line 19): Unicorn cannot decode Thumb `mrs`; newlib `_sbrk` uses it; rewrite to 4-byte equivalent + nop (same footprint)
2. **i2c_init NVIC patch** (cli.mjs ~line 252): patch offset 0x8001BBC (block 0x8001BBC–0x8001BDB) replaced with inline ISER0/ISER1 writes + preserved `SetPriority` calls (Unicorn skips the two `bl HAL_NVIC_EnableIRQ`)
3. **hi2c->Mode patch** (cli.mjs `memWriteHook`): when `0x40005410` (I2C1 DR) is written with the R-bit set, patch RAM `*(0x200002d8)+0x3D` to 0x22 — HAL I2C1 ISR requires `hi2c->Mode == 0x22` (MASTER_RX) before reading DR
4. **Interrupt frame saved in JS closure variables** (not memory): stack frames get clobbered by handler PUSH; save R0-R3,R12,LR,PC,xPSR in JS locals, restore after handler — restoring xPSR is REQUIRED (see §7: the handler's emu_start clobbers APSR; dropping it mis-evaluates any cmp/beq that straddles the batch boundary)
5. **64-IRQ loop in `processInterrupts()`** (cli.mjs; emulator.js drains all pending): prevents starvation when high-priority IRQ re-pends itself — paired with the NVIC `last_popped` fairness, a hot IRQ (TXE) alternates with other pendings instead of consuming every slot (e.g. CAN TX IRQ37 prio16 vs I2C EV IRQ31 prio32)
6. **DMA**: batched `dma_get_all_pending()` / `dma_set_completed_many()` — one WASM call instead of 7; the **periph byte pump now lives in Rust** (`dma_absorb_periph(addr,size)` / `dma_push_periph(addr,bytes)`, keeping the exact per-chunk periph_read/periph_write call pattern) so JS only touches RAM: periph→mem = `uc.mem_write(dst, absorb(peri_addr,size))`, mem→periph = `push(peri_addr, uc.mem_read(src,size))`, dir 2 / non-peripheral = raw memcpy. Note: Vec<u8> returns arrive in JS as a plain number array, not Uint8Array (test_all joins bytes with String.fromCharCode)

## To Run / Rebuild
```bash
cargo check                          # Rust sanity (fast)
wasm-pack build --target web         # rebuild pkg/stm32_bluepill_wasm_bg* (Rust → wasm)
node tests/test_all.mjs              # 210 unit tests
node tests/canary.mjs                # regression canary: 39/39 firmware checks, ~25s
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
1. **Verify nothing regressed** — rerun `tests/test_all.mjs` (210) + canary (`node tests/canary.mjs`, 39/39) after any edit to `src/` or `pkg/cli.mjs`

### Known issue (monitor only)
- WASM abort (`Fatal: undefined Stack: undefined`) at ~35M+ instructions — seen once, **not reproducible** across ~2.5B stress instructions (7 runs). Re-investigate if it re-occurs

## Next Phase — Long-term Optimizations
1. **Single WASM module** (Emscripten): compile Rust peripheral code + Unicorn C into one `emcc` output (Linux toolchain; `wasm32-unknown-emscripten` target). Recommended-free approach elsewhere in docs
2. ~~**Replace mem hooks with shared linear memory**~~ — **retired (moot)**: `uc_mem_map_ptr(mem, periph_range)` would remove the JS crossing, but peripheral access was measured at 0.001 accesses/instruction (~0.1% of runtime) — no measurable win available
3. **DMA + interrupts fully in Rust** (no JS round-trip; `uc_intr` or stop+re-exec)
4. **Alternative: pure-Rust Cortex-M emulator** (cargo-cortex-m / mdl) — evaluate vs porting Unicorn

## Files Most Relevant
- `src/peripherals/i2c.rs` — I2C state machine
- `src/lib.rs` — WASM API; new exports
- `pkg/cli.mjs` — DR Mode patch, workarounds, loop, SVC hook, fault gate
- `src/peripherals/usart.rs` — TXE byte-time pacing, `rx_pending()`
- `src/ext_devices/spi_flash.rs`, `src/peripherals/spi.rs`, `src/ext_devices/touchscreen.rs` — touchscreen SPI reads (deferred_reply)
- `src/peripherals/gpio.rs` — electrical model (`pin_level()`), slew (`pending_transitions`), `read_pin_effective()`
- `src/peripherals/scb.rs` — deep sleep, SHPR routing, `raise_fault()`
- `src/peripherals/fsmc.rs`, `src/ext_devices/fsmc_nor.rs` — FSMC banks + backing
- `src/peripherals/adc.rs` — real conversion state machine
- `tests/arduino_periph_test/` — the 24-peripheral firmware (39 checks incl. SVC/PendSV) + config