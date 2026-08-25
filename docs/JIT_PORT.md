# JIT / TCI Port Notes — Path A Unicorn build

This documents the attempt to rebuild the merged Unicorn engine (`merged_unicorn_arm.cjs`)
from source so that Path A (single combined wasm: Unicorn C + Rust peripherals) can ship.

## Source layout
- `patha_spike/unicorn.js/` — alexaltea Unicorn 2.1.4 fork (submodule `unicorn/` = QEMU 5.0.1),
  with TCI (tiny code interpreter) patches. `build.py` compiles to `dist/unicorn.js`
  (SINGLE_FILE=1, EXPORT_NAME='MUnicorn'). This is byte-identical to the working
  `unicorn_exp/unicorn_rebuild` and to the browser-only `pkg/unicorn_arm.js`.

## Toolchain
- emsdk at `/home/danish1075/emsdk`; `source emsdk_env.sh` → `emcc 6.0.6`
  (ce75e06884093bcefb86a6b8fd56a5d62a4cc245). This is the version `docs/NEXT_PHASE.md`
  requires (its wasm-opt accepts `--enable-bulk-memory-opt` for the final -O2 link).

## Failure: Unicorn 2.1.4 does NOT compile with emcc 6.0.6
Errors encountered (in order) when running `python3 build.py arm`:

1. **Spaced project path** — `ERROR: main directory cannot contain spaces nor colons`.
   The repo lives at `/home/danish1075/Documents/stm32 emu blue pill`. cmake/emmake refuse
   paths with spaces. Worked around with a symlink (`/tmp/nbproj -> ...`), but cmake
   canonicalizes the source dir so the warning persists (non-fatal).

2. **`int128` redefinition** — `unicorn/qemu/include/qemu/int128.h:150`:
   `typedef Int128 __int128_t;` clashes with emscripten 6.0.6's clang builtin `__int128_t`.
   Fixed by guarding with `!defined(__SIZEOF_INT128__)` (native int128 available → skip).

3. **POSIX / implicit-declaration cascade** — after the int128 fix, the build fails hard:
   - `qemu/util/osdep.c`: `PROT_READ`, `PROT_WRITE`, `PROT_EXEC`, `PROT_NONE`, `mprotect`
     undeclared. QEMU only includes `<sys/mman.h>` under `CONFIG_LINUX`, which is NOT set
     for the emscripten/wasm target, so the POSIX memory API is never declared.
   - `qemu/util/oslib-posix.c`: `MAP_PRIVATE`, `MAP_ANON`, `MAP_FAILED`, `mmap` undeclared.
   - Many `implicit declaration` / `implicit-function-declaration` errors (clang 16+ treats
     these as hard errors; QEMU 5.0.1 code predates that strictness).

   Passing `-D_GNU_SOURCE -Wno-error=implicit-function-declaration -fgnuc-version=4.2.1`
   via `CMAKE_C_FLAGS` did NOT resolve the `PROT_*`/`MAP_*` errors because the headers are
   simply not included without `CONFIG_LINUX`. This is a genuine QEMU 5.0.1 ↔ modern-clang
   portability gap, not a flag tweak.

## Conclusion
The merge build (Unicorn 2.1.4 + Rust staticlib → one wasm) is **not achievable** with the
installed emscripten 6.0.6. The original working `pkg/unicorn_arm.js` was built with an
*older/unknown* emscripten that tolerated QEMU 5.0.1; that toolchain is not available here.

## Why the broken `merged_unicorn_arm.cjs` also fails at runtime
The artifact actually present (`pkg/merged_unicorn_arm.cjs`, 3.74 MB) was built from
**upstream `unicorn-engine/unicorn`** (NOT the alexaltea 2.1.4 fork). It compiles with 6.0.6
but has a Thumb decode/JIT defect: it faults on `lsls` (shift) and on register-offset / plain
`str` stores (`str r3,[r2,r1]`, `movs r3,r0`) with heap-out-of-bounds depending on context.
Because the firmware uses Thumb shifts and stores constantly, IRQs never get enabled
(`ISER0` stays 0) → async tests hang → IWDG reset → `recoverBus` `delayMicroseconds` hang.
The 2.1.4 fork (TCI) decodes these correctly, which is why `pkg/unicorn_arm.js` passes 39/39.
