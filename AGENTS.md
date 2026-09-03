# STM32 Bluepill WASM Emulation — Context File

## Project Overview
Full-system emulation of an STM32F103C8 (Bluepill) microcontroller running real Arduino firmware. Two modules bridge through JavaScript:
1. **Unicorn ARM** (`pkg/unicorn_arm.cjs`) — ARM Cortex-M3 CPU emulator (binary Node addon, unmodifiable)
2. **Rust Peripherals** WASM (`pkg/stm32_bluepill_wasm_bg.wasm`) — GPIO, USART, TIM, SPI, I2C, DMA, RTC, CRC, CAN, NVIC, EXTI, ADC, DAC, FLASH, PWR, BKP, IWDG, WWDG, etc.

> **Staging rule (CI incident 2026-08-09):** always `git add -A` or stage BOTH `pkg/` and `site/` together. Commit `7040bd0` staged only `site/` (fresh `pkg/emulator.js` stayed uncommitted at 100K batches) — CI's `cmp pkg/emulator.js site/emulator.js` guard failed on the next commit and caught it. Local working trees mask this; fresh checkouts don't.

> **SVD vs hardcoded layout (FIXED 2026-08-11):** both maps now agree on real STM32F103 addresses (DMA1@0x40020000, DMA2@0x40020400, CAN1@0x40006400) — the hardcoded `init()` board no longer puts DMA1 at 0x40006000, so real-address firmware passes under EITHER path. `from_svd()` also auto-registers the ARM core peripherals (NVIC/SysTick/SCB at their fixed 0xE000Exxx addresses) when an SVD omits them (STM32F105xx.svd has no SCB/SysTick — without the fallback, millis()/SysTick and PendSV silently break).

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
│    6. processInterrupts()  ← up to 64 IRQs per batch (intr_next)      │
│    7. is_watchdog_reset_requested() check                           │
│                                                                      │
│  DMA crosses the WASM boundary:                                      │
│    Rust queues DmaTransfer → dma_pump_all() pops the whole queue,     │
│    absorbs/pushes periph bytes internally (dma_absorb_store /         │
│    dma_push_periph) and returns a flat op plan for JS: [op,a,b,c]:    │
│    op0=RAM memcpy, op1=store absorbed (dma_take_absorbed),            │
│    op2=read RAM then push, op3=done bits (dma_set_completed_many);    │
│    JS only touches Unicorn RAM via uc.mem_read/mem_write              │
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
- **Closed-form timer advance** (2026-08-13): `tim.rs advance()` was the only remaining O(ticks) loop — it iterated every accumulated tick (3 active timers × 20K ticks × 4 channels ≈ 2.4B compare checks per 200M run = 14.5% of runtime, 1409ms of 9.7s). Rewritten to jump directly to event ticks (update wrap + CCx compare matches) with bit-identical event sets: final CNT, UIF/CCxIF/TRGO/IRQ pendings all match the per-tick code exactly (same tick_once body at event ticks; no-event ticks skipped — CNT is only observable at batch boundaries, events only pend into batch-boundary queues). Down/up/center-aligned modes preserved (down = cms==0 && dir==1; ccr==0-down-at-cnt==0 edge case: the wrap tick can't match_down, handled). step_batch: 1409ms → **11ms** (124×); 200M run 9.54s → **8.32s (~24M IPS)**; Unicorn TCG is now ~97.5% of runtime — the JS/Rust layer is exhausted. Verified: 236/236, canary 39/39, emulator.js 200M, formats 14/14, ESM 4/4, all bare ELFs A/B-tested vs old wasm (no regression; their ~22s is pre-existing workload behavior), browser CDP smoke live (22 frames rainbow).
- **Batch size 20K** (cli.mjs, pkg/emulator.js DEFAULT_MAX_BATCH, site/index.html runLoop): was 100K (legacy from the slow-tick era). Per-batch tick is now cheap, so 5× smaller batches cut IRQ/interrupt delivery latency (~5.4ms → ~1.1ms) at zero measurable cost — 200M run 10.86s vs 10.8s baseline; canary still 39/39. More batch crossings = better UART RX/TIM/EXTI response in the browser demo (live per-frame UART render)
- **Site runLoop**: batch ~4× `step(20000)` per rAF frame (80ms budget), one UI pass per frame; Speed stat divides real frame instructions, not a fixed 500K
- **Regression canary**: `node tests/canary.mjs` (or `node tests/canary.mjs <maxInstr>` default 100M) — runs firmware, asserts exit 0, no FAIL lines, SUMMARY pass=39 fail=0, ~25s. Faster than the full 200M run.

## Current Status (all work below is committed; see git log)

> Last updated: 2026-08-22. The emulator is **feature-complete and stable**:
> 236/236 unit tests, 39/39 firmware checks, ~22M IPS headless. Recent work:
> `--help`/`--verbose` CLI + better errors, comprehensive About page, **removed all
> `panic!` from user-input paths** (bad pin names / empty bus ranges now degrade
> gracefully instead of aborting the WASM module), and an audit document
> (`docs/AUDIT.md`) covering memory, security, overhead and performance.
### Test suite: `node tests/test_all.mjs`
**354/354 unit tests PASS** (GPIO incl. electrical model + pin events, USART, ADC incl. RC sample-and-hold / DAC loopback / external triggers / AWD IRQ, RCC incl. clock decode, SysTick, TIM, IWDG, WWDG EWI, NVIC, CRC, SPI, I2C, RTC incl. second/overflow + flags, PWR incl. PVD, FLASH, CAN, DMA, AFIO, EXTI, BKP incl. tamper, DAC, TIM6, RTC Alarm, UART RX, FSMC, SDIO incl. MMC, USB, deep-sleep gating, fault escalation).

### Firmware test — `tests/arduino_periph_test/` (24-peripheral Arduino sketch, 39 checks)
```
echo -n "AB" | node pkg/cli.mjs --config=tests/arduino_periph_test/config.yaml --max=200000000     # ~9s, 10000 steps
```
- **PASS (39/39)**: sync section (GPIO, USART TX, UART Loopback, RCC, FLASH, PWR, BKP, IWDG, WWDG, RTC, CRC, DAC, ADC, AFIO, EXTI reg, CAN, SPI Flash, I2C EEPROM, I2C OLED, touchscreen, LCD, I2C2 EEPROM, SPI2 Flash, USART2 Loopback, SVC) + async section (DMA TX/RX, UART RX, TIM2, EXTI0, EXTI1, EXTI13, CAN RX, SysTick, TIM3 PWM, TIM4, RTC Alarm IRQ, PendSV)
- **SVC + PendSV test**: `testSVC()` in setup() does `__asm volatile("svc #2")` (fires synchronously mid-batch via the JS INTR hook), sets SHPR3 (SVCall=0x40, PendSV=0x80), then pendSVC via ICSR PENDSVSET; PendSV fires at the next batch boundary.
- **CAN RX injection**: cli.mjs polls the firmware's `canRxArmed` RAM global (symbol from ELF), then calls `can_inject_message(0x40006400, 0<<21, 2, 0xDEAD, 0)`. Note the firmware's filter bank 0 is ID-list mode (FS1R=1, FM1R=0, F0=0) → only STDID **0** matches — inject ID 0, not 0x123. **Never hardcode the flag address**: 0x200000b8 silently became `canRxTries` after a rebuild — every non-CLI driver (test_emulator_js, test_browser*, page/worker autopilot) then injected a full 3M-iteration RF0R timeout late (~4s per 200M run, browser 9→22 MIPS after fix). All drivers now resolve `canRxArmed` from ELF symbols via `parseElf` at load (index.html passes it to worker as `canFlagAddr`; hex/bin keep the constant fallback).
- **Batch-boundary timing**: emulator ticks peripherals only in `step_batch()` between `emu_start` batches — never mid-batch. Any test that reads CNT/SR/IRQ flags after a `spin()` must be async-style (arm once, poll across batches), else it sees CNT=0. Exception: `svc` fires synchronously inside a batch.
- **Important**: 50M instr cap stops mid-print (not a deadlock); use `--max=200000000` for the full run. `A` (0x41) is reserved for the DMA RX test; `B` is the UART RX byte. `uart_rx_pending()` gate in cli.mjs prevents `A` from being consumed by the UART RX test.
- **USART TX test notes**: firmware test polls SR TXE up to 2M iterations. In emulator TXE re-asserts at batch boundaries, DRW per ISR run → ~1 byte / 100K-instr batch (not byte_time at 6250). Poll of 100K iters previously failed because the 22-byte drain needs ~2.2M instructions. Real HW at 115200: ~1.9ms drain, well under 2M-iteration budget.
- **I2C2/SPI2 devices**: `build/eeprom2.bin` (0x51, 64K) + `build/spi_flash2.bin` (JEDEC `0xEF4017`, CS PB12) — both must be re-created after an arduino-cli rebuild (build dir gets wiped):
  `node -e "const fs=require('fs'); const e2=Buffer.alloc(65536); e2[0]=0x42; e2[1]=0x24; fs.writeFileSync('tests/arduino_periph_test/build/eeprom2.bin', e2); fs.writeFileSync('tests/arduino_periph_test/build/spi_flash2.bin', Buffer.alloc(65536));"`

## What We Did — Current Sprint (committed)
### 0. rp2040js-style peripheral bus + custom JS peripherals + multi-chip (`src/bus.rs`, `src/peripherals/mod.rs`, `pkg/cli.mjs`, `pkg/emulator.js`, site/index.html)
- **Bus**: new `src/bus.rs` — runtime registry (rp2040js `bus.ts` equivalent): `Bus::register(start, end, tick, p)`, sorted slots + binary search (`get()`), tick bookkeeping rebuilt on every register. `Peripherals.peripherals`/`tick_indices` → `bus: RefCell<Bus>`; `PeripheralSlot` gained a `tick` flag. **Last registration wins on overlap** (custom peripherals can shadow built-ins).
- **JS peripherals**: `JsPeripheral` (impl `Peripheral`, holds `js_sys::Function`s) + wasm export `register_js_peripheral(base, size, read, write) -> bool` — callbacks get `(addr, size)` / `(addr, value, size)` with the ABSOLUTE address; requires init first; cleared by the next init (fresh bus). emulator.js: `emu.addJsPeripheral(...)` + `opts.js_peripherals`; cli.mjs: `--periph-plugin=<file.mjs>` (default export array of {base,size,read,write}).
- **Hardcoded table fixed to real F103 addresses**: DMA1 0x40006000 → **0x40020000**, +DMA2 0x40020400 (clock_enabled had the right gates already; `dma_request()` lookup updated) — the SVD-vs-hardcoded dual-map bug class is GONE; real-address firmware passes under either path. CAN MCR write mask corrected: 0x7F3F → 0x180FF (INRQ..TTCM 0-7, RESET 15, DBF 16 — ABOM bit 6 was silently dropped).
- **Multi-chip**: `from_svd()` now auto-registers the ARM core peripherals (NVIC/SysTick/SCB at fixed 0xE000Exxx addresses) when the SVD omits them — **STM32F105xx.svd has no SCB/SysTick**, so millis()/PendSV silently broke until the fallback (verified: browser periph37 on F105 = 39/39, CAN2@0x40006800 live). Unsupported SVD peripherals (ETH) skipped by name. Ship `svd/STM32F105xx.svd` (connectivity line, CAN2) + page chip selector (`chipSelect`: F103C8 builtin | F105 SVD).
- **Verified**: `tests/test_all.mjs` 224/224 (+9 JS-peripheral, +5 F105); canary 39/39; 200M 39/39 @ 9.67s; browser F103C8 39/39 + F105 39/39.
- **Docs**: ARCHITECTURE.md "The peripheral bus" (layer table), USAGE.md (chip/js_peripherals/plugin), AGENTS.md SVD-note rewritten.
### 1. Stale ext-device fix — `reset_ext_devices()` (`src/lib.rs`, `pkg/cli.mjs`, `pkg/emulator.js`) [commit f05a44d]
- **Symptom**: running arduino_periph_test in the page AFTER the showcase preset → 38/39 (touchscreen FAIL). Root cause: `add_*` calls append to module-level static lists — a second `init()` keeps the showcase's devices. The stale showcase LCD (SPI1 cs **PA8**) sits BEFORE the new run's touchscreen (cs PA1) in SPI1's device list; the fresh GPIO leaves PA8 low → stale LCD selected during the touchscreen test → `read()`=0 → `[Touchscreen] FAIL`. Direct-API re-init passed because it re-added the SAME devices (fresh list order), masking the bug.
- **Fix**: new `reset_ext_devices()` wasm export (clears spi_flashes, i2c_eeproms, usart_probes, lcds, touchscreens, displays, i2c_oleds, fsmc_nors + software SPI configs); called at the top of `createEmulator()` and both cli.mjs paths (before config/bare-firmware device registration, always before `init()`). Single-instance runs unaffected.
- **Verified**: `node tests/test_all.mjs` 224/224; canary 39/39; `--max=200000000` 39/39 in 9.56s; browser periph37-after-showcase 39/39 badge Done; headless CDP button press → `t=8s btn=1` heartbeat (earlier misses were the test clicking at y=-89 — button scrolled above the viewport; `scrollIntoView` first).
### 2. Real GPIO/electrical behaviour (`src/peripherals/gpio.rs`)
- **Pin-level electrical model** for IDR readback: input pull-up/down (CNF=01, ODR bit selects direction), floating input (external driver or 0), push-pull output readback (slew-aware), open-drain (low driven; high released → external pull or 0), external drivers (JS read callbacks) win over driven state, analog → 0.
- **Slew** (`GPIO_SLEW` + `gpio_set_slew(n)`): output transitions land in `pending_transitions` (pin, settle_at, old_level); IDR shows the old level until settle. Open-drain driven-low ignores external drivers (`driven_pin_level`).
- **ODR/BSRR/BRR write the full register** (input pins use ODR for pull selection — the old `output_mask` filter silently broke INPUT_PULLUP); output-mode side effects (device callbacks, EXTI) still only fire for output pins.
### 3. Sleep state timing (`src/system.rs`, `src/peripherals/scb.rs`, `src/peripherals/mod.rs`, `src/peripherals/tim.rs`)
- `SCR.SLEEPDEEP` (SCB 0xE000ED10 bit 2) → deep sleep: `system.tick()` calls `tick_frozen()` on every peripheral except RTC (0x40002800) + IWDG (0x40003000), and skips SysTick accrual.
- **New trait method `tick_frozen()`** (default no-op): instruction-delta peripherals advance their delta base WITHOUT processing state. TIM overrides it — without this, frozen timers CATCH UP on wake (a 200-tick sleep produced a +220 CNT jump).
- Wake is immediate: UART RX pends from JS at the next batch boundary.
### 4. Exceptions other than IRQs (SVC, PendSV, faults) (`src/peripherals/nvic.rs`, `src/peripherals/scb.rs`, `pkg/cli.mjs`, `pkg/emulator.js`)
- **Unicorn probe**: `svc` fires HOOK_INTR intno 2 (execution continues after svc if not redirected); `bx lr` to 0xFFFFFFF9 THROWS UC_ERR_FETCH_UNMAPPED (no hook); MODE_BIG → UC_ERR_ARCH (must use THUMB|LITTLE_ENDIAN). The old `intno === 8` INTR branch is dead code (kept intact).
- **SVC**: Rust builds the 32-byte frame AND the mirror (`intr_svc_enter` in `src/interrupts.rs`, depth-capped at 8, shared by cli.mjs + emulator.js) — written to the real stack AND mirrored in Rust — sets LR = EXC_RETURN (0xFFFFFFF9, or 0xFFFFFFFD when `CONTROL.SPSEL`), PC = SVCall vector (exception 11 → vector_table + 44). Return: the main-loop catch sees PC in 0xFFFFFFF0..0xFFFFFFFF with `intr_svc_depth() > 0` → pops the mirror via `intr_svc_leave()`.
- **PendSV**: `ICSR.PENDSVSET` (SCB write) → NVIC pending → dispatched by the normal `processInterrupts` path.
- **Faults**: unmapped faults are now REAL except the known Unicorn `bl` artifact at `HAL_NVIC_EnableIRQ` (resolved via ELF symbols; skip PC+2). `raise_fault(kind, addr)` (Rust export) sets CFSR (IBUSERR/PRECISERR/UNDEFINSTR), BFAR+BFARVALID, HFSR FORCED, and pends BusFault (-11)/UsageFault (-10) if the SHCSR enable bit is set, else **escalates to HardFault (-13)**.
- **System-handler priorities**: SCB SHPR1-3 writes route to the NVIC `sys_handler_priority[16]` (default 0x80; fixed: NMI 0, HardFault 0, MemManage 1, BusFault 2, UsageFault 3). SHCSR write mask fixed (`& 0xFFFF` dropped bit 18).
- **Fault dispatch caveat**: STM32duino's default fault handler is `while(1)` — a genuinely faulting firmware hangs the run (realistic; the artifact skip + symbol gating keeps periph39 clean). No symbol table (hex-only browser firmware) → legacy tolerant skip.
### 5. Real ADC conversion (`src/peripherals/adc.rs`)
- Full rewrite: conversion state machine with real timing (`Tconv = SMP + 12.5` cycles, 1 instr = 1 ADC cycle; SMP codes 0-7 → 14/20/26/41/54/68/84/252), per-sequence channels (SQ1-16 from SQR3/2/1, JSQ1-4), `end_at`-based completion in `tick()`, EOC per conversion unless EOCS (CR2 bit 10), STRT at sequence start, AWD vs HTR/LTR, CONT (bit 16) auto-restart, SWSTART (bit 22)/JSWSTART (bit 21), CAL/RSTCAL self-clear, ADC1→DMA1 ch1 / ADC2→DMA1 ch2 requests via the new `dma_request()` trait method.
- ADC unit test now waits `step_batch(14)` after SWSTART (was: instant EOC).
### 6. DAC→ADC loopback + ADC external triggers (`src/peripherals/dac.rs`, `src/peripherals/adc.rs`, `src/peripherals/tim.rs`, `src/peripherals/exti.rs`)
- **DAC wires its pins**: enabled DAC channels drive a 12-bit analog wire (DAC1→PA4/ch4, DAC2→PA5/ch5); `Peripherals::dac_output()` (trait method `dac_output`, default None) consulted by `Adc::channel_voltage()` with the source resolver: wired GPIO (manual) > DAC output > nominal internal > sim value.
- **External trigger machinery**: `adc_timer_trigger(sys, tim_base, ch)` (ch 4 = TRGO) + `adc_exti_trigger(sys, line)` trait methods, fanned out from `Peripherals` to every ADC (0x40012400/0x40012800). TIM emits TRGO on update when MMS=010 and CC triggers on compare matches; EXTI emits on lines 11/15 rising edges. ADC gates on EXTTRIG (CR2 bit 20) / JEXTTRIG (bit 15) and EXTSEL (bits 17-19) / JEXTSEL (bits 12-14) tables (TIM1_CC1..TIM1_TRGO, TIM2_CC2, TIM3_TRGO, TIM4_CC4; injected TIM1_CC4/TIM2_TRGO/TIM2_CC2/TIM3_CC4/TIM4_TRGO, EXTI15). New conversion only when idle.
- Timer: `base` address field added (from name, `timer_base()`); triggers fire inside `advance()`/`generate_update()` mid-batch — conversion `end_at` completes at the NEXT step_batch (conversion runs with the batch-boundary ticker: same semantics as SWSTART).
- **Fixes**: double-RefCell panic (GPIO write → EXTI → `channel_voltage` while `gpio.borrow_mut()` held) avoided by `gpio.try_borrow()` in `channel_voltage` (source is re-read at sample completion anyway); test bug (`EXTSEL=7 TIM1_TRGO` needs TIM1 CR2 MMS=010 → value 0x20 not 0x10).
### 7. Full FSMC (`src/peripherals/fsmc.rs`, `src/ext_devices/fsmc_nor.rs`)
- All 7 external-memory banks: NE1-4 @ 0x6000_0000/0x6400_0000/0x6800_0000/0x6C00_0000 (NOR), NAND2/3 @ 0x7000_0000/0x8000_0000, PC-Card @ 0x9000_0000. BCR1-4 @ 0xA000_0000.. (BTR/BWTR at +8/+10/+18), PCR/PMEM/PATT 2-4 at +0x60/+0x80/+0xA0 (+8 stride).
- NOR banks gate on BCR MBKEN (bit 0); NOR writes also need WREN (bit 1). NAND/PC always enabled. `read_sized`/`write_sized` assemble bytes per access width.
- Backing: `FsmcNor` ext device with a JS `Uint8Array` image; `add_fsmc_bank('FSMC.BANK1', data)` (must precede init()); ext_devices lookup in `find_mem_device` matches `fsmc_nors` by name (`FSMC.BANK1..7`).
### 8. IRQ delivery correctness (`pkg/cli.mjs`, `pkg/emulator.js`, `src/peripherals/nvic.rs`) [commits add9fe2, 9626233]
- **xPSR restore**: `processInterrupts` saved R0-R3,R12,LR,PC,xPSR but cli.mjs never wrote xPSR back (emulator.js's frame read-back did). A handler's emu_start clobbers APSR, so a cmp/beq pair split across a batch boundary (e.g. timer demo guard `cmp` @0x8000222, `beq` @0x8000224) evaluated with the HANDLER's flags: TIM2's ISR landing exactly there fell through a guard that should have skipped → the print body ran twice per second with a stale `now` + fresh `CNT` → every `t=Ns cnt=N+1` line duplicated. Fix: restore xPSR from the stacked frame before restoring PC. Since §9 both files restore from the frame (handler edits to the saved context are honored).
- **SysTick debt drain**: SysTick is delivered as `irq=-1` (const `SYSTICK: i32 = -1` in nvic.rs — exception #15 via `vector_table + 4*(16+irq)`), but both JS paths drained the re-pend debt with a dead `if (irq === 15)` check — `nvic_systick_take()` never ran, so a multi-period elapsed (large SysTick debt) delivered only ONE of the owed ticks.
- **Debt accounting**: wiring the drain to `irq === -1` alone double-delivered: `maybe_set_systick_intr_pending` sets the pending bit AND adds `ticks` to debt, then `systick_take()` re-pends the same tick again (~150 deliveries/6M vs 75 = 2× fast millis). Fixed in nvic.rs: the pending IRQ covers the FIRST tick, debt holds only the remainder (`ticks.saturating_sub(1)`, or full `ticks` when a SysTick was already pending); steady state = 1 delivery per period (75/6M ≈ 83 expected at 72000-instr 1ms).
### 9. Interrupt dispatch policy in Rust + CI (`src/interrupts.rs`, `pkg/cli.mjs`, `pkg/emulator.js`, `.github/workflows/test.yml`)
- **New `src/interrupts.rs`** (`IntrDispatch` state on `WasmSystem`): owns everything about interrupt delivery that isn't pure Unicorn transport — the per-batch 64-IRQ budget (`intr_next()`, reset inside `step`/`step_batch`, -255 when exhausted) and the SVC frame mirror (`intr_svc_enter` returns the 32-byte Cortex-M frame and pushes the mirror, depth-capped at 8; `intr_svc_leave` pops to `[r0,r1,r2,r3,r12,lr,pc,sp]`; `intr_svc_depth` guards the JS catch). Registers/vector fetch/handler `emu_start` stay in JS (Unicorn-bound — see NEXT_PHASE.md §2).
- **One dispatch implementation**: cli.mjs and emulator.js now use identical code — `intr_next()` loop (both had different loop shapes: cli capped at 64 with a for-loop, emulator drained all pending), SVC hook via `intr_svc_enter`, catch via `intr_svc_depth()/intr_svc_leave()`. Both also unified on **restore-from-stacked-frame** (was: cli restores from JS locals, emulator from frame read-back) — handler edits to the saved context are now honored in both, and the xPSR restore requirement is covered by the frame.
- **Batch register transport** (regsRead/regsWrite helpers, identical in cli.mjs + emulator.js, keep in sync): unicorn_arm exposes raw `uc_reg_read_batch`/`uc_reg_write_batch` without marshalling — the helpers allocate id/pointer/value arrays so an IRQ dispatch crosses the addon boundary 2× instead of ~17×. Dispatch measured at 1.3% of runtime (124ms of 9.29s for 6198 IRQs in 200M), so this is a correctness-adjacent micro-opt, not a speed lever. **Regression fixed in the same change**: e113a74's unification dropped the XPSR write that add9fe2 had added (restore-from-frame wrote R0-R3/R12/LR/PC/SP but not XPSR, despite the comment claiming it was required — the canary misses it because periph39 doesn't straddle a cmp/beq pair across a batch boundary like the timer demo does). Restore now writes XPSR from frame offset 0, and the intrHook intno-8 (bx lr EXC_RETURN pop) also restores XPSR (real hardware does).
- **CI** (`.github/workflows/test.yml`): `cmp` guard extended to all three `pkg`↔`site` artifacts (emulator.js, stm32_bluepill_wasm.js, stm32_bluepill_wasm_bg.wasm), stale "37/37" canary comment → 39/39, added the full 200M config run (`echo -n "AB" | node pkg/cli.mjs --config=... --max=200000000`) after the canary. **The wasm guard compares the CODE section byte-for-byte** (the 7040bd0 failure mode — stale/missing site artifact — still fails because the code differs). **Determinism root cause (2026-08-14)**: the data section embeds 81 panic-location file paths (`$CARGO_HOME/registry/src/index.crates.io-.../...`) — `/home/<user>` differs per machine (CI runner = `/home/runner`), shifting the data section (−320 B here) and one i32.const data-offset constant by the same amount → code sections "differed at byte 100" despite identical toolchains (rustc 1.97.1 / wasm-pack 0.14.0 / wasm-bindgen-cli 0.2.126 / wasm-opt 132 were all verified byte-identical; wasm-bindgen-cli version drift was a red herring). **Fix**: build with `RUSTFLAGS="--remap-path-prefix=$HOME=/build"` locally and `--remap-path-prefix=/home/runner=/build` in the workflow env → byte-identical artifacts on any machine (verified: local md5 == CI md5 `ab25282f...`, 1396229 bytes). The two JS artifacts stay byte-exact. CI pins `wasm-pack@0.14.0` + binaryen `version_132` (download+PATH step) so the wasm-opt step matches local builds; rustc@stable both sides.
- **Verified**: `tests/test_all.mjs` 224/224; canary 39/39; cli 200M 39/39 @ 9.64s; emulator.js (browser path, node smoke) 200M 39/39 incl. SVC/PendSV/EXTI/TIM — same run shape as the page's run loop.
### 10. Emulator.js path in CI + stale test fixes (`tests/test_emulator_js.mjs`, `tests/test_esm.mjs`, `tests/test_firmware_formats.mjs`, `.github/workflows/test.yml`)
- **New `tests/test_emulator_js.mjs`**: drives `createEmulator` + `run()` (the exact page code path, previously only manually verified) with the periph_test firmware + devices, UART RX "AB", CAN autopilot (chunked runs polling `canRxArmed` @ 0x200000b8 like site/index.html:537), asserts no FAIL + SUMMARY 39/39 — 200M in ~10s. Added to CI after the cli 200M step.
- **test_esm.mjs fixed**: pointed at the current wasm glue (`stm32_periph_wasm.js` → `stm32_bluepill_wasm.js`), made a real pass/fail with exit code (ESM glue loads + Unicorn boots a cortex-m instance).
- **test_firmware_formats.mjs fixed**: was committed-but-broken (needed a `comprehensive_test` ELF that no longer ships). Rewritten self-contained on the arduino_periph_test artifacts (hex + map committed, ELF copied by CI from site/) with cross-format consistency checks — hex SP == map _estack, hex/elf reset == map Reset_Handler, symbols present, garbage rejection — no hardcoded addresses (those drifted when the sketch/core changed).
- **CI**: added "Firmware format + ESM smoke tests" step + "Emulator.js path (browser run loop) 200M firmware run" step.
- Path A (single wasm module) documented as an experiment in docs/NEXT_PHASE.md §4 — see "Next Phase — Long-term Optimizations" item 1.
### Unit tests / firmware / misc
- `tests/test_all.mjs`: 189 → **224 PASS** (DAC→ADC loopback via DOR1/2, TIM1 TRGO/CC1 + EXTI11 external triggers, EXTI 11 → ADC without SWSTART, DMA pump exports `dma_absorb_periph`/`dma_push_periph`; AWD IRQ needs ISER enable: `can_fire` requires the IRQ enabled in the NVIC, real hardware semantics — pending without enable stays pending).
- Firmware: +SVC (synchronous, `svc #2` in setup()) +PendSV (ICSR-pended, fires next batch) → **39/39**; canary asserts 39.
- cli.mjs/emulator.js: SVC hook, EXC_RETURN pop, symbol-gated fault raise; emulator.js `faultSym` merged into the existing `resolveSymbol` path (`resolveSym` shared helper, `setSymbols` resets it).
- site/: synced (emulator.js + wasm + unicorn_arm.js), refreshed `arduino_periph_test.elf` (39 checks) + eeprom2/spi_flash2 images.
- docs/: PERIPHERALS.md (FSMC/ADC Full, GPIO electrical, sleep, exceptions, 224 tests, 39 checks), ARCHITECTURE.md (Exceptions/Sleep/GPIO/FSMC sections), USAGE.md (gpioSetSlew + fsmc ext_devices + 39), README links.
- 200M full run: 39/39 in **9.00s** (~22M IPS).

## Active Workarounds (temporary, remove or upstream later)
1. **`mrs rX, msp` → `mov rX, sp`** (cli.mjs `patchMrsMsp`, ~line 19): Unicorn cannot decode Thumb `mrs`; newlib `_sbrk` uses it; rewrite to 4-byte equivalent + nop (same footprint)
2. **i2c_init NVIC patch** (cli.mjs ~line 252): patch offset 0x8001BBC (block 0x8001BBC–0x8001BDB) replaced with inline ISER0/ISER1 writes + preserved `SetPriority` calls (Unicorn skips the two `bl HAL_NVIC_EnableIRQ`)
3. **hi2c->Mode patch** (cli.mjs `memWriteHook`): when `0x40005410` (I2C1 DR) is written with the R-bit set, patch RAM `*(0x200002d8)+0x3D` to 0x22 — HAL I2C1 ISR requires `hi2c->Mode == 0x22` (MASTER_RX) before reading DR
4. **Interrupt frame restored from the stacked frame** (not JS locals): the frame at SP-32 is written before the handler runs and read back after (both cli.mjs + emulator.js, since §9) — restoring xPSR is REQUIRED (see §8: the handler's emu_start clobbers APSR; dropping it mis-evaluates any cmp/beq that straddles the batch boundary)
5. **64-IRQ budget in `intr_next()`** (src/interrupts.rs, reset by step/step_batch): prevents starvation when a high-priority IRQ re-pends itself — paired with the NVIC `last_popped` fairness, a hot IRQ (TXE) alternates with other pendings instead of consuming every slot (e.g. CAN TX IRQ37 prio16 vs I2C EV IRQ31 prio32)
6. **DMA**: the **whole pump loop lives in Rust** — `dma_pump_all()` pops the queue, absorbs periph→mem bytes internally (`dma_absorb_store`/`dma_take_absorbed` side buffer) and returns a flat op plan for JS (`[op,a,b,c]`: 0=RAM memcpy, 1=store absorbed, 2=read RAM then `dma_push_periph`, 3=done bits → `dma_set_completed_many`) — JS only executes `uc.mem_read`/`mem_write` on Unicorn RAM, exactly one crossing per RAM op. Completion is signaled LAST (op 3) so TC IRQs fire only after data lands. The `dma_absorb_periph`/`dma_push_periph` chunked-call pattern is preserved for tests. ISR return is one Rust call: `finish_interrupt(irq)` = `clear_current_interrupt()` + SysTick debt drain (JS `nvic_systick_take` loop gone). Note: Vec<u8> returns arrive in JS as a plain number array, not Uint8Array (test_all joins bytes with String.fromCharCode)

## To Run / Rebuild
```bash
cargo check                          # Rust sanity (fast)
PATH=<binaryen-version_132>/bin:$PATH RUSTFLAGS="--remap-path-prefix=$HOME=/build" wasm-pack build --target web   # rebuild pkg (Rust → wasm) — MUST use pinned binaryen version_132 (wasm-opt) AND the path remap, else site/ sync breaks the CI guard
# (the remap neutralizes $HOME in panic-location strings baked into the data section:
#  81 file paths like /home/<user>/.cargo/registry/src/... → the wasm byte-exact on
#  any machine — CI's runner ($HOME=/home/runner) gets the same flag in the workflow)
node tests/test_all.mjs              # 224 unit tests
node tests/canary.mjs                # regression canary: 39/39 firmware checks, ~25s
node tests/test_emulator_js.mjs      # browser run-loop path: 200M, 39/39 (~10s)
node tests/test_firmware_formats.mjs # hex/map/elf cross-format consistency
node tests/test_esm.mjs              # ESM glue + unicorn boot smoke
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
1. **Verify nothing regressed** — rerun `tests/test_all.mjs` (224) + canary (`node tests/canary.mjs`, 39/39) after any edit to `src/` or `pkg/cli.mjs`
2. **Path A spike** (next project): Rust → `wasm32-unknown-emscripten` staticlib with raw `#[no_mangle]` exports, link with emcc-compiled unicorn C, boot the firmware — see docs/NEXT_PHASE.md §4 for the acceptance gate. Dual-wasm stays the default until it passes.

### Known issue (monitor only, mostly explained)
- Historical `Fatal: undefined Stack: undefined` at ~35M+ instructions — **identified (2026-08-11)**: that text is cli.mjs's own catch handler format (`console.error('Fatal:', e.name, e.message)` + `'Stack:', ...`, present since the initial commit), not any wasm/glue string — no "Fatal:" exists in unicorn_arm.cjs/.js, stm32_bluepill_wasm.js or the .wasm. So the incident was a JS promise rejection with a nameless value (bare string/undefined; wasm-bindgen panics throw `new Error(msg)` with name+message, so a REAL Rust/wasm panic would have printed differently). Current handler is hardened (`e?.name || '(no name)'`, `Type:` dump) so a re-occurrence is now diagnosable. Not reproduced across ~6B stress instructions (22 runs, 2026-08-13: 3×200M + 2×500M + 1×1B periph39 cli, canary, emulator.js 200M browser path, showcase/ws2812/echo/fade/flash/timer_uart/adc_uart ELFs 100–200M — all exit 0, zero `Fatal`/`(no name)` in output); monitor only.

### 12. Pin-activity monitor on the demo board (site/index.html) — onPinChange live demo
- The Blue Pill board SVG now glows pins amber for ~2s after the chip drives them to a NEW level (stroke #fbbf24 + glow circle at the pin tip), tooltips show cumulative per-pin toggle counts, and a caption under the PCB shows the running total ("N pin toggles — hover a glowing pin").
- Fed by the §11 onPinChange API (drained per batch in run()/step() + before each memWriteHook's write watchers): the PIN_ACTIVITY map + onPinActivity(port, pin) subscriber, registered once per initEmulator (fresh emulator = fresh watchers) and cleared on every firmware load.
- Generic: works with any firmware/preset, not just the showcase. Verified vs the showcase firmware (30 emulated s): PC13 LED 15, PA4 7-seg CS 15, PA8 LCD CS 15, PB14 buzzer 14, PB6 I2C1 SCL 39 toggles; PB13 button (input, JS-driven) and PA5 SPI SCK (AF) correctly silent — demonstrates the exact §11 semantics live.
- index.html only; pkg/ and wasm untouched (cmp guard trivially synced).
### 13. DMA correctness fixes + WS2812 strip demo (src/peripherals/dma.rs, tests/arduino_ws2812/, site/index.html) [current sprint]
- **DMA ISR flag layout was off by one channel**: `tick()` set completion at bit `(ch+1)*4` (0-based ch); real HW puts channel N flags at `(N-1)*4..(N-1)*4+3`, TCIF_N = `(N-1)*4+1`. CH4 completion lit bit 16 instead of 13; CH3 lit 12 instead of 9. The periph39 DMA TX/RX tests passed anyway because both transfers queue in the same loop call and their completion bits landed in one shared ISR read (RX's bit 20 satisfied TX's check) — the firmware even had `/* TCIF4 (emulator layout) */` comments. Fixed to `isr |= 1 << (ch*4+1)`; firmware's TCIF4 check → bit 13 (real HW).
- **DMA direction was inverted vs CMSIS**: `do_xfer` mapped DIR=1 → `DmaDir::Read` and treated it as periph→mem absorb (pump op1), so a CMSIS-correct mem→periph channel (DIR=1) silently ABSORBED from the peripheral address instead of pushing — the WS2812 firmware's SPI1 transfer "completed" (CNDTR=0, TCIF set) but zero bytes ever reached SPI1 DR, and the JS-side `dma_push_periph` read Unicorn memory at the *peripheral* address (op2 `t.src` was CPAR, not CMAR) → dropped by the processDma catch. periph39 tests used the inverted convention (RX with DIR=1, TX with DIR=0) and only checked flags. Fixed: DIR=1 → mem→periph (src=CMAR, push to CPAR), DIR=0 → periph→mem (absorb CPAR, write CMAR), M2M (CR bit 14) → CPAR→CMAR memcpy (per RM0008). Firmware updated to CMSIS-correct bits.
- **DMA pushes bypassed onPeriphWrite**: `dma_push_periph` writes the periph bus Rust→Rust, but page-side write watchers only fire from the Unicorn memWriteHook — page decoders never saw DMA traffic (7-seg demo used direct CPU writes, so it was never hit). processDma now feeds `writeWatchers` per pushed byte for op2 (exactly one call per byte, like real HW).
- **WS2812 demo** (`tests/arduino_ws2812/` + page preset): 800kHz strip over SPI1 at 2.25MHz (div32) + DMA1 CH3 fire-and-forget; each ws bit = 3 SPI bits (0b110=1, 0b100=0), GRB, 8 LEDs = 72 bytes/frame; rainbow `hue=(frame*9 + i*45)%360`; UART `WS2812=ok` + `frames=N` every 2s. Page: preset option + LED-strip card (`wsCard`, 8 `.ws-led` divs) decoded live via `onPeriphWrite(wsWatch)` — 24 ws-bits per LED, GRB, frame counter. **8-bit DMA transfers**: the first build used PSIZE_0|MSIZE_0 (16-bit) — real HW clocks 8 bits (DS=8) per 16-bit DR write, but the emulator's `data_size = max(psize,msize)*ndtr` pushed 144 bytes/frame (misaligned decode). Dropped to 8-bit (PSIZE/MSIZE=00) → 72 bytes exact.
 - **Verified**: 236/236; canary 39/39; cli 200M 39/39; emulator.js 200M 39/39; WS2812 Node smoke (5 runs): frame0 LED0 exact red, all 40 LED-color checks across first 5 frames exact, 84 frames decoded, UART count tracks; headless CDP page smoke: preset loads, 16-34 frames decoded live, rainbow colors on the DOM strip, ALL PASS.

### 14. Ergonomic `STM32F1` wrapper + Wokwi-style virtual-peripheral event queue (`pkg/stm32f1.js`, `src/system.rs`, `src/peripherals/{usart,spi,i2c}.rs`, `src/lib.rs`, `docs/STM32F1_API.md`) [committed]
- **Wokwi-style event queue**: `WasmSystem` now holds a `RefCell<Vec<VmEvent>>` (enum `VmEvent`: `SpiTransfer{channel,tx,rx}`, `I2cStart{channel,addr}`, `I2cWrite{channel,byte}`, `I2cRead{channel}`, `I2cStop{channel}`, `UartTx{usart,byte}`). Pushed in `usart.rs::write_dr` (UartTx), `spi.rs` DR write (SpiTransfer + optional MISO inject consume), `i2c.rs` (Start/Write/Read/Stop + optional RX inject). `drain_events()` (`src/lib.rs`) flattens to an `i32[]`; `emulator.js` exposes `drainEvents()` / `spiInjectMiso(ch,bytes)` / `i2cInjectRx(ch,bytes)` / `uartRxAddr(addr,byte)`.
- **Why**: `getUartOutput()` is USART1-only and USART DR writes do NOT fire `onPeriphWrite`, so per-USART TX (e.g. USART2) could not be observed. The event queue is the transaction-level model Wokwi virtual peripherals expect and captures ALL buses.
- **`pkg/stm32f1.js`**: `STM32F1` class + `GPIO`/`GPIOPin`/`USART`/`SPI`/`I2C` wrappers. `execute()`/`step()` auto-drain events and dispatch to `gpio.pin().on('change')`, `usartN.onData`, `spiN.onTransfer(ch,tx,rx)`, `i2cN.onStart/onWrite/onRead/onStop`; `usartN.send()`, `spiN.injectMiso()`, `i2cN.injectRx()` for host→MCU injection. Thin layer, no hot-path overhead.
- **Tests**: `tests/test_stm32f1_api.mjs` (7: USART1 TX + SPI1 transfers via ws2812 elf), `tests/test_i2c_events.mjs` (3: I2C1 Start/Write/Stop via periph_test + empty-data div-by-zero guard on spi_flash requires non-empty image). Both wired into `.github/workflows/test.yml`. `docs/STM32F1_API.md` written.
- **Rebuild note**: wasm rebuilt with pinned binaryen `version_132` (downloaded to `/tmp/binaryen-version_132` locally) + `RUSTFLAGS="--remap-path-prefix=$HOME=/build"`; `pkg/` and `site/` re-synced (CI byte-exact guard).

### 15. Extend virtual-peripheral event queue to EXTI / ADC / TIM (`src/system.rs`, `src/peripherals/{exti,adc,tim}.rs`, `src/lib.rs`, `pkg/stm32f1.js`) [committed]
- `VmEvent` gained `ExtiEdge{line}`, `AdcDone{adc,chan}`, `TimUpdate{tim}` (flat discriminants 7/8/9). Pushed in `exti.rs::gpio_pin_changed` (hardware edge), `adc.rs::advance_regular/advance_injected` (EOC/JEOC), `tim.rs::tick_once` + `generate_update` (UIF). Encoded in `src/lib.rs::drain_events`.
- `STM32F1` gained top-level callbacks `onExtiEdge(line)`, `onAdcDone(adc,chan)`, `onTimUpdate(tim)` (dispatched from `_drain_events`).
- Tests: `tests/test_extra_events.mjs` (TIM + ADC via periph_test; periph_test headless does NOT self-trigger EXTI edges, so EXTI is logged, not asserted there), `tests/test_exti_events.mjs` (deterministic: configure EXTI0/1 via the bus, drive PA0/PA1 high via `gpioSetInput` -> ExtiEdge{0,1}). Both wired into CI. `docs/STM32F1_API.md` updated.

### 16. More virtual-peripheral events: DAC/CRC/RTC/Watchdog/CAN (`src/system.rs`, `src/peripherals/{dac,crc,rtc,iwdg,wwdg,can}.rs`, `src/lib.rs`, `pkg/stm32f1.js`) [committed]
- `VmEvent` gained `DacWrite{chan,value}` (disc 10, dac.rs DHR write), `CrcResult{value}` (disc 11, crc.rs DR read), `RtcAlarm{alarm}` (disc 12, rtc.rs tick when alarm crossed), `WdogReset{which}` (disc 13, iwdg.rs/wwdg.rs reset request — which: 1=IWDG,2=WWDG), `CanTx{can,id,len,data[8]}` (disc 14, can.rs TX mailbox submit), `CanRx{can,id,len,data[8]}` (disc 15, can.rs inject_message). Encoded in `src/lib.rs::drain_events` (id = 11-bit STDID or 29-bit EXTID, len = DLC, data = 8 bytes).
- `STM32F1` gained top-level callbacks `onDacWrite/onCrcResult/onRtcAlarm/onWdogReset/onCanTx/onCanRx` (dispatched from `_drain_events`).
- Tests: `tests/test_more_events.mjs` (DAC/CRC/RTC fire naturally in periph_test; CAN RX/TX driven deterministically by configuring a pass-all filter + inject / submitting a mailbox). Wired into CI. `docs/STM32F1_API.md` updated.
- **Note**: F103 has no onboard comparator, and TIM input-capture wasn't modeled — now it is (see item 17).

### 17. Implement TIM input capture + FSMC transaction events (`src/peripherals/{tim,fsmc}.rs`, `src/system.rs`, `src/lib.rs`, `pkg/stm32f1.js`) [committed]
- **Real TIM input capture** (`tim.rs`): added `last_cap`/`cap_count`/`cap_inited` per channel + `sample_input_capture()` (called once per batch in `tick()`). When a channel's `CCMR CCxS != 0` (input mode) and an edge matching `CCxP`/`CCXNP` polarity occurs on its source pin (default `tim_chan_pin` mapping, no AFIO remap; `CCxS=10` -> partner pin), CNT is latched into `CCRx`, `CCxIF` set, IRQ pending if `CCxIE`, and a `TimCapture{tim,ch,value}` event (disc 16) is emitted. The output-compare block in `tick_once` now skips input-capture channels. Also hardened a latent `ARR=0xFFFF_FFFF` divide-by-zero in `advance()` (`self.arr + 1` wrap) — guard `arr != u32::MAX`.
- **FSMC transaction events** (`fsmc.rs`): `FsmcAccess{bank,offset,write,size,value}` (disc 17) pushed on every NOR/NAND/PC-Card data read/write in `read_sized`/`write_sized` (regardless of whether a backing ext_device image is attached).
- `STM32F1` gained `onTimCapture(tim,ch,value)` + `onFsmcAccess(bank,offset,write,size,value)`, dispatched from `_drain_events` (types 16/17).
- Tests: `tests/test_tim_capture.mjs` (deterministic: TIM2 CH1 input-capture on PA0, drive rising edge -> TimCapture{2,0}), `tests/test_fsmc_events.mjs` (deterministic: enable BANK1, read+write 0x60000000 -> FsmcAccess read+write). Both wired into CI. `docs/STM32F1_API.md` updated.
- **Verified**: 236/236 unit (incl. new TIM/FSMC paths), tim_capture 4/4, fsmc 8/8.

### 18. Wokwi virtual peripheral end-to-end + TIM AFIO remap (`src/peripherals/{tim,afio}.rs`, `pkg/stm32f1.js`, tests) [committed]
- **AFIO remap in `tim_chan_pin`**: `sample_input_capture` now reads the AFIO MAPR
  remap code via `sys.p.afio_remap_status(name)` and `tim_chan_pin(name, ch, remap)`
  returns the remapped pins for TIM2/TIM3/TIM4 (TIM2_REMAP bits[9:8], TIM3_REMAP
  bits[11:10], TIM4_REMAP bit12). Also fixed the buggy AFIO MAPR TIM bit shifts in
  `afio.rs` (`remap_status` had TIM1>>4/TIM2>>24/TIM3>>9/TIM4>>10 — now correct
  >>6/>>8/>>10/>>12; CAN>>22). `periph_remap()` is unused, so risk-free.
- **End-to-end Wokwi virtual peripheral**: `tests/test_fsmc_display.mjs` drives an
  FSMC-backed LCD model entirely through `onFsmcAccess` — the MCU writes LCD
  command/data over FSMC BANK1 (RS decoded from the address line) and a JS
  `FsmcLcd` class accumulates its command register + framebuffer. This is exactly
  the path real firmware takes (MC11 register writes = what compiled C emits).
- Tests: `tests/test_tim_remap.mjs` (TIM2_REMAP=01, CH2 -> PB3: a PA1 rising edge
  must NOT capture, a PB3 rising edge MUST capture -> exactly one TimCapture{2,1}),
  `tests/test_fsmc_display.mjs` (virtual LCD receives reset cmd + 3 pixels in order).
  Both wired into CI. `docs/STM32F1_API.md` updated (remap note).

### 19. WebSocket bridge: headless Node emulator + browser viewer (`pkg/ws-server.mjs`, `site/ws-viewer.html`) [committed]
- **`pkg/ws-server.mjs`**: Node HTTP static file server (`site/`) + WebSocket at
  `/ws`. Loads firmware via `createEmulator()`, runs `emu.step(20000)` at ~60fps
  (`setInterval`), drains `drainEvents()` + `takePinEvents()` (Array.from for
  correct JSON serialization), broadcasts as JSON to all connected clients. Receives
  `uart_rx`/`gpio_set`/`can_inject` commands from clients. Flags: `--port`, `--max`.
  Idle when no clients connected. Rebuild requires `npm install ws` (runtime dep).
- **`site/ws-viewer.html`**: Standalone browser page that auto-connects to the WS
  server. Decodes all 17 event types (SPI/I2C/USART/EXTI/ADC/TIM/DAC/CRC/RTC/
  WDG/CAN/FSMC). Renders: UART terminal, GPIO pin grid (click to toggle input),
  event log, FPS/instruction counter. Reconnects on disconnect.
- **Usage**: `node pkg/ws-server.mjs <firmware.elf> [--port=8080]`, then open
  `http://localhost:8080/ws-viewer.html` in a browser.
- **Committed** as `224a65a`. `pkg/.gitignore` updated with `!ws-server.mjs`.

### 20. SDIO host + SDHC card image + DMA2 completion fix (`src/peripherals/sdio.rs`, `src/ext_devices/sd_card.rs`, `src/peripherals/dma.rs`, `src/system.rs`, `src/lib.rs`, `pkg/emulator.js`, `pkg/cli.mjs`, `tests/test_all.mjs`) [committed]
- **SDIO** @ 0x40018000, IRQ 49, SDHC-only (CCS=1): CMD0/2/3/6/7/8/9/12/13/16/17/18/24/25/55 + ACMD41 (busy-first power-up), 32-word FIFO window, DATAEND/DBCKEND/CMDREND/CMDSENT/CTIMEOUT + MASK-gated IRQ, DCOUNT/FIFOCNT. Commands complete synchronously on CPSMEN (all firmware timeouts generous); unknown CMDs get lenient R1; no card → CMDSENT/CTIMEOUT. `SdCard` ext device (`add_sd_card('SDIO', data)`): CID/CSD/OCR/RCA derived, CSD capacity from image size; `ext_devices.sd_card` + cli `sd_card:` config (file/size).
- **DMA2 completion was broken**: completion streams/IRQ tables were sized 8 with local channel indices, so DMA1 CH4 and DMA2 CH4 both claimed stream 3 and DMA1's tick drained DMA2's bits — DMA2 ISR/CNDTR never completed (nothing had driven DMA2 concurrently before, so it never showed). Streams are now GLOBAL (DMA1 ch0-6 → 0-6, DMA2 ch0-4 → 7-11): `do_xfer` maps by name, ticks take only their own bits (`dma_take_completions_masked`), tables siz 12, `dma_set_completed_many` loops 0..12. JS pump untouched (passes plan bits through). SDIO issues one `dma_request(11)` (DMA2 CH4) per transfer; TX finalizes when DLEN bytes land in the FIFO path, RX drains from the image — polled and DMA share one implementation.
- **Verified**: 277/277 unit (41 SDIO: init sequence, block R/W + read-back, IRQ49, DMA2 pump absorb of real image bytes + TCIF4/CNDTR clear, no-card timeouts, F103-SVD registration); full gate green (canary 39/39, cli + emulator.js 200M 39/39, all event/format/esm/ws/browser tests).

### 21. Coverage-audit leftovers: WWDG EWI (proven), PVD, RTC fix, tamper, RCC clocks, USB FS device (`src/peripherals/{wwdg,pwr,rtc,rcc,bkp,usb}.rs`, `src/peripherals/{mod,exti,gpio}.rs`, `src/system.rs`, `src/lib.rs`, `pkg/{emulator,stm32f1}.js`, `tests/test_all.mjs`, `tests/test_stm32f1_api.mjs`, `tests/arduino_periph_test/`, `docs/COVERAGE.md`) [committed]
- **Audit corrections first**: WWDG EWI was already implemented (added the missing test — 5 asserts green, no src change); TIM9–11 `timer_base()` entries were already present (audit claim wrong, compiler caught the duplicate). Real gaps fixed below.
- **PVD** (`pwr.rs`, EXTI line 16): fixed 3.3 V supply model — PVDO follows PVDE, edges fan out via new `Peripherals::exti_line_edge()` (same IMR/RTSR/FTSR gating as GPIO, no port check; shared `fire_line()` core) → PVD_IRQn (1). PVDO bit made read-only. Tested rising + falling + read-only (6 asserts).
- **RTC second/overflow + flag overhaul** (`rtc.rs`): fixed a mirrored CRH bit-order mistake (code AND test firmware both had ALRIE/SECIE swapped — RM0008: SECIE=0/ALRIE=1/OWIE=2, verified against the manual): alarm gate → bit 1, new per-second SECF + IRQ (SECIE), wrap OWF + IRQ (OWIE), CRL SECF/ALRF/OWF with write-0-clears, CRH mask widened to 0x07 (OWIE was unwritable). Firmware fixed (`CRH = 2`, handler clears ALRF) and rebuilt (arduino-cli; eeprom images restored; `canRxArmed` stable at 0x200000bc); canary still 39/39.
- **Tamper** (`bkp.rs`, `gpio.rs`): BKP remapped to the RM0008 layout (DR1-10 @ 0x04-0x28, RTCCR @ 0x2C, CR @ 0x30, CSR @ 0x34 — was DR[20] @ 0x04-0x50 with RTCCR/CR/CSR at 0x00/0x58/0x5C; caught the stale RTCCR unit test too). TPE/TPAL edge detection on PC13 input edges (new `bkp_tamper()` fan-out from `set_input_pin`; output-driven LED unaffected), event clears all DRs, TEF+TIF, TAMPER IRQ (2), W1C via CTEF/CTI. Tested active-high/low, silent-when-off (9 asserts).
- **RCC clocks** (`rcc.rs`): decoded SYSCLK/HCLK/PCLK1/PCLK2 from CFGR (SW/PLLSRC/PLLMUL/HPRE/PPRE, HSE assumed 8 MHz) via new `rcc_clocks()` trait method + `rcc_sysclk_hz()` export (72 MHz PLL×9 verified). Deliberately no timing rescale: 1 instr = 1 cycle keeps TIM/ADC exact, and rescaling SysTick/USART would 9× every delay loop and break all firmware instruction budgets.
- **USB FS device** (`usb.rs`, ~420 lines): EP0-7R with hardware toggle semantics, CNTR masks, ISTR (W0C flags; CTR/DIR/EP_ID derived from endpoint state), DADDR, BTABLE, 512 B PMA with byte-exact access (PMA window exempted from bus word-lane logic), RESET on FRES release + IRQ20, SETUP/OUT injection (NAK unless VALID, DTOG sequencing) via `usb_inject_setup/out`, IN completion drained as `UsbIn` (discriminant 18) + `onUsbIn` in `STM32F1` + `usbInjectSetup/Out` on the emulator. No SOF engine/suspend/wakeup/double-buffer. USB window sized 0x800 in both maps (registers + PMA). Tested end-to-end at register level incl. a full SETUP→descriptor-IN→bulk-OUT flow (~45 asserts) + `onUsbIn` dispatch.
- **Verified**: 354/354 unit; full gate green (canary 39/39, cli + emulator.js 200M 39/39, all 16 event/format/esm tests, ws_bridge, browser speed).



## Next Phase — Long-term Optimizations
1. **Single WASM module — "Path A" (EXPERIMENT status, see docs/NEXT_PHASE.md §4)**: compile Rust peripherals + Unicorn C into one `emcc` output (`wasm32-unknown-emscripten` + `staticlib` + raw `#[no_mangle]` exports — wasm-bindgen does NOT support the emscripten target, so the ~70 exports need a shim; fetch unicorn C source, `third_party/unicorn/` is gitignored). Dual-wasm stays the default until the acceptance gate passes (224/224, 39/39 both paths, IPS within ~2× of 22M). Gain is architectural, not speed.
2. ~~**Replace mem hooks with shared linear memory**~~ — **retired (moot)**: `uc_mem_map_ptr(mem, periph_range)` would remove the JS crossing, but peripheral access was measured at 0.001 accesses/instruction (~0.1% of runtime) — no measurable win available
3. **DMA + interrupts fully in Rust** (no JS round-trip; `uc_intr` or stop+re-exec) — mostly landed (plan-based pump, intr dispatch §9); remaining RAM→RAM moves need Unicorn memory access
4. **Pure-Rust Cortex-M core — "Path B" (deferred, do NOT start)**: rewrite Unicorn's core (mdl / cargo-cortex-m, or unicorn's in-progress Rust core upstream). Highest risk (TCG perf parity unproven, decoder drift); only when a feature needs CPU-core changes, or upstream ships it free

## Files Most Relevant
- `src/peripherals/i2c.rs` — I2C state machine
- `src/lib.rs` — WASM API; new exports
- `pkg/cli.mjs` — DR Mode patch, workarounds, loop, SVC hook, fault gate
- `src/peripherals/usart.rs` — TXE byte-time pacing, `rx_pending()`
- `src/ext_devices/spi_flash.rs`, `src/peripherals/spi.rs`, `src/ext_devices/touchscreen.rs` — touchscreen SPI reads (deferred_reply)
- `src/peripherals/gpio.rs` — electrical model (`pin_level()`), slew (`pending_transitions`), `read_pin_effective()`; `set_input_pin()` fires EXTI edges (page-driven button widgets → attachInterrupt works)
- `src/peripherals/scb.rs` — deep sleep, SHPR routing, `raise_fault()`
- `src/peripherals/fsmc.rs`, `src/ext_devices/fsmc_nor.rs` — FSMC banks + backing
- `src/peripherals/adc.rs` — real conversion state machine
- `tests/arduino_periph_test/` — the 24-peripheral firmware (39 checks incl. SVC/PendSV) + config
- `tests/arduino_hw_showcase/` — 7-device live demo firmware (OLED 0x3C, SPI LCD CS PA8, 7-seg 74HC595 via SPI1+PA4, RGB TIM2 PWM PA0-2, buzzer PB14, button PB13 EXTI13) — build `tests/arduino_hw_showcase/build/arduino_hw_showcase.ino.elf`; ship `site/arduino_hw_showcase.elf` (force-add)
- `site/index.html` — `showcase` preset (`newString`/`oldString`); 7-seg is a JS-side shift-register decode: `onPeriphWrite` watches SPI1 DR (0x4001300C) while PA4 CS is low (GPIOA ODR/BSRR/BRR tracking), latch 4 bytes → `segDigits`; OLED/LCD render from `emu.i2cOledFb('I2C1',0x3C)` / `emu.lcdFb('SPI1')`; button widget calls `emu.gpioSetInput(1,13,true/false)`
- `pkg/ws-server.mjs` — WebSocket bridge server (HTTP static + WS streaming + emulation loop)
- `site/ws-viewer.html` — browser WebSocket viewer (event decoder, UART terminal, GPIO grid)