# Next Phase: Single WASM Module (Unicorn + Rust Peripherals)

## Current Architecture (Two WASM Modules)

```
  Unicorn ARM (WASM)  ← JS boundary →  Rust Peripherals (WASM)
       │                                      │
       │  uc_hook_add(memReadHook)             │ periph_read/write
       │  → JS callback ←                      │
       │  → periph_read() → WASM ───────────────
       │  → result → Unicorn memory
       │
       │  DMA: Rust queues transfer ──────────┐
       │    JS reads queue, calls              │
       │    uc.mem_read/mem_write,             │
       │    calls dma_set_completed() ────────┘
```

**Problem:** Every peripheral register access crosses WASM→JS→WASM. ~3-5 µs overhead per access.

## Target Architecture (Single WASM Module)

```
  ┌─────── Single WASM Linear Memory ───────────────────┐
  │   Unicorn ARM (C compiled via Emscripten)            │
  │   Rust Peripherals (Rust static lib, Emscripten)    │
  │   Shared memory region @ 0x40000000                  │
  │                                                       │
  │   uc_mem_map_ptr(0x40000000, size, PROT_ALL, ptr)   │
  │   → Unicorn reads/writes Rust memory directly        │
  │   → No hooks needed at all                           │
  │   → DMA = memcpy inside WASM (Rust side)             │
  └───────────────────────────────────────────────────────┘
```

**Gain:** Zero boundary crossings for memory hooks. Estimated 5–10× speedup.

---

## How to Build (Linux)

### 1. Toolchain Setup

```bash
# Install Emscripten
git clone https://github.com/emscripten-core/emsdk.git
cd emsdk && ./emsdk install latest && ./emsdk activate latest
source ./emsdk_env.sh

# Verify
emcc --version  # should show emcc (Emscripten gcc)
```

### 2. Get & Build Unicorn C Source to WASM

You have the Unicorn source code somewhere. The key is:

```bash
cd unicorn/          # ← your Unicorn source dir

# Unicorn already has Emscripten support at bindings/unicorn_wasm/Makefile
# But we'll do a custom build to link with Rust:

emcc -s WASM=1 \
     -s TOTAL_MEMORY=512MB \
     -s ALLOW_MEMORY_GROWTH=1 \
     -O2 \
     -I include \
     qemu/target/arm/*.c \
     qemu/exec.o qemu/translate-all.o \
     ... (all Unicorn C files) \
     -c -o unicorn.o
```

**Alternative: Use Unicorn's existing WASM build system** as a reference:
```bash
# In unicorn/bindings/unicorn_wasm/
make          # produces unicorn_wasm.js + unicorn_wasm.wasm
```

You can then extract the `.wasm` and examine its exports with:
```bash
wasm-objdump -x unicorn_wasm.wasm | grep Export
```

### 3. Build Rust Peripheral Code for Emscripten

The Rust code currently uses `wasm-bindgen` which generates JS glue for web targets. For Emscripten linking, we need to compile Rust to a **static library** (`libperiph.a`) that Emscripten can link.

**Key changes needed:**

#### `Cargo.toml` — add wasm32 target
```toml
[lib]
crate-type = ["staticlib"]      # ← change from cdylib

[dependencies]
# Remove wasm-bindgen dependency
# Use raw extern "C" exports instead
```

#### `src/lib.rs` — replace wasm-bindgen with raw C exports

Replace every `#[wasm_bindgen]` with `#[no_mangle] pub extern "C"`:

```rust
// Before:
#[wasm_bindgen]
pub fn periph_read(addr: u32, width: u32) -> u32 { ... }

// After:
#[no_mangle]
pub extern "C" fn periph_read(addr: u32, width: u32) -> u32 { ... }
```

Remove `console_error_panic_hook` and `wasm-bindgen` deps. Replace `wasm_bindgen::prelude::*` with nothing.

#### Build
```bash
# Ensure you have wasm32-unknown-emscripten target
rustup target add wasm32-unknown-emscripten

# Build static lib
cargo build --target wasm32-unknown-emscripten --release
# Output: target/wasm32-unknown-emscripten/release/libstm32_bluepill_wasm.a
```

### 4. Link Everything Together

```bash
emcc -s WASM=1 \
     -s TOTAL_MEMORY=512MB \
     -s ALLOW_MEMORY_GROWTH=1 \
     -s EXPORTED_FUNCTIONS="['_uc_open','_uc_close','_uc_mem_map','_uc_mem_map_ptr',
                             '_uc_mem_read','_uc_mem_write',
                             '_uc_hook_add','_uc_hook_del',
                             '_uc_emu_start','_uc_emu_stop',
                             '_uc_reg_read','_uc_reg_write',
                             '_periph_read','_periph_write',
                             '_step_batch','_init']" \
     -s EXPORTED_RUNTIME_METHODS="['ccall','cwrap','getValue','setValue']" \
     -s MODULARIZE=1 \
     -s EXPORT_ES6=1 \
     -O2 \
     unicorn/*.o \
     libstm32_bluepill_wasm.a \
     -o unicorn_periph.js
```

This produces `unicorn_periph.js` (JS glue) + `unicorn_periph.wasm` (single WASM binary containing both Unicorn + peripherals).

### 5. Inside the WASM, Hook Everything Directly

The key architectural change in Rust:

```rust
// Instead of:
// periph_read hooks are JS callbacks delegated by Unicorn

// We do:
#[no_mangle]
pub extern "C" fn init() {
    // (same as current init, but now also sets up Unicorn memory mapping)
}

#[no_mangle]
pub extern "C" fn emulation_loop(max_inst: u32) {
    // Instead of JS calling uc.emu_start + step_batch + processDma + processInterrupts,
    // everything runs inside WASM:

    loop {
        // 1. Run Unicorn for batch
        uc_emu_start(cur_pc, 0, 0, max_batch);

        // 2. Tick peripherals
        for _ in 0..batch_count {
            system.tick();
        }

        // 3. Process DMA (direct memcpy in WASM)
        for transfer in system.pending_dma {
            unsafe {
                std::ptr::copy_nonoverlapping(
                    (memory_base + transfer.src) as *const u8,
                    (memory_base + transfer.dst) as *mut u8,
                    transfer.size
                );
            }
        }

        // 4. Process interrupts (direct Unicorn API call)
        while let Some(irq) = system.next_interrupt() {
            // Inject interrupt into Unicorn
            // Set PC to vector table entry
        }
    }
}
```

---

## Critical Details

### `uc_mem_map_ptr` is the Magic

The exported `_uc_mem_map_ptr` function maps a **host memory pointer** as Unicorn memory. Since we're in a single WASM instance:

```c
// In C/Rust within same WASM module:
void* periph_mem = malloc(0x70000000);  // 0x40000000..0xB0000000
uc_mem_map_ptr(uc, 0x40000000, 0x70000000, UC_PROT_ALL, periph_mem);

// Now Unicorn reads/writes directly to that memory.
// Rust peripherals can also read/write that memory.
// No hooks needed!
```

### Memory Map Layout in WASM Linear Memory

```
  0x00000000  ┌──────────────────┐
              │   Unicorn data   │
              │   (code, stacks) │
  0x40000000  ├──────────────────┤  ← uc_mem_map_ptr target
              │ Peripheral regs  │
              │ (Rust reads here)│
  0xB0000000  ├──────────────────┤
              │   Unused/extra   │
  0xE0000000  ├──────────────────┤
              │  NVIC, SysTick,  │
              │  SCB regs        │
  0xE1000000  └──────────────────┘
```

### DMA Becomes Simple

```rust
// Current (two WASM modules):
// 1. Rust queues DmaTransfer struct
// 2. JS reads it via dma_get_pending()
// 3. JS calls uc.mem_read/mem_write
// 4. JS calls dma_set_completed()

// New (single WASM):
// Rust writes: memcpy(periph_mem + dst, periph_mem + src, size)
// No JS involved. Takes ~10ns instead of ~5µs.
```

### Interrupt Handling

```rust
// Current: JS polls has_pending_interrupt(), calls get_next_pending_interrupt(),
// then manually saves context, writes stack frame, sets PC

// New: Rust directly calls the Unicorn C API:
extern "C" {
    fn uc_emu_stop(uc: *mut c_void);  // stop current batch
    // And then inject interrupt by:
    // 1. Read current SP, PC, XPSR from Unicorn via uc_reg_read
    // 2. Write stack frame to memory via memcpy
    // 3. Write new PC via uc_reg_write
}

// Actually better: stop emulation, let host loop handle it
// (same pattern as now, but all within WASM)
```

---

## Alternative: Phase 1a (No Emscripten, Pure JavaScript Bridge Fix)

If the full Emscripten link is too complex, a simpler improvement:

### SharedArrayBuffer between two WASM instances

```javascript
// In cli.mjs:
const shared = new SharedArrayBuffer(0x8000);  // 32KB for peripheral registers

// Load Unicorn WASM with shared memory
const uc_module = await WebAssembly.instantiate(uc_wasm, {
    env: { memory: new WebAssembly.Memory({ initial: 256, maximum: 1024, shared: true }) }
});

// Load Rust WASM with SAME memory
const rust_module = await WebAssembly.instantiate(rust_wasm, {
    env: { memory: uc_module.memory }  // ← share the SAME memory
});
```

Then peripherals read/write the SAME linear memory Unicorn uses. `uc_mem_map_ptr` maps the peripheral region to a pointer within that shared memory. **Zero-copy, no JS hooks.**

This can be done on Windows right now without Emscripten.

---

## Alternative: Phase 1b (Napi-rs Native Addon)

Replace Rust WASM with a **native Node addon** compiled via napi-rs:

```bash
# In peripheral crate:
cargo build --release  # builds .node (native addon)

# In cli.mjs:
const periph = require('./periph.node');
periph.init();
// periph_read, periph_write are direct native function calls
// No WASM boundary at all for peripheral access
```

But this loses the WASM portability. Only worth it if Emscripten is too hard.

---

## Files to Create/Modify

### New:
- `build_single_wasm.sh` — Build script for Linux
- `src/wasm_exports.rs` — Raw extern "C" exports (replaces wasm-bindgen glue)
- `src/emulation_loop.rs` — New unified emulation loop (DMA + interrupts inside WASM)

### Modified:
- `Cargo.toml` — Change to `staticlib`, remove wasm-bindgen
- `src/lib.rs` — Remove #[wasm_bindgen], add #[no_mangle] extern "C"
- `pkg/cli.mjs` — Rewrite to call the single WASM module (much simpler)

### Removed:
- wasm-bindgen dependency entirely
- `pkg/stm32_bluepill_wasm_bg.js` / `.wasm` (replaced by single module)

---

## Quick Check: Can We Do a Partial Optimization Right Now?

Yes — **Phase 1a** (SharedArrayBuffer) can be done on Windows without Emscripten:

1. Extract `unicorn_arm.wasm` from `unicorn_arm.js`
2. Load it with a shared `WebAssembly.Memory`
3. Load Rust WASM with the same memory
4. Map peripheral region using `uc_mem_map_ptr` to a pointer in that shared memory
5. Remove all `memReadHook`/`memWriteHook` callbacks
6. Rust peripherals read/write directly from shared memory

This alone eliminates the JS boundary for every register access. DMA still crosses JS but that's a smaller bottleneck.

Want me to write this out as a concrete implementation plan?
