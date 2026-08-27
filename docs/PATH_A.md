# Path A — Investigation & Decision (ABANDONED)

> Status: **abandoned**. The dual-module architecture is the supported product.
> See `AGENTS.md` ("Next Phase — Long-term Optimizations", item 1) for context.

## Goal

Path A was an experiment to merge the project's two-module emulation stack into
a **single self-contained WASM module**:

- Rust peripheral code (`src/`) compiled to a `staticlib`
- linked together with the **Unicorn Cortex-M C engine** (`libunicorn.a`) by `emcc`
- the two sides talking through raw `#[no_mangle] extern "C"` exports
  (`src/raw_exports.rs`) instead of wasm-bindgen

The hoped-for payoff: remove the per-peripheral-access JS boundary (the
dual-module runs a `memReadHook`/`memWriteHook` in JS for every CPU↔peripheral
transaction) and ship one artifact instead of the glue + Unicorn addon + wasm
trio.

## Approach attempted

1. Rust `staticlib` with `#[no_mangle] pub extern "C"` wrappers mirroring the
   wasm-bindgen API, [`panic = "abort"`][profile.dev] so the
   `wasm32-unknown-emscripten` staticlib links cleanly.
2. `emcc` links `libunicorn.a` + the Rust `staticlib` into one module, using
   `uc_mem_map_ptr` to share Unicorn's linear memory with the Rust peripheral bus
   (no JS crossing on memory access).
3. A runner (`pkg/merged_emulator.js`) with the same surface as `pkg/emulator.js`
   (`createEmulator`, `run`, `step`, hooks).

## Blockers (why it failed)

**1. Unicorn 2.1.4 does not build with emcc 6.0.6.**
Unicorn 2.1.4 is based on QEMU 5.0.1 C code, which predates modern clang's
strictness. The compile fails with:

- `int128` redefinition — Unicorn/QEMU's `__int128` helpers collide with the
  toolchain's builtins.
- `PROT_READ` / `MAP_PRIVATE` undeclared — QEMU 5.0.1 is missing the
  `<sys/mman.h>` includes newer emscripten expects.
- multiple **implicit-function-declaration** errors (now hard errors in modern
  clang) for POSIX/BSD functions.

Patching QEMU 5.0.1-era code to build under a current LLVM is substantial and
fragile — and would have to be re-done on every Unicorn bump.

**2. The available Unicorn C core has a decode bug.**
The Node `unicorn_arm.cjs` and the attempted `merged_unicorn_arm.cjs` mis-decode
Thumb **shift/store** instructions. The only *working* Unicorn
(`pkg/unicorn_arm.js`) is **browser-only** (no Node export), so it cannot serve
as the Node-side merged artifact. Even a successfully-linked merged module would
mis-emulate real firmware.

**3. Raw C FFI has no safety net.**
`src/raw_exports.rs` (~70 exports) must manually mirror Unicorn's struct layouts
and register/exception enums with no wasm-bindgen checks; any drift breaks
silently at runtime.

**4. Shared-memory rewrite.**
Making the Rust peripheral bus read Unicorn's mapped RAM directly (instead of via
JS hooks) is a non-trivial change to `src/bus.rs` / `src/lib.rs`.

## Decision

**Abandon Path A.** The dual-module architecture remains the product:

- `pkg/emulator.js` — JS glue (wasm-bindgen)
- `pkg/stm32_bluepill_wasm_bg.wasm` — Rust peripherals
- `pkg/unicorn_arm.{js,cjs}` — Unicorn Cortex-M core

Rationale:

- Blocker (1) is fundamental (old QEMU code vs modern LLVM), not a quick fix.
- Even if built, the per-access JS-hook boundary is only **~2.5% of runtime**
  (Unicorn's TCI interpreter is ~97.5%), so merging would not move the
  **~22 MIPS** ceiling meaningfully.
- A correct merged module still needs a working, accurate Unicorn C core — which
  we do not have (blocker 2).

## Artifacts preserved (reference only — NOT functional)

On the `path-a` branch (pushed to `origin/path-a`):

- `pkg/merged_emulator.js`, `pkg/merged_wasm.js` — glue scaffolding. These were
  recreated from memory after an accidental deletion; they were **never built
  into a working single module**.
- `docs/PATH_A.md`, `docs/JIT_PORT.md`, `docs/PERFORMANCE_EXPERIMENTS.md`,
  `summary_research.md` — earlier notes.

> **Caveat on those notes:** `path-a:docs/PATH_A.md` describes an optimistic
> scenario ("merged module achieves 24.1 MIPS / 1.3–1.7× speedup / 39/39"). That
> was **never actually achieved** — it predates the build failures above. Treat
> those numbers as *aspirational*, not measured. This document is the accurate
> record of the decision.

## What would change the decision

- A Unicorn release that builds cleanly under a current emscripten, **or** a
  pure-Rust Cortex-M core (see `docs/NEXT_PHASE.md` "Path B"), removing blockers
  (1) and (2).
- Until then, the dual module is the supported and verified path (236/236 unit
  tests, 39/39 firmware checks).
