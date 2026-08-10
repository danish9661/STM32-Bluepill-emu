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
  went 10.86s → ~9.0s (**~22M IPS** measured, browser demo 0.5s for the full firmware run).
- The "single WASM module" idea was therefore evaluated as **not worth the rebuild** —
  the JS boundary was the whole cost, and it is already gone without any rebuild.

Current status: **203/203 unit tests**, **39/39 firmware checks**, canary ~25s at 100M.

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
- DMA: replace the JS `dma_get_all_pending()`/`uc.mem_read|mem_write`/`dma_set_completed`
  round trip with a Rust-side `memcpy` across flat memory (drops 1 WASM crossing per
  batch; DMA count is tiny but removes a whole class of JS state).
- Interrupts: run IRQ handling inside Rust (`uc_intr`-style injection or stop+re-exec),
  instead of the JS stack-frame push/pop.

### 3. Evaluate a pure-Rust Cortex-M3 emulator (`cargo-cortex-m` / `mdl`)
Replace Unicorn entirely: no C build, no `unicorn_arm` binary addon, coherent memory
model (no JS hooks at all — peripherals are plain memory reads). Measure against the
~22M IPS baseline before committing.

### 4. Single WASM module (Emscripten link) — only if steps 2–3 don't land
The old build plan (Emscripten toolchain, `staticlib` Cargo change, `#[no_mangle]` raw
exports, `uc_mem_map_ptr` on a shared heap) is still in git history (pre-2026-08-09
version of this file) if ever needed, but expected gain is now **marginal** — the
historical "5–10×" estimate was wrong, not the plan.

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
| Instructions per second | ~22M (200M in ~9.0s; 100M in ~4.8s) |
| Hooks per instruction | 0.001 (peripheral access — not a bottleneck) |
| Batch size | 20K (5× lower IRQ latency vs 100K, zero speed cost) |
| Canary | 39/39 firmware checks, ~25s at 100M |
| Unit tests | 203/203 |