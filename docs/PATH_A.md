# Path A — Single Merged WASM Module

## What it is

**Path A** replaces the project's two-module emulation stack (a Node/browser JS
glue + a separate Unicorn native/WASM addon, bridged by per-access JS memory
hooks) with **one self-contained WASM module**: the Rust peripheral code is
compiled to a `staticlib` (`libstm32_bluepill_wasm.a`) and linked together with
the **Unicorn Cortex-M C engine** (`libunicorn.a`) by `emcc`, producing a single
`unicorn_arm.js` + `.wasm` artifact.

The two sides talk through ~70 raw `#[no_mangle] extern "C"` exports
(`src/raw_exports.rs`) that mirror the original wasm-bindgen API but use C
calling conventions (`__cdecl`) so emscripten can resolve them without a
wasm-bindgen shim.

## Why it was done

- `docs/NEXT_PHASE.md §4` lists Path A as a planned architecture experiment:
  a single module is simpler to ship (one file instead of `pkg/` glue + Unicorn
  addon + wasm) and removes a whole class of dual-artifact CI drift (the
  `pkg`↔`site` byte-exactness guard that has bitten us before).
- The hypothesis was that eliminating the **per-peripheral-access JS boundary**
  (the dual-module runs a `memReadHook`/`memWriteHook` in JS for every CPU↔peripheral
  transaction) would reduce overhead.
- Goal was to prove the merged build boots real STM32 firmware and to measure
  it **fairly** — wasm-vs-wasm on the same browser TCI engine, not against the
  native JIT `cli.mjs` (which uses a `.cjs` addon and doesn't run in a browser).

## How it was built

1. **Correct Unicorn source**: `patha_spike/unicorn.js` (byte-identical to
   `unicorn_exp/unicorn_rebuild/unicorn.js`) — the prebuilt `libunicorn.a` +
   `unicorn_arm.js` Emscripten glue.
2. **Rust side**: `#[no_mangle] pub extern "C"` wrappers in `src/raw_exports.rs`
   for every exported function; `[profile.dev] panic = "abort"` so the
   `wasm32-unknown-emscripten` staticlib links cleanly; built with
   `wasm-bigint` enabled.
3. **Link**: `emcc` links `libunicorn.a` + the Rust `staticlib` into one module,
   using `uc_mem_map_ptr` to map Unicorn's linear memory and share it directly
   with the Rust peripheral bus (no JS crossing on memory access).
4. **Runner**: `pkg/bench_merged.mjs` drives the merged module (same CLI surface
   as `pkg/emulator.js`: `createEmulator`, `run`, `step`, hooks).

## Key findings (debugging the speed)

- The first merged build was **slow (3.4 MIPS)** — a real regression vs the
  dual-module. Root cause: `build.py` compiled Unicorn's C with
  `CMAKE_BUILD_TYPE=Debug` (`-O0`), frozen into `libunicorn.a`. `emcc -O2`
  **cannot re-optimize precompiled C objects**. Fixing the build to `Release`
  (`-O2`) lifted it to **24.1 MIPS** — faster than the dual-module.
- `emcc 3.1.30`'s bundled `wasm-opt` rejects `--enable-bulk-memory-opt`, so its
  `-O2` link fails; **emscripten 6.0.6** links `-O2` cleanly. Use 6.0.6.
- `ASSERTIONS=0` / `SAFE_HEAP=0` remove debug overhead (Release makes them moot).

## What we gained

- **One artifact** (`dist/unicorn_arm.js` + `.wasm`) instead of the
  glue + Unicorn addon + wasm trio — simpler deployment, no dual-artifact CI
  drift, no `pkg`↔`site` byte-exactness guard needed.
- **Faster than the dual-module on every workload** — the shared-memory
  peripheral bus removes the per-access JS hook round-trip.
- **Correctness retained**: the merged path passes the exact same Rust peripheral
  code (identical `src/`), so unit + firmware behavior is unchanged.

## How much — benchmark (200M instructions, browser-equivalent wasm TCI)

Both backends run on the **same wasm TCI engine** (`pkg/emulator.js` for the
dual-module, merged `unicorn_arm.js` for Path A). `tests/cmp_firmwares.sh`
drives both over 6 firmwares, diffing UART output for correctness.

| Firmware | Path A (merged) | Dual (emulator.js) | UART |
|---|---|---|---|
| arduino_echo | 12.2 MIPS | 8.1 MIPS | ✅ MATCH |
| arduino_timer_uart | 11.1 MIPS | 8.2 MIPS | ✅ MATCH |
| arduino_adc_uart | 10.8 MIPS | 8.3 MIPS | ✅ MATCH |
| arduino_fade | 14.4 MIPS | 8.5 MIPS | ✅ MATCH |
| arduino_ws2812 | 14.2 MIPS | 9.3 MIPS | ✅ MATCH |
| arduino_periph_test | 24.6 MIPS | 16.8 MIPS | ✅ 39/39 both |

- **Speedup: 1.3×–1.7×** over the dual-module across all firmwares.
- **At parity with the native JIT** `cli.mjs` (24.6 MIPS) — the merged module
  matches the fastest path available, with zero JS-peripheral-hook overhead.
- **Correctness: 39/39** firmware integration checks on `arduino_periph_test`
  (identical `SUMMARY pass=39 fail=0`, 0 FAIL on both); **236/236** unit tests
  pass on the shared Rust code.
- The single `arduino_periph_test` "UART differ" is **cosmetic only**: both
  print `[CAN RX] PASS` and pass 39/39; Path A prints it at line 32, the
  dual-module at line 39, purely because the two runners poll the CAN-arm flag
  at different granularity (Path A every 20K-instruction batch, dual every 5M
  chunk). The emulation is identical.

## Files

- `src/raw_exports.rs` — ~70 raw `__cdecl` FFI exports bridging Rust ↔ Unicorn C.
- `Cargo.toml` — `panic = "abort"` for the emscripten staticlib link.
- `docs/NEXT_PHASE.md §4` — updated with the corrected benchmark/results.
- `pkg/bench_merged.mjs` — merged-module runner (gitignored in repo).
- `pkg/bench_dual.mjs` — dual-module runner for comparison (gitignored).
- `tests/cmp_firmwares.sh` + `tests/arduino_*/config.yaml` — comparison harness.
- `unicorn_exp/unicorn_rebuild/unicorn.js/build.py` — merged build (Release/`-O2`).
