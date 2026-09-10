# Path B — Native Rust CPU backend (ACTIVE, `rust_cpu` branch)

> Status: **default backend; Unicorn deleted**. The `--cpu` / `cpu` / `?cpu`
> options are gone (single backend). This doc records the cutover.

## Goal

Run guest firmware on the vendored pure-Rust ARMv7-M interpreter
(`src/cpu/`: `mem.rs`, `mod.rs`, `regs.rs`, `thumb.rs`, from the F407 emulator
tree, M4 core) instead of Unicorn TCG, against the **same** peripheral model,
DMA plan pump, NVIC dispatch and bus. M3 correctness: `dsp: false` faults the
DSP extension; CPUID still comes from the model SCB (`0x410FC241`).

## Selection (historical — options removed with Unicorn)

During bring-up both backends coexisted (`--cpu=rust`, `createEmulator({cpu})`,
worker `msg.cpu`, `?cpu=rust`) for benchmarking; Unicorn stayed the default
until the cutover, then the flags were deleted.

## Architecture (what's shared, what isn't)

Shared (identical on both backends): `WasmSystem` model, peripheral bus, DMA
plan (`dma_pump_all`), `intr_next` budget dispatch, SVC mirror, DWT CYCCNT,
`process_batch` tick, UART/GPIO/events/fb exports.

Rust-backend only (`src/native.rs`, `rustcpu_*` exports): process-global
`Cpu` + `FlatMemory` (60 — Bluepill sizes from the driver), SVC dispatched
inline onto the real stack, lazy IRQ dispatch via `intr_next` +
`take_exception` with handlers **single-stepped** (a chunked handler run keeps
executing past `bx lr` into thread code — seen overshooting an SPI2 print into
`testSVC`'s `svc #2`), exact instruction accounting (executed count returned,
handlers included; Unicorn credits full batches instead — within ~1%).

Two Unicorn-isms are replicated, not inherited:
- **hi2c Mode RAM patch** (`*(0x200002d8)+0x3D = 0x22` on I2C1 DR write with
  R-bit): the model sets a flag (`WasmSystem.i2c_dr_hook`), the driver patches
  Rust RAM pre-dispatch. The Unicorn hook is untouched.
- **`onPeriphWrite` watchers**: no mem hooks exist on this path, so
  `Peripherals::write` records `(addr, size, value)` into a gated log
  (`rustcpu_write_tap` on iff a watcher subscribes; DMA pushes funnel through
  the same write path). Drained per batch after pin events (CS-before-DR order
  preserved as on the hook path).

Deleted with Unicorn: `patchMrsMsp`, the `i2c_init` NVIC patch, the `bl`
artifact skip, hook poll-shrinking, the SVC mirror, `regsRead`/`regsWrite`
pooling, `unicorn_arm.cjs/.js` (~1.6MB), `test_unicorn.cjs`,
`test_svd_run.cjs`. The core implements MRS and runs the real `bl`s (proven
by native I2C passing unpatched); adaptive batch keeps 20K/50K on
pending/DMA/RX state.

## Bugs found bringing it up (all fixed)

- **Register-shift-by-0 is no-shift** (`thumb.rs`): `shift_op`'s `amt==0` arm is
  the *immediate* encoding (LSR#0=#32, ROR#0=RRX). Register forms with Rs=0
  returned 0 — broke `HAL_GPIO_Init` (`lsrs.w r5,r2,r6`, IMR stayed 0) and
  zeroed `SystemCoreClock` (`lsrs r0,r3` in `HAL_RCC_ClockConfig`). Fixed for
  16-bit LSR/ASR/ROR + FA32 LSR/ASR/RORS.
- **16-bit TST decoded as SUB**: `tst r1,r0; beq` in `HAL_GPIO_EXTI_IRQHandler`
  mis-evaluated only when a==b — EXTI0 never cleared PR/called back while
  lines 1/13 worked.
- **No DWT CYCCNT** (new `src/peripherals/dwt.rs`, both maps): `micros()` /
  `TwoWire::recoverBus` spin on `0xE0001004`, which read 0 — I2C recovery hung
  forever natively. Tracks `INSTRUCTION_COUNT` (1 instr = 1 cycle, like TIM).
- **`take_exception` never cleared `sleeping`**: a WFI core hung forever even
  when dispatched. Exception entry wakes the core now.
- Harness (not product): SPI flash CS pins in the native smoke test (`PA4` /
  `PB12` — page-program never finalizes without CS edges).

## Parity notes (bisected, not bugs)

- UART log insertions (`[GP¥IO]`, `[I2C2 EZEPROM]`) are a **shared firmware
  race**: `Serial.print` queues into the TX ring drained per batch while
  loopback/USART2/DMA write DR directly and overtake — byte-identical on
  Unicorn. The DMA `D` lands ±1 byte apart between backends (benign interleave
  slop). Real hardware races the same way.
- Prompt (inline) delivery is proven (`periph39_inline_irqs_native`: every
  vector + SVC served, no faults) but 36/39: `[EXTI reg]` (handler clears PR0
  before the test reads it) and `[DMA RX]`/`[UART RX]` (AB-stdin timing) assume
  lazy batch-boundary latency. Product paths keep lazy dispatch on both CPUs.

## Benchmarks (same machine, release, 20K slices/batches)

| Workload | Rust CPU | Unicorn | Ratio |
|---|---|---|---|
| periph39 200M, 39/39 both | 2.68–2.79s → **72–75 MIPS** | 9.33–9.82s → **20–21 MIPS** | **~3.5×** |
| blink 10M | 0.17–0.18s → **~58 MIPS** | 0.96s → **~10.4 MIPS** | **~5.5×** |

Native wins: Unicorn here is TCI plus JS/WASM crossings on RAM ops and mem
hooks; native keeps CPU+model in one binary with direct model calls. (Native
handler dispatch is single-stepped — that overhead is *included* above.)

## Tests

- `cargo test --release --lib cpu::smoke` — blinky, echo, periph39 39/39,
  `firmware_gallery_native` (adc/timer/fade/flash/showcase/ws2812, live-digit
  assertions), `periph39_inline_irqs_native` (delivery proof, see note).
- `cargo test --release --lib cpu::core_tests` — WFI sleep/wake, PSP switch.
- `node tests/test_rustcpu.mjs` — 200M periph39 via the page path (CI-wired).
- `node pkg/cli.mjs --cpu=rust --config=...` — headless backend + benchmark.

## What's left

- Headed browser sweep of the single backend (headless CDP green:
  periph39 200M 39/39 @ ~96 MIPS, 7-seg decode live).
- Vendored-core license (see below).
- Soak: any latent decoder gap now faults loudly via `rustcpu_fault()` +
  UNDEFINSTR escalation instead of hanging silently.

## License note (vendored CPU core — resolved MIT)

- Provenance: `src/cpu/` was extracted verbatim from
  `danish9661/stm32F4-emulator`, path `stm32-periph-wasm/src/cpu/`, commit
  `3df073e` (`feature/wasm-cpu`) — then ported M4→M3 on this branch (DSP
  gates, IT-suppress flag rule, register-shift fixes, TST-as-AND, WFI wake).
- That repo's `LICENSE` is the full **GPL-3.0** text (verified 2026-09-04).
- **Resolution (2026-09-04): the author confirms the vendored portion is
  released under MIT for this project** (recorded in the `src/cpu/mod.rs`
  header). Shipped artifacts (`stm32_bluepill_wasm_bg.wasm`, npm files)
  stay MIT like the rest of the repo. This note is a record, not a legal
  determination.
