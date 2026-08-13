# Architecture — How the Emulator Works

Full-system emulation of an **STM32F103C8 (Bluepill)** running real compiled firmware
(STM32duino/Arduino core, STM32Cube HAL, libopencm3 — anything that targets the real chip).
The CPU is emulated by **Unicorn** (C, compiled to WASM via Emscripten) and every
peripheral is emulated in **Rust** (compiled to WASM via `wasm-pack`). A small JavaScript
layer bridges the two.

```
┌───────────────────────────── JS (pkg/emulator.js / pkg/cli.mjs) ────────────────────────┐
│                                                                                          │
│  HOOKLESS instruction counting — emu_start(begin, 0, 0, maxBatch)                       │
│  stops exactly at maxBatch (faults: ~0.01% of batches, skip + credit full batch)        │
│  memReadHook / memWriteHook → periph_read / periph_write   [fires ~0.1% of instructions]│
│                                                                                          │
│  Loop (1 iteration = 1 batch of 20K instructions):                                       │
│    1. pump stdin → uart_rx_byte()                                                        │
│    2. processDma()              ← move queued DMA data via Unicorn mem_read/write       │
│    3. uc.emu_start(pc|1, 0, 0, 20000)  ← run one batch in Unicorn                       │
│    4. step_batch(20000)          ← Rust ticks ALL peripherals once (instruction-delta)  │
│       (status==1 → watchdog reset requested → stop)                                     │
│    5. processDma()                                                                       │
│    6. processInterrupts()       ← up to 64 IRQs per batch, NVIC-priority ordered        │
│    7. watchdog reset check                                                               │
└──────────────────────────────────────────────────────────────────────────────────────────┘
```

## The two modules

| Module | What it is | Notes |
|---|---|---|
| `pkg/unicorn_arm.cjs` | ARM Cortex-M3 CPU (Unicorn engine) | Emscripten-compiled C; **self-contained** — the WASM binary is embedded (`binaryDecode`), no external files, works in Node and browser. Effectively unmodifiable (binary). |
| `pkg/stm32_bluepill_wasm_bg.wasm` | All peripherals in Rust | wasm-bindgen exports; the JS glue is `pkg/stm32_bluepill_wasm.js`. Source lives in `src/` (`src/bus.rs`, `src/peripherals/*`, `src/ext_devices/*`). |

Both are instantiated once and communicate only through the JS bridge — Unicorn never
calls into Rust directly and vice versa.

## The peripheral bus (rp2040js-style)

`src/bus.rs` is a runtime registry in the style of Wokwi's `rp2040js`/`avrjs`
`bus.ts`: peripherals register an address range `[start, end)` and accesses are
routed with a binary search. Board assembly picks the peripheral set:

| Layer | rp2040js/avrjs equivalent | Ours |
|---|---|---|
| CPU core | `src/cpu/` (own ARM core) | Unicorn (separate, faster) |
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

- **Flash + SRAM** are mapped *inside Unicorn* (`uc.mem_map`), so firmware executes from
  real mapped memory. Flash is mapped with read/execute, SRAM read/write.
- **Peripherals are NOT mapped** as memory. Instead, a memory hook covers the peripheral
  ranges (`0x4000_0000–0xB000_0000` and `0xE000_0000–0xE100_0000`); every read/write in
  that range is forwarded to `periph_read()` / `periph_write()` in the Rust WASM, which
  looks up the register and simulates the peripheral.
- Peripheral accesses are rare in real firmware (~0.001 accesses/instruction, ~27K per
  50M instructions), so the hook crossing costs ~0.1% of runtime. This is why "shared
  linear memory" (`uc_mem_map_ptr` over the Rust heap) was measured and **retired** — there
  was no win left to extract.

## Instruction counting without a hook

The original design counted instructions with a per-instruction JS `codeHook`
(`instCount++; batchInstCount++;`). Profiling showed that cost **~20% of runtime**
(200M instructions: 10.9s with hook → 8.7–9.1s without). Because every batch is run as
`uc.emu_start(pc, 0, 0, maxBatch)`, Unicorn stops *exactly* at `maxBatch` instructions, so:

- a **normal batch** executed exactly `maxBatch` instructions → credit `maxBatch`;
- a **faulted batch** (unmapped read/write/fetch, measured at 1 in 9988 batches) is
  handled by the fault path (see Exceptions): the known Unicorn `bl` artifact at
  `HAL_NVIC_EnableIRQ` is skipped (`PC+2`), any other unmapped fault is raised into the
  firmware as a real fault, and the batch is credited in full — overcounting < 1 batch,
  invisible;
- **interrupt handler runs** (inside `processInterrupts`) are not counted — peripheral
  timers are instruction-delta based and self-correct.

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
- After each batch, JS calls `has_pending_interrupt()` /
  `get_next_pending_interrupt()` and runs the handler via
  `uc.emu_start(handler_pc, ...)`, saving/restoring the interrupted context in JS
  variables (see workarounds) — including **xPSR**: the handler's emu_start clobbers
  APSR, so failing to restore xPSR makes any `cmp`/branch pair straddling a batch
  boundary evaluate with the *handler's* flags (a TIM2 ISR landing between a guard's
  `cmp` and `beq` duplicated a demo's print line every second). cli.mjs processes up to
  **64 IRQs per batch** (the browser version drains all pending); the NVIC's
  `last_popped` fairness makes a hot self-re-pending IRQ (e.g. USART TXE while draining
  a ring buffer) alternate with other pending IRQs instead of starving them.
