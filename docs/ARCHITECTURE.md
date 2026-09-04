# Architecture — How the Emulator Works

Full-system emulation of an **STM32F103C8 (Bluepill)** running real compiled firmware
(STM32duino/Arduino core, STM32Cube HAL, libopencm3 — anything that targets the real chip).
Both the CPU (native Rust ARMv7-M interpreter, `src/cpu/`) and every peripheral are
emulated in **Rust** (compiled to one WASM module via `wasm-pack`). A small JavaScript
layer drives batches and pumps I/O. (Unicorn TCG was the CPU before 2026-09; see
`docs/PATH_B.md` for the cutover.)

```
┌───────────────────────────── JS (pkg/emulator.js / pkg/cli.mjs) ────────────────────────┐
│                                                                                          │
│  EXACT instruction counting — rustcpu_run(batch) returns executed instructions          │
│  (thread + handlers). No mem hooks: peripheral writes are recorded in-Rust             │
│  for onPeriphWrite watchers; DMA pumps against Rust RAM with zero JS crossings.         │
│                                                                                          │
│  Loop (1 iteration = 1 batch, adaptive 20K/50K):                                         │
│    1. pump stdin → uart_rx_byte()                                                        │
│    2. rustcpu_dma_pump()        ← plan build + exec against Rust RAM                   │
│    3. rustcpu_run(curBatch)     ← run one batch in the native interpreter              │
│       curBatch = 20K when IRQ/DMA pending, 50K when idle                                │
│       batch_size opts overrides adaptive                                                 │
│    4. step_batch(count)         ← Rust ticks ALL peripherals once (instruction-delta)  │
│       (status==1 → watchdog reset requested → stop)                                     │
│    5. rustcpu_dma_pump()                                                                 │
│    6. rustcpu_dispatch()        ← up to 64 IRQs per batch, NVIC-priority ordered        │
│    7. watchdog reset check                                                               │
└──────────────────────────────────────────────────────────────────────────────────────────┘

Browser dual-mode (`site/index.html:16`, `site/worker.js:1`, `site/_headers:1`):
  ┌─ Worker path (preferred, off-main-thread) ──────────────────────────────────┐
  │  main thread → new Worker('./worker.js', {type:'module'}) (site/index.html:449)      │
  │  worker.js: createEmulator + step loop @ ~60fps / 80ms budget (site/worker.js:130)   │
  │  worker posts {frame, pins, uartOut, oledFb/lcdFb, rgbDuty, buzz} → main renders     │
  │  OffscreenCanvas: main transfers canvas → worker renders directly (site/worker.js:117) │
  │  SAB fast path when crossOriginIsolated (site/worker.js:123, site/_headers:1):        │
  │    SharedArrayBuffer 32B [instCount, PC, SP, runSteps] + Atomics.store/notify        │
  │    queueMicrotask loop (~0ms) vs setTimeout 4ms clamp when not isolated              │
  │  UI_THROTTLE 10: step every rAF, render visuals every 10th frame (site/index.html:441)│
  └────────────────────────────────────────────────────────────────────────────────────────┘
  ┌─ Main-thread fallback ──────────────────────────────────────────────────────┐
  │  requestAnimationFrame(runLoop) (site/index.html:1034) when Worker unavailable       │
  │  same adaptive 20K/50K batch + UI_THROTTLE 10 DOM decoupling                         │
  └────────────────────────────────────────────────────────────────────────────────────────┘
  SAB dual-mode (`site/coi-serviceworker.js:1`, `site/index.html:300` badge):
    COOP/COEP headers present → crossOriginIsolated=true → SAB ON (zero-copy, faster)
    GitHub Pages (no headers) → coi-serviceworker polyfill → SAB OFF → still works
```

## The two modules

| Module | What it is | Notes |
|---|---|---|
| `src/cpu/` | ARM Cortex-M3 CPU (native Rust Thumb-2 interpreter) | Probed in `cargo test --lib cpu::` (firmware gallery, inline-IRQ, WFI, PSP); exact instruction accounting. |
| `pkg/stm32_bluepill_wasm_bg.wasm` | CPU + all peripherals in Rust | wasm-bindgen exports (`rustcpu_*` backend + model API); the JS glue is `pkg/stm32_bluepill_wasm.js`. Source lives in `src/` (`src/cpu/`, `src/native.rs`, `src/bus.rs`, `src/peripherals/*`, `src/ext_devices/*`). |

One module holds the whole machine: the CPU calls into the peripheral model
directly (no JS crossings per instruction or per access).

## The peripheral bus (rp2040js-style)

`src/bus.rs` is a runtime registry in the style of Wokwi's `rp2040js`/`avrjs`
`bus.ts`: peripherals register an address range `[start, end)` and accesses are
routed with a binary search. Board assembly picks the peripheral set:

| Layer | rp2040js/avrjs equivalent | Ours |
|---|---|---|
| CPU core | `src/cpu/` native Rust Thumb-2 interpreter | In-module: direct model calls, exact counting |
| Bus | `bus.ts` `addPeripheral(addr, peripheral)` | `src/bus.rs` `Bus::register(start, end, tick, p)` |
| Peripheral impl | one class per file in `src/peripherals/` | one `impl Peripheral` per file in `src/peripherals/*.rs` |
| Chip assembly | `rp2040.ts` / `mcu.ts` | `Peripherals::new_wasm()` (builtin STM32F103C8 table) / `from_svd()` (any F1-family SVD) |
| Custom chips | TS classes on the bus | `register_js_peripheral(base, size, read, write)` — JS callbacks called with absolute address + width; last registration wins (can shadow built-ins) |

Registration is live: a new peripheral (Rust or JS) can be added to a running
emulator; `init()`/`init_svd()` rebuild the whole bus from scratch. SVDs that omit
the ARM core peripherals (STM32F105xx has no SCB/SysTick) get them auto-registered
at their fixed 0xE000Exxx addresses. F1-family SVDs work out of the box (F105 adds
CAN2@0x40006800); F4/G0-class chips need new peripheral modules (MODER-style GPIO,
different RCC layout) but the bus itself is chip-agnostic.

## Memory model

- **Flash + SRAM** live in `FlatMemory` (`src/cpu/mem.rs`): plain Rust byte arrays
  loaded from the firmware image. Flash is execute/read-only for the guest (stores
  ignored — real flash needs the erase/program sequence); `rustcpu_load()` bypasses
  the protection at load time, like a debugger's memory write.
- **Peripheral addresses route into the model in-Rust**: `FlatMemory` forwards any
  `0x4000_0000–0xB000_0000` / `0xE000_0000–0xE100_0000` access to `Peripherals::read`
  / `write` (binary-search bus) with zero JS crossings. Unmapped reads return 0 and
  record the address in `bad` for diagnostics; writes are dropped.

## Instruction counting (exact)

`rustcpu_run(slice)` returns the instructions actually executed (thread +
handlers, handlers single-stepped). Accounting is exact — no hooks, no credit
slop. A CPU decode gap stops the run and surfaces via `rustcpu_fault()` as
`[pc, op1, op2, len]`; the driver logs it, raises UNDEFINSTR when symbols are
available (realistic fault escalation), and steps past. (The Unicorn era
credited full batches and skipped a known `bl` artifact — both gone.)

## Timing model (how fast is "fast")

Peripheral time is derived from **instruction counts**, not wall-clock:

- `INSTRUCTION_COUNT` is a global in Rust, advanced in `step_batch(count)`.
- Every peripheral keeps its own "advance by N instructions" logic: SysTick accrues
  1 ms per 8,000 instructions (8 MHz emulated clock), timers advance CNT by
  `count / prescaler`, USART paces bytes at the configured baud rate
  (`byte_time = 8_000_000 / baud` instructions), etc.
- `step_batch` ticks **once per batch**, not per instruction: advancing the shared counter
  by `count` and calling `sys.tick()` once is equivalent to 20,000 per-instruction ticks
  (each peripheral processes *all* accumulated deltas) but is ~100K× cheaper. This single
  change was a **3.8× speedup** (21.2s → 5.6s for 100M instructions).

## Interrupts

- Rust implements the **NVIC** (priority dispatch, pending/active sets, PRIMASK/BASEPRI
  gating) and **SysTick** with a 1 ms "debt" system: a batch can accrue several 1 ms
  IRQs, and `nvic_systick_take()` drains them one per handler run (wired to `irq === -1`
  — SysTick is delivered as offset `-1` into `vector_table + 4*(16+irq)`; the accounting
  subtracts the first tick, which the pending IRQ itself delivers, so steady state is
  exactly one SysTick per period).
- After each batch the driver calls `rustcpu_dispatch()`: `intr_next()` loop (up to
  **64 IRQs per batch**), each taken via `Cpu::take_exception` (hardware-shaped
  stacking incl. **xPSR**) and run single-stepped to `bx lr` / `exception_return`.
  The NVIC's `last_popped` fairness makes a hot self-re-pending IRQ (e.g. USART TXE
  while draining a ring buffer) alternate with other pending IRQs instead of
  starving them. (Single-stepping is load-bearing: a chunked handler run keeps
  executing past `bx lr` into thread code — once observed overshooting an SPI2
  print into `testSVC`'s `svc #2`.)
- Interrupt latency is bounded by the batch size: **20K instructions ≈ 1.1 ms** at real
  speed (was 5.4 ms at 100K).

## Exceptions (SVC, PendSV, faults)

Beyond IRQs the emulator models the system exceptions that real firmware exercises:

- **SVC**: the core faults `svc` to the driver, which steps past it and calls
  `take_exception(-5)` — full hardware-shaped stacking on the real stack, `LR` =
  EXC_RETURN (0xFFFFFFF9, or 0xFFFFFFFD when `CONTROL.SPSEL` is set), PC = SVCall
  vector (exception 11). The handler runs inline and `bx lr` returns through
  `exception_return` (no mirror, no fault trick — the old Unicorn path needed
  both). With lazy dispatch the driver does this at the batch edge; with
  `deliver_irqs` the core takes it mid-slice itself.
- **PendSV**: pended normally via `ICSR.PENDSVSET` (SCB register write → NVIC pending),
  dispatched by the same lazy path as IRQs.
- **Faults**: unknown encodings stop the run and surface via `rustcpu_fault()`; the
  driver raises them with `raise_fault(kind, addr)` in Rust:
  CFSR (IBUSERR/PRECISERR/UNDEFINSTR), BFAR + BFARVALID, and HFSR FORCED are recorded,
  and the fault exception is pended — BusFault (or UsageFault for undefined instructions)
  if the corresponding SHCSR enable bit is set, otherwise **escalated to HardFault**.
  Priority of the system handlers is programmable via SCB SHPR1–3, which the SCB
  peripheral routes into the NVIC's `sys_handler_priority` table.
- Fault handlers run through `processInterrupts` and re-execution of the faulting
  instruction is left to the handler (as on real hardware).

## DMA

DMA never crosses JS (one `rustcpu_dma_pump()` call per pump):
1. Rust queues transfers (`queue_dma_transfer`, deduped per channel while queued).
2. The pump pops the queue, absorbs/pushes peripheral bytes internally, and executes
   the op plan against Rust RAM directly (memcpy / store-absorbed / read-RAM-then-push
   / done-bits); completion is signaled last so TC IRQs fire after the data lands.

## Sleep modes (STOP/STANDBY)

`SCR.SLEEPDEEP` (SCB register write) puts the CPU in a deep-sleep mode. `system.tick()`
then calls `tick_frozen()` on every peripheral except the LSI/LSE-clocked **RTC** and
**IWDG**, which keep running, and stops SysTick accrual. Instruction-delta peripherals
(e.g. timers) override `tick_frozen()` to advance their delta base *without* processing
state, so they don't catch up when the CPU wakes. Wake-up is immediate on the next
interrupt (e.g. UART RX), which pends from JS at the next batch boundary.

## GPIO electrical model

IDR readback is a per-pin **wire-level** model: input pull-up/down (CNF=01, ODR bit
selects direction), floating input (external driver or 0), push-pull output readback,
open-drain (low driven, high released → external pull or 0), and external drivers
(JS-registered read callbacks) win over driven state. Output transitions can be slowed
with `gpio_set_slew(n)`: IDR shows the previous level until the transition settles.
ODR/BSRR/BRR write the full register (input pins use ODR for pull selection), while
output-state side effects (device callbacks, EXTI) only fire for pins in output mode.

## External devices (I2C/SPI bus devices)

Devices such as SPI NOR flash, I2C EEPROMs, OLED panels, an LCD, and a resistive
touchscreen live in `src/ext_devices/` as Rust structs that plug into the SPI/I2C
peripheral state machines. They can be file-backed (a `Buffer.alloc` in JS, e.g. a
64K EEPROM image) so state survives across runs. GPIO pin state is consulted for
chip-select and touch-detect lines (`read_pin_effective()`: read callback first,
else driven output value). **FSMC external memory** is a memory-mapped ext device:
each enabled bank (`BCR.MBKEN`, writes also need `BCR.WREN`) reads/writes a JS-backed
byte image (`add_fsmc_bank('FSMC.BANK1', data)`) at its 0x6000_0000+ window, with
byte/16/32-bit accesses. NAND/PC-Card banks are always enabled.

## Worker + SAB + OffscreenCanvas + adaptive batch & pooling

- **Worker** (`site/worker.js:1`, `site/index.html:449`): module Worker (`{type:'module'}`) runs `createEmulator` + `emu.step()` loop at ~60fps (80ms budget, `site/worker.js:130`). Main thread keeps DOM/canvas; worker posts `{frame, pins, uartOut, oledFb/lcdFb, rgbDuty, buzz}` per frame (`site/worker.js:199`). Fallback is main-thread `requestAnimationFrame(runLoop)` (`site/index.html:1034`) when Worker unavailable. Both paths share the same `emulator.js` batch logic.
- **SAB dual-mode** (`site/_headers:1`, `site/coi-serviceworker.js:1`, `site/index.html:16`, `site/index.html:300`, `site/worker.js:123`): `Cross-Origin-Opener-Policy: same-origin` + `Cross-Origin-Embedder-Policy: require-corp` → `crossOriginIsolated=true` → `SharedArrayBuffer` 32B `[instCount, PC, SP, runSteps]` written with `Atomics.store`/`Atomics.notify` (`site/worker.js:163`). Loop uses `queueMicrotask(loop)` when isolated (~0ms, `site/worker.js:220`) vs `setTimeout 4ms` clamp otherwise. GitHub Pages has no headers → `coi-serviceworker.js` polyfill registers a ServiceWorker that injects COOP/COEP and reloads; SAB badge (`site/index.html:300`) shows `SAB ON` (green) or `SAB OFF` (grey) — both modes work, ON is +6.6% faster.
- **OffscreenCanvas** (`site/worker.js:117`, `site/index.html:977`): main transfers `oledCanvas`/`lcdCanvas` via `transferControlToOffscreen()` to worker; worker renders via `OffscreenCanvas.getContext('2d')` directly (`site/worker.js:168`, `site/worker.js:185`). If transfer fails (Safari/no support) worker sends framebuffers to main for rendering — no capability loss.
- **Adaptive batch**: `curBatch = (anyPending || dmaBusy) ? 20K : 50K` (`batch_size` opt overrides adaptive). 20K keeps IRQ latency ≈1.1 ms when active; 50K doubles idle throughput. Measured free vs fixed 20K.
- **UI_THROTTLE 10** (`site/index.html:441`, `site/index.html:1082`): DOM decoupling — `step()` runs every `rAF` frame (~60fps) for full throughput; visuals (`renderBoard`, `renderGpio`, `renderShowcase`, `updateRegs`) run only every 10th frame (~6fps). Stepping is never starved by rendering; this lifted headed browser from 4.5 → 8.6 MIPS before Worker, and with Worker keeps main thread ~6fps paint.

## Known workarounds (temporary)

1. **hi2c->Mode patch**: when I2C1 DR is written with the read bit, the model flags
   it (`WasmSystem.i2c_dr_hook`) and the driver patches `*(hi2c)+0x3D` in RAM to
   `0x22` (MASTER_RX) pre-dispatch — the HAL I2C ISR requires it.
2. **64-IRQ budget** (`src/interrupts.rs`) + NVIC `last_popped` fairness: prevents a
   hot re-pending IRQ from starving the rest.
(Retired with Unicorn: the `mrs`→`mov` patch, the `i2c_init` NVIC patch, the `bl`
artifact skip, hook poll-shrinking, and the SVC mirror.)

## Performance

| Metric | Value |
|---|---|
| Headless (Node CLI, periph39) | **~23 MIPS** — 50M in ~2.1s; pure compute 26.5 MIPS (`pkg/cli.mjs:632`) |
| Headless Rust CPU (`--cpu=rust`, periph39) | **72–75 MIPS** — 200M in ~2.7s, 39/39 (`docs/PATH_B.md`) |
| Emulator.js path (Node, periph39) | **~24 MIPS** — 200M in 8.2s (`tests/test_emulator_js.mjs`) |
| Emulator.js Rust CPU (`cpu:'rust'`) | **~70 MIPS** — 200M in ~2.8s, 39/39 (`tests/test_rustcpu.mjs`) |
| Browser direct run (headless Chromium) | **~21.5 MIPS** — 200M in 9.3s, SAB OFF (`tests/test_browser.mjs`) — parity with Node since the CAN-autopilot fix below |
| Browser interactive loop | **8.6 MIPS** headed (frame-budgeted rAF/worker loop, `site/worker.js:130`); headless rAF throttled |
| SAB ON vs OFF (browser loop) | 9.22 MIPS (ON) vs 8.65 MIPS (OFF) = **+6.6%** (`site/_headers:1`, `site/worker.js:163`) |
| Batch | **adaptive 20K/50K** — 20K when IRQ/DMA pending (≈1.1 ms latency), 50K idle (`pkg/emulator.js:698`, `pkg/cli.mjs:632`); `batch_size` overrides (`pkg/emulator.js:217`) |
| Memory | stable ~150 MB RSS, no growth with instruction count |
| Per-batch crossings | one `rustcpu_run` + tick + pump + dispatch; interpreter ~99% of runtime (profiled) |

Historical optimizations, in order: per-instruction tick → once-per-batch `step_batch`
(3.8×), plain-number counters (1.19×), **hookless batch crediting** (1.16×), 20K batches
(latency, free), **closed-form timer advance** (1.15×; step_batch 1409ms → 11ms — the
only remaining O(ticks) loop was `tim.rs advance()`, rewritten to jump directly
to update/compare-match event ticks with bit-identical event sets), **adaptive 20K/50K**
(idle throughput, free), **REG_POOL pooling** (pkg/emulator.js:228 — 384 allocs/40M → pooled),
**Worker + OffscreenCanvas + SAB** (site/worker.js:1 — browser loop 8-9 MIPS, +6.6% SAB),
**UI_THROTTLE 10** (site/index.html:441 — DOM 6fps, step 60fps; never starves emulation),
**CAN-autopilot symbol resolution** (all drivers parseElf-resolve `canRxArmed` instead of
a hardcoded address that went stale: emulator.js 200M 12.45s → 8.18s, browser 9.2 → 21.8 MIPS),
**native Rust CPU** (`docs/PATH_B.md` — replaces Unicorn TCG: periph39 200M ~9.5s → ~2.7s).
