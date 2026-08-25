# Path A Research Summary

## Goal
Make Path A — a single merged wasm module (Unicorn C engine + Rust peripherals linked by
emcc) — a drop-in replacement for the dual module (wasm-bindgen Rust + Unicorn Node addon),
and repoint `site/index.html` to it.

## Design (as implemented in the Path A commits on `origin/path-a`)
- Rust exposes raw `#[no_mangle]` extern-C exports (`src/raw_exports.rs`) so emscripten can
  link them with the Unicorn C engine into ONE wasm (`merged_unicorn_arm.cjs`).
- `pkg/merged_wasm.js` is a wasm-bindgen-compatible glue over that combined module (same
  exports `tests/test_all.mjs` and `emulator.js` expect).
- `pkg/merged_emulator.js` is the high-level `Emulator` class (run loop, SVC/IRQ, DMA pump,
  hooks) ported from `bench_merged.mjs`, loading the combined module.
- Shared linear memory: Unicorn and Rust live in one wasm, so Rust reads/writes Unicorn RAM
  directly (no per-byte JS round-trip). This is the architectural win of Path A.

## What was proven
- The 5 Path A commits on `origin/path-a` show the single module "validates on 6 firmwares,
  faster than dual" at one point. The raw-export FFI refactor (`ce68919`) and the staticlib+C
  link spike (`d3557a1`) are committed and pushed.

## What blocked it (this session)
1. The committed `merged_unicorn_arm.cjs` is **Unicorn-only** (built from upstream
   `unicorn-engine/unicorn`), not the combined module. Every Rust export
   (`_periph_read`, `_init`, …) is missing → `merged_emulator.js` / `merged_wasm.js` crash
   with `u._reset_ext_devices is not a function`. The combined module was never actually built.
2. Rebuilding the combined module from the alexaltea 2.1.4 fork fails under emcc 6.0.6
   (see `docs/JIT_PORT.md`): int128 redefinition + QEMU 5.0.1 POSIX/implicit-decl errors.
3. The upstream-built `merged_unicorn_arm.cjs` has a Thumb shift/store decode defect, so even
   as a Unicorn engine it can't run the firmware (IRQs never enable → hang).
4. The only working Unicorn binary (`pkg/unicorn_arm.js`) is **browser-only** — `require()`
   in Node returns `{}` (no Node export). It passes 39/39 in the browser demo.
5. The raw Rust API requires shared memory (the combined module); it cannot run as *separate*
   Unicorn + Rust modules because pointer params cross module boundaries. So swapping in
   `unicorn_arm.cjs` + `merged_wasm.js` separately does not work.

## Decision
Path A (single combined wasm) is **not achievable** with the available toolchain. The working
product remains the **dual module** (`emulator.js` + `unicorn_arm.cjs` + `stm32_bluepill_wasm.js`,
39/39). The `merged_*` files are preserved on disk/branch for future reference but are not the
active path. The 5 Path A commits remain on `origin/path-a`.