- Interrupt latency is bounded by the batch size: **20K instructions ≈ 1.1 ms** at real
  speed (was 5.4 ms at 100K).

## Exceptions (SVC, PendSV, faults)

Beyond IRQs the emulator models the system exceptions that real firmware exercises:

- **SVC**: Unicorn's `HOOK_INTR` fires with `intno 2` on `svc`. The JS bridge stacks the
  interrupted context (R0–R3, R12, LR, PC, xPSR — written to the real stack so handlers
  can inspect it, with a JS mirror for restoration), sets `LR` to the EXC_RETURN value
  (0xFFFFFFF9, or 0xFFFFFFFD when `CONTROL.SPSEL` is set), and jumps to the SVCall vector
  (exception 11). Unicorn cannot execute `bx lr` with an EXC_RETURN value (fetch fault at
  0xFFFFFFFx), which the main-loop fault handler recognizes and pops the mirror stack.
  Nested SVCs are bounded (depth 8).
- **PendSV**: pended normally via `ICSR.PENDSVSET` (SCB register write → NVIC pending),
  dispatched by the same `processInterrupts` path as IRQs.
- **Faults**: a real unmapped access (anything other than the known Unicorn `bl` artifact
  at `HAL_NVIC_EnableIRQ`, which is skipped) calls `raise_fault(kind, addr)` in Rust:
  CFSR (IBUSERR/PRECISERR/UNDEFINSTR), BFAR + BFARVALID, and HFSR FORCED are recorded,
  and the fault exception is pended — BusFault (or UsageFault for undefined instructions)
  if the corresponding SHCSR enable bit is set, otherwise **escalated to HardFault**.
  Priority of the system handlers is programmable via SCB SHPR1–3, which the SCB
  peripheral routes into the NVIC's `sys_handler_priority` table.
- Fault handlers run through `processInterrupts` and re-execution of the faulting
  instruction is left to the handler (as on real hardware).

## DMA

DMA transfers cross the WASM boundary in batches:
1. Rust queues transfers (`queue_dma_transfer`, deduped per channel while queued).
2. JS pulls them with `dma_get_all_pending()` (one WASM call, batched).
3. JS moves the bytes with Unicorn `uc.mem_read`/`uc.mem_write` (direction decoded
   from the queue entry: 0 = peripheral→memory, 1 = memory→peripheral, 2 = memory→memory).
4. JS calls `dma_set_completed_many(bits)`; the next `step_batch` finishes the channel
   (CNDTR counts down, transfer-complete IRQ fires).

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

## Known workarounds (temporary, in the JS layer)

1. **`mrs rX, msp` → `mov rX, sp`** (`patchMrsMsp` in cli.mjs): Unicorn cannot decode the
   Thumb `mrs` instruction used by newlib `_sbrk`; rewritten to a 4-byte equivalent.
2. **i2c_init NVIC patch** (cli.mjs ~line 252): Unicorn skips two `bl HAL_NVIC_EnableIRQ`
   calls, so the block is replaced with inline ISER0/ISER1 writes.
3. **hi2c->Mode patch** (`memWriteHook`): when I2C1 DR is written with the read bit,
   `*(hi2c)+0x3D` in RAM is patched to `0x22` (MASTER_RX) — the HAL I2C ISR requires it.
4. **Interrupt frame in JS closures**: stack frames are clobbered by handler PUSH;
   R0–R3, R12, LR, PC, xPSR are saved/restored in JS locals around handler runs.
5. **64-IRQ loop** + NVIC `last_popped` fairness: prevents a hot re-pending IRQ from
   starving the rest.
6. **DMA batching**: one `dma_get_all_pending()` WASM call instead of seven.

## Performance

| Metric | Value |
|---|---|
| Throughput (periph39 firmware) | **~24M instructions/sec** (200M in ~8.3s) |
| Browser demo (periph39 full run) | ~0.5 s wall |
| Batch size | 20K instructions (≈1.1 ms IRQ latency) |
| Memory | stable ~150 MB RSS, no growth with instruction count |
| Per-instruction JS cost | only the mem hooks (~0.1% of instructions) |

Historical optimizations, in order: per-instruction tick → once-per-batch `step_batch`
(3.8×), plain-number counters (1.19×), **hookless batch crediting** (1.16×), 20K batches
(latency, free), **closed-form timer advance** (1.15×; step_batch 1409ms → 11ms — the
only remaining O(ticks) loop was `tim.rs advance()`, rewritten to jump directly
to update/compare-match event ticks with bit-identical event sets; Unicorn TCG
is now ~97.5% of runtime, so the JS/Rust layers are exhausted).
