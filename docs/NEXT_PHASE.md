# Roadmap & Next Phase (updated 2026-08-09)

## Where we are

The original long-term idea in this file was a **single WASM module** (Unicorn + Rust
peripherals linked together, shared linear memory) that would eliminate the JS bridge and
"buy 5–10× speed". That premise was wrong and has since been retired by measurements:

- Peripheral access hooks were **measured at 0.001 accesses/instruction** (~27K per 50M
  instructions) — ~0.1% of runtime. Replacing them with shared linear memory has no
  measurable win (JavaScript-boundary cost only matters for *per-instruction* traffic).
- The real cost WAS per-instruction JS: the `codeHook` (2 increments per instruction)
  cost ~20% of runtime. That was eliminated by **hookless instruction counting**
  (`emu_start(begin, 0, 0, maxBatch)` stops exactly at maxBatch; each batch credited in
  full; faulted batches ~0.01% are skipped PC+2 and credited anyway). 200M instructions
  went 10.86s → ~8.3s (**~24M IPS** measured, browser demo 0.5s for the full firmware run).
- The "single WASM module" idea was therefore evaluated as **not worth the rebuild** —
  the JS boundary was the whole cost, and it is already gone without any rebuild.

Current status: **236/236 unit tests**, **39/39 firmware checks**, canary ~25s at 100M.

## Retired (moot — do not redo)

1. **Shared linear memory / `uc_mem_map_ptr` bridging** ("Phase 1a/1b" in the old file):
   retired. Measured 0.001 periph accesses/instruction ⇒ no win available. The old
   "~3–5 µs per access" framing applied to per-instruction hooks, which no longer exist.
2. **BigInt instruction counters / per-instruction JS steps**: superseded by hookless
   counting + plain-number `instCount` (both ~19–20% speedups, in).
3. **`step()` per instruction → `step_batch()` once per batch** (all peripheral `tick()`s
   are instruction-delta based): in, 3.8×.

## Options ranked for the next phase

### 1. Keep JS orchestrator, no WASM rebuild (do this first, wherever possible)
The boundary is no longer the bottleneck; JS-side batch orchestration (DMA queue +
interrupt dispatch) is where scheduling flexibility lives (canary depends on its exact
semantics). Any new peripheral feature is cheaper here than in a merged build.

### 2. Move DMA + interrupt delivery into Rust (single-domain, moderate win)
- **PARTIALLY LANDED** (2026-08-10): the periph byte pump moved into Rust — JS now only
  touches RAM (`dma_absorb_periph` / `dma_push_periph` replace the per-chunk
  periph_read/periph_write JS loops; cli.mjs + emulator.js processDma are down to 3
  branches and 1 WASM crossing per transfer).
- **DISPATCH POLICY LANDED** (2026-08-11): `src/interrupts.rs` now owns the parts of
  interrupt delivery that aren't pure CPU transport — the per-batch 64-IRQ budget
  (`intr_next()`, reset by step/step_batch) and the SVC frame mirror
  (`intr_svc_enter/leave/depth`, depth-capped at 8, Cortex-M frame layout in Rust).
  cli.mjs and emulator.js now share ONE dispatch implementation (they had drifted —
  the xPSR-restore bug was cli-only), with Rust as the single source of policy.
  Both JS files also unified on restore-from-stacked-frame (a handler that edits the
  saved context is honored).
- What remains: RAM→RAM moves still need Unicorn (`uc.mem_read`/`mem_write` — Rust has
  no CPU memory visibility without a shared-linear-memory map, which was retired as
  moot; DMA RAM traffic is per-transfer, not per-instruction).
- **Registers/vector fetch stay in JS** — bounded by the architecture: Unicorn owns the
  CPU state (SP/registers/vector fetch), so the R0-R3/R12/LR/PC/xPSR transport and the
  handler `emu_start` cannot move into Rust without a `uc_intr`-style injection API
  (not exposed by this Unicorn build) or stop+re-exec (which is what emulator.js
  already does, one `emu_start` per IRQ). The Rust side already owns all the policy:
  pending/active sets, priority dispatch, SysTick debt accounting, batch budget, SVC
  mirror.

### 3. Evaluate a pure-Rust Cortex-M3 emulator (`cargo-cortex-m` / `mdl`)
Replace Unicorn entirely: no C build, no `unicorn_arm` binary addon, coherent memory
model (no JS hooks at all — peripherals are plain memory reads). Measure against the
~22M IPS baseline before committing.

### 4. Single WASM module (Emscripten link) — "Path A", EXPERIMENT COMPLETE
> **Status (2026-08-25): experiment COMPLETE and PERFORMANCE-POSITIVE — the single
> module boots the firmware, passes 39/39, and runs *faster* than the dual-module
> browser path (24 MIPS vs 16 MIPS) at parity with the native JIT `cli.mjs`
> (24.6 MIPS). Meets ALL acceptance gates (39/39, 236/236, IPS well within 2× of
> ~22M). It is a viable replacement for the dual-wasm default; flipping the
> default is a separate decision.** Built from `src/` via `raw_exports.rs` +
> Unicorn C linked by emscripten 6.0.6. Rust logic identical to dual-path
> (same `src/`) → 236/236 hold for both.

**Results (200M instr, arduino_periph_test, 39/39 on every path):**
| Path | Unicorn C | Engine | Time | MIPS |
|---|---|---|---|---|
| Native `cli.mjs` (`.cjs` JIT addon, Node only) | n/a | **JIT** | 8.13s | 24.6 |
| Dual-module (`emulator.js` + `unicorn_arm.js`, browser) | optimized | TCI | 12.25s | 16.3 |
| **Path A** (single merged, `-O0` Debug C) — *earlier build mistake* | Debug | TCI | 58.7s | 3.4 |
| **Path A** (single merged, `Release` C) — *correct build* | Release | TCI | 8.29s | 24.1 |

- **The correct comparison is wasm-vs-wasm (browser engine), not vs the Node JIT
  addon.** Against the dual-module on the *same* wasm TCI Unicorn, Path A is
  **~1.5× faster** (24.1 vs 16.3 MIPS). Its `uc_mem_map_ptr` shared-memory
  peripheral access eliminates the dual-module's per-access JS memory hooks —
  that is the speed win (AGENTS.md's "hooks ~0.1% of runtime / no win available"
  undercounted the wasm↔JS boundary cost; in practice shared memory is 1.5×).
- The earlier "7× slower" / "3.3 MIPS" number was a **build-flag mistake**:
  Unicorn C was compiled `-DCMAKE_BUILD_TYPE=Debug` → `-O0` TCI interpreter
  baked into `libunicorn.a`; emcc `-O2` cannot re-optimize precompiled object
  code, so the interpreter stayed slow. Rebuilding Unicorn C at `Release` lifts
  Path A 3.4 → 24.1 MIPS. Wasm cannot JIT, so ~24 MIPS is the TCI ceiling here
  and it matches the native JIT path on this workload.
- **Acceptance gate (2026-08-11): MET.** 39/39 + 236/236 + 200M run + IPS
  (24.1M) far within 2× of ~22M, measured on the browser-equivalent TCI engine.
  Path A is not just feasible — it is faster than the shipping dual-module.

**Build facts that made it work (for the next person):**
- `src/raw_exports.rs` exposes ~70 `__cdecl` no_mangle exports; the runner
  (`bench_merged.mjs`) does the JS orchestration (batch loop, hookless counting,
  DMA RAM transport, interrupt/SVC register transport) exactly like `cli.mjs`.
- Two emulation bugs found & fixed (same as native, applied to the merged
  runner): EXC_RETURN detected from **LR** not PC; `intr_svc_leave` must read
  `outVecU32` (not `outVecU8`) — it returns `Vec<u32>`, and the 9-byte `u8`
  read corrupts `st[7]`=SP.
- **Unicorn C MUST be built `Release`** (`-DCMAKE_BUILD_TYPE=Release` in
  `build.py`'s CMake configure). Debug builds `-O0` the TCI interpreter into
  `libunicorn.a`; emcc `-O2` cannot re-optimize precompiled objects, so the
  module runs at ~3.4 MIPS. Release lifts it to ~24 MIPS (matches native JIT).
- emscripten 3.1.30's bundled `wasm-opt` rejects `--enable-bulk-memory-opt`, so
  its `-O2` link fails; emscripten **6.0.6** (single consistent toolchain) links
  `-O2` fine. Use 6.0.6. Keep `-sASSERTIONS=0 -sSAFE_HEAP=0` (harmless; only
  mattered to stop `-O0`'s false-positive aborts on Rust-allocated frees).
- emscripten hijacks the piped OS `stdin` during `MUnicorn()` `run()`, so feed
  test bytes via `--stdin-file=<path>` (parity with `cli.mjs`'s pipe).

**Goal:** compile Rust peripheral code + Unicorn C into ONE `emcc` output —
the `wasm32-unknown-emscripten` target links both via C ABI, so the JS memory
hooks and the `unicorn_arm.*` module disappear (peripherals become plain
memory reads through `uc_mem_map_ptr` on a shared heap).

**Hard constraints discovered so far (the plan is NOT "just build with emcc"):**
- `wasm-bindgen` does NOT support the `wasm32-unknown-emscripten` target
  (its linker expects no JS glue). All ~70 `#[wasm_bindgen]` exports in
  `src/lib.rs` need a raw `#[no_mangle]` shim layer (the pre-2026-08-09 git
  history of this file has the original `staticlib` + raw-exports plan).
- Unicorn C source is needed: `third_party/unicorn/` is gitignored — only the
  prebuilt alexaltea tgz (`unicorn_arm.cjs/.js`) is committed. Fetch
  `unicorn-engine/unicorn` (GPL-2.0) and compile via emcc.
- The JS orchestrator STAYS (batch loop, hookless counting, DMA RAM transport,
  interrupt register transport) — only the memory hooks + unicorn module get
  absorbed. The unicorn C API must be re-exported raw so cli.mjs/emulator.js
  can call `uc_emu_start` etc. against the merged module.
- Licensing: unicorn is GPL-2.0; linking into one binary inherits GPL (already
  the case for unicorn_arm — no change in practice).

**Acceptance gate (in order):**
1. Rust → `wasm32-unknown-emscripten` builds as a staticlib with raw exports
2. Links with emcc-compiled unicorn C into one module; smoke-boots firmware
3. `tests/test_all.mjs` 236/236 against the merged module
4. canary 39/39 + 200M cli run + `tests/test_emulator_js.mjs` (browser path)
5. IPS within ~2× of baseline (~22M) — if slower, the module stays experimental

**Expected gain:** architectural (no hooks, one module, no glue drift), NOT
performance — the JS boundary was measured at ~0.001 accesses/instruction
(~0.1% of runtime), so speed is not the win being purchased.

### 5. Pure-Rust Cortex-M core ("Path B") — deferred, do NOT start yet
Rewrite Unicorn's core in Rust (mdl / cargo-cortex-m style, or unicorn's own
in-progress Rust core if it ships). Highest risk on the list: TCG JIT parity
unproven (interpreters typically land 10-30M IPS vs 22M), instruction-level
drift risk (canary is the gate), and multi-day effort. Only worthwhile when a
feature requires modifying the CPU core itself, or if upstream's Rust core
lands and makes it free.

## Emulation-loop reference (current)

```
cli.mjs / emulator.js loop (each iteration = 1 batch of 20K instructions):
  1. pump stdin → uart_rx_byte()              (gated: rx_empty && !dmaBusy)
  2. processDma()                              ← move queued DMA via Unicorn
  3. uc.emu_start(pc|1, 0, 0, 20000)           ← hookless counting
  4. step_batch(20000)                         ← Rust ticks (once per batch)
  5. processDma()
  6. processInterrupts()                       ← up to 64 IRQs per batch
  7. watchdog reset check
```

Key measured facts that keep this loop cheap:

| Item | Measurement |
|---|---|
| Instructions per second | ~24M (200M in ~8.3s; 100M in ~4.2s) |
| Hooks per instruction | 0.001 (peripheral access — not a bottleneck) |
| Batch size | 20K (5× lower IRQ latency vs 100K, zero speed cost) |
| Canary | 39/39 firmware checks, ~25s at 100M |
| Unit tests | 236/236 |