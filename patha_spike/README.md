# Path A spike — single WASM module (Rust staticlib + Unicorn C via emcc)

**Status (2026-08-24): MECHANISM PROVEN.**

This directory is a minimal proof-of-concept that the core Path A link works:
a Rust crate compiled to a `staticlib` for `wasm32-unknown-emscripten`, linked
together with a C `main` by a single `emcc` invocation, producing ONE wasm
module that runs in node.

## Proof

`src/lib.rs` exports two raw `#[no_mangle]` functions. `main.c` calls them.
Building + linking + running:

```bash
# 1. Rust staticlib (host, rustc 1.97.1, wasm32-unknown-emscripten target installed)
cargo build --target wasm32-unknown-emscripten
# -> target/wasm32-unknown-emscripten/debug/libpatha_spike.a

# 2. Link with C via a MATCHED emscripten toolchain (host emsdk install)
source /tmp/opencode/emsdk/emsdk_env.sh
emcc main.c target/wasm32-unknown-emscripten/debug/libpatha_spike.a -o spike.js -O2

# 3. Run
node spike.js
# spike_add(3,4)=7 spike_multiply(3,4)=12
# PATHA_LINK_OK
```

## Why the host emscripten matters (toolchain version match)

`qbuild2` has `emcc 3.1.50` but pairs it with binaryen 132, which *removed*
`--enable-bulk-memory-opt`. rustc 1.97.1's emscripten target emits that feature
flag, so linking in `qbuild2` fails at the `wasm-opt` step. A fresh, self-
consistent emsdk on the host (matched emcc + binaryen) links cleanly. Lesson for
the real port: keep emcc + binaryen + rustc's emscripten target on one matched
toolchain.

## What the REAL Path A requires (next, on branch `mergedwasm`)

1. Port the ~70 `#[wasm_bindgen]` exports in `src/lib.rs` to raw `#[no_mangle]`
   (52 `wasm_bindgen` references today).
2. Remove `wee_alloc` / `js-sys` / `web-sys` (none survive the emscripten
   target); re-express JS interop via a hand-written shim.
3. Compile the classic Unicorn C (`unicorn_rebuild/unicorn.js/`, the QEMU-based
   Unicorn 2.x that builds `pkg/unicorn_arm.cjs`) via emcc and link it INTO the
   same module as the Rust staticlib (modify `unicorn.js/build.py`'s emcc link
   to also include the Rust `.a`).
4. Write a JS shim so `cli.mjs` / `emulator.js` / `site/` work unchanged.
5. Re-verify `tests/test_all.mjs` 236/236 + `canary.mjs` 39/39 + 200M + browser.

## Caveat (from docs/NEXT_PHASE.md §4)

Path A's gain is **architectural only — NOT a speed win**. unicorn.js (TCI) is
~22–24 MIPS and stays the engine; qemu-wasm JIT is slower. Path A buys "one
module, no glue drift" at high effort and a real risk of failing the
"within ~2× of 22M IPS" gate.
