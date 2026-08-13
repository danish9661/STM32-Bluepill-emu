# Path A — single-module build recipe (spike artifacts)

Everything here lives outside the repo (dual-wasm remains the shipped default).
The merged module = Rust peripherals + Unicorn C linked into ONE emscripten wasm,
with a hand-written loader shim (wasm-bindgen CLI cannot process the output).

## Build

```bash
# 1. Unicorn C (source of record: AlexAltea/unicorn.js)
cd third_party/unicorn-js/unicorn && ./make.sh   # -> build/libunicorn.a (clone AlexAltea/unicorn.js)

# 2. Rust peripherals as an emscripten staticlib
cargo build --target wasm32-unknown-emscripten --release
# -> target/wasm32-unknown-emscripten/release/deps/libstm32_bluepill_wasm.a

# 3. Link (table growth is REQUIRED for hook_add callbacks)
emcc third_party/unicorn-js/unicorn/build/libunicorn.a \
     target/wasm32-unknown-emscripten/release/deps/libstm32_bluepill_wasm.a \
     -o merged4.mjs \
     -sWASM_BIGINT=1 -sINITIAL_MEMORY=134217728 -sALLOW_MEMORY_GROWTH=1 \
     -sALLOW_TABLE_GROWTH=1 -sRESERVED_FUNCTION_POINTERS=256 \
     -sERROR_ON_UNDEFINED_SYMBOLS=0 -sMODULARIZE=1 -sEXPORT_NAME='Merged' \
     -sEXPORTED_FUNCTIONS=<~84: 30 uc_* + _fflush + _malloc/_free/_realloc + 54 _<periph> names> \
     -sEXPORTED_RUNTIME_METHODS=ccall,addFunction,removeFunction,getValue,setValue,writeArrayToMemory,HEAPU8,HEAPU32
```

## Loader invariants (loader-core.mjs) — why each piece exists

1. **Exports are plain names** (`add_i2c_eeprom`, …) bound as `Module._name`;
   all 54 periph exports use the classic wasm-pack ABI: scalars direct, strings
   (ptr,len), Vec returns via caller scratch (i32 ptr), `i2c_oled_writes` -> i64
   BigInt. No `__wbindgen_*` describe metadata is needed.
2. **Arg buffers are freed by the glue, NOT the caller** — the first bug
   (`Aborted(native code called abort())`) was a double-free in the shim.
3. **JsValue = u32 index** into a JS-side registry (old wasm-bindgen ABI, no
   externref). `__wbg_call_*` args are ALL indices (fn, this, args); returns are
   registered (`objIdx`) and re-resolved by `__wbg___wbindgen_number_get`.
4. **`__wbindgen_describe_cast(fn, ptr)`** is the unrewritten `wbg_cast` runtime:
   `breaks_if_inlined` packs the f64 at struct offset 0 then does
   `ptr::read(describe_cast(fn, ptr))` to obtain the JsValue index. The adapter
   registers the f64 as a JS number and returns a scratch pointer holding its
   index — otherwise the JsValue is garbage (was `ptr::read(0)` = heap byte
   soup like 0x63676465).
5. **unicorn-js/emcc semantics**: `emu_start` returns undefined (throws a string
   on error); `begin=0` SETS PC to 0 (no resume); `until=0` stops at address 0 —
   the JS loop must read back PC each batch and pass `(pc|1)`. Periph ranges
   must be MAPPED (PROT_READ|PROT_WRITE, no exec) and hooked
   `[0x40000000,0xB0000000]` + `[0xE0000000,0xE1000000]`; MEM hooks are void
   callbacks, values injected via `uc.mem_write`; INTR hook receives
   (handle, intno, user_data). Dist class: `handle_ptr` + `reg_read_i32` /
   `reg_write_i32` only (no reg_read_batch methods; use
   `Module.getValue(uc.handle_ptr,'*')` + `Module.ccall('uc_reg_read_batch',…)`
   like emulator.js does).

## Acceptance evidence (all green)

- `node test_all_merged.mjs` — 236/236 unit tests on the merged module
- `node test_emulator_merged.mjs` — periph39 200M, 39/39 (~14s, ~14 M IPS)
- `echo -n "AB" | node cli-merged.mjs --config=<repo>/tests/arduino_periph_test/config.yaml --max=200000000` — 39/39 in ~8.2s (~24 M IPS, beats ~22M dual-wasm baseline)
- `node browser-smoke.mjs` — headless Chrome CDP, 200M, SUMMARY 39/39
- Dual-wasm regression net (repo, untouched): 236/236 + canary 39/39

## Not yet done (CI story)

- CI build needs emcc + unicorn source checkout (gitignored) — documented here,
  no CI job yet. Real-browser test needs chrome; the smoke script spawns it.
