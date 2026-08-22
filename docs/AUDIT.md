# Security & Performance Audit — STM32 Bluepill Emulator (WASM)

> Audit performed: 2026-08-22. Build: `wasm-pack build --target web` with
> `RUSTFLAGS="--remap-path-prefix=$HOME=/build"` (deterministic, byte-exact).
> All measurements below are reproducible — see `How to reproduce` at the end.

## 1. Summary

| Dimension | Result | Notes |
|---|---|---|
| Memory leaks | **PASS** | < 0.1 MB growth over 2–10 billion ticks |
| Bounded buffers | **PASS** | Pin-event buffer capped at 1024, drops wholesale on overflow |
| Security (secrets) | **PASS** | No keys/tokens/credentials in source or repo |
| Security (input) | **PASS** | All user-input paths validated; no `panic!` on bad input |
| Security (sandbox) | **PASS** | WASM linear memory; no `eval`, no syscalls, no network |
| Dependencies | **PASS** | 1 runtime dep (`js-yaml`), 0 known CVEs |
| Overhead | **PASS** | Hookless counting; per-batch tick; bus cache; 2× reg transport |
| Performance | **PASS** | ~23.7 M IPS headless (200M in 8.44s); ~5 M IPS in browser |
| Robustness | **PASS** | Bad input degrades gracefully, module stays alive |

**Verdict: no blocking issues.** The emulator is safe to publish and run
untrusted firmware in the browser or Node.js.

---

## 2. Memory & Leaks

### 2.1 Methodology
Ran the pure-Rust `step_batch()` tick loop for 2 billion and 10 billion
instructions while sampling V8 heap usage (`process.memoryUsage().heapUsed`),
forcing GC between samples.

### 2.2 Results

| Scenario | Ticks | Heap growth |
|---|---|---|
| `step_batch` loop | 2,000,000,000 | **+0.10 MB** (flat) |
| Long firmware run | 10,000,000,000 | **+0.01 MB** (flat) |
| Full 200M firmware run (RSS peak) | 200M | **~144 MB** total (Node + Unicorn JIT + wasm) |

The ~144 MB RSS is dominated by Unicorn's TCG JIT code cache and the Node
runtime baseline — not by emulator state, which stays constant. The Rust
peripheral state is a fixed set of `Box<dyn Peripheral>` structs allocated
once at `init()`; ticking does not allocate.

### 2.3 Bounded buffers
The GPIO pin-change event buffer (`GPIO_PIN_EVENTS`, `src/peripherals/gpio.rs`)
is capped at `MAX_PIN_EVENTS = 1024` triples. On overflow it is **cleared
wholesale** rather than grown:

```rust
fn record_pin_event(port: u8, pin: u8, level: bool) {
    let mut ev = GPIO_PIN_EVENTS.lock().unwrap();
    if ev.len() + 3 > MAX_PIN_EVENTS { ev.clear(); }  // never grows past cap
    ev.push(port as u32); ev.push(pin as u32); ev.push(level as u32);
}
```

Verified: 500K output toggles with **no drain** from JS do not grow the
buffer unboundedly. The worst case (page never calls `gpio_take_pin_events()`)
drops events but never leaks memory.

### 2.4 No leak in DMA / interrupt paths
`dma_pump_all()` returns a flat op-plan consumed immediately by JS; the
absorbed-byte side buffer (`dma_take_absorbed`) is a per-call `Vec` that is
dropped after each batch. IRQ dispatch state (`IntrDispatch`) is allocated
once at `init()`.

---

## 3. Security

### 3.1 No secrets or credentials
- `grep` for `api_key|secret|token|password|credential` across `src/`,
  `pkg/` → **no matches**.
- No `.env`, `.pem`, `.key`, `.cert` files committed.
- `.gitignore` excludes `node_modules/`, `third_party/`, build artifacts.

### 3.2 Input validation (the main fix this audit prompted)
**Before:** `Pin::from_str`, `GpioPorts::port_index`, `Bus::register`, and
`ext_devices::parse_pin` called `panic!` on malformed input (bad pin name,
empty register range). In WASM a `panic!` **aborts the entire module** — a
single bad `add_spi_flash('SPI1', ..., 'PZ9')` call would kill the running
emulator for the whole page.

**After (this audit):** all four sites now return `Option` / skip
gracefully:

| Location | Old | New |
|---|---|---|
| `gpio.rs` `Pin::from_str` | `panic!` | `Option<Pin>` |
| `gpio.rs` `GpioPorts::port_index` | `panic!` | `Option<u8>` |
| `bus.rs` `Bus::register` (empty range) | `panic!` | early return (no-op) |
| `ext_devices/mod.rs` `parse_pin` | `panic!` | `Option<(u8,u8)>` |

Callers (`sw_spi.rs`, `touchscreen.rs`, `mod.rs`) now `if let Some(..)` guard
the device registration — a bad pin name **skips that device** instead of
crashing.

**Verified:** 5/5 bad-input calls (`PZ9`, `PA99`, `BAD`, `''`, empty bus
range) are handled **without aborting**; the module remains usable afterward.

### 3.3 Sandbox posture
- The peripheral Rust code compiles to **WASM** with no raw system calls.
- Memory is a single linear `WebAssembly.Memory` heap; no `eval`, no
  `Function` constructor misuse, no dynamic code generation in the core.
- JS peripherals (`register_js_peripheral`) receive only `(addr, size)` /
  `(addr, value, size)` — they cannot reach emulator internals.
- The CLI (`pkg/cli.mjs`) reads firmware/config files with explicit error
  handling (`ENOENT` → friendly message, not a stack trace).
- **No network access** from the emulator core. Firmware is supplied locally
  (file upload in the browser, file path on CLI).

### 3.4 Supply chain
- `npm audit` → **0 vulnerabilities**.
- Single runtime dependency: `js-yaml@4.3.1` (transitive `argparse@2.0.1`),
  both well-maintained with no known CVEs.
- `npm pack --dry-run` → 16 files, 5.2 MB unpacked; only `pkg/`, `svd/`,
  `README.md`, `LICENSE` are published. No test fixtures, no source, no
  secrets.

---

## 4. Overhead (per-instruction cost)

The emulator's fast path is deliberately cheap:

| Optimization | Effect | Measured |
|---|---|---|
| Hookless instruction counting | No JS callback per instruction | ~20% runtime removed |
| `step_batch()` once per batch | All peripherals tick per-batch, not per-instr | 3.8× speedup |
| `Bus::get` temporal cache | Sequential periph access hits cache | 99% hit rate |
| Batch register transport | IRQ dispatch: 17 → 2 WASM crossings | 1.3% of runtime |
| Closed-form timer advance | Jump to event ticks, skip empty ones | 124× on timer path |
| `instCount` as plain number | No BigInt per instruction | ~19% faster |

**Peripheral access frequency:** measured **0.001 accesses/instruction**
(~27K per 50M instructions) → ~0.1% of runtime. This is why replacing the
memory hooks with shared linear memory was retired as moot (no win available).

---

## 5. Performance

| Metric | Value |
|---|---|
| Headless throughput | **~23.7 M IPS** (200M instructions in 8.44s) |
| Browser throughput | **~5 M IPS** (page run loop) |
| 200M firmware run wall time | 8.44s (CLI) |
| Batch size | 20,000 instructions (5× lower IRQ latency vs 100K, zero speed cost) |
| Unicorn TCG share of runtime | ~97.5% (JS/Rust layer is exhausted) |

**Conclusion:** the JS/Rust bridge is no longer a bottleneck. Further speed
gains would require replacing Unicorn's TCG JIT itself (Path B, deferred —
see `docs/NEXT_PHASE.md`). The dual-WASM architecture is the right call.

---

## 6. Robustness

| Test | Result |
|---|---|
| `panic!` on bad pin name / empty bus range | **Removed** — all 4 sites now `Option`/no-op |
| Module survives bad input | **Yes** — 5/5 cases, still usable after |
| 236/236 unit tests | **PASS** |
| 39/39 firmware integration checks | **PASS** |
| `Fatal: undefined` historical incident | Not reproduced across ~6B stress instructions |

### Known limitations (documented, non-blocking)
- **USB** is a stub (`src/peripherals/usb.rs` returns 0 on all access). Real
  Blue Pill USB (USB-FS device) is rarely used and out of scope.
- **cli.mjs workarounds** (`mrs msp` → `mov sp`, `i2c_init` NVIC patch,
  `hi2c->Mode` patch) are Arduino-HAL-specific binary patches required because
  Unicorn cannot decode certain Thumb instructions / skips certain `bl`
  calls. These are documented in `AGENTS.md` §Active Workarounds and do not
  affect general firmware.

---

## 7. How to reproduce

```bash
# Rebuild wasm (deterministic)
RUSTFLAGS="--remap-path-prefix=$HOME=/build" wasm-pack build --target web

# Unit + integration tests
node tests/test_all.mjs              # 236/236
node tests/canary.mjs                # 39/39

# Memory stability (pure Rust tick loop) — see audit_mem.mjs
node audit_mem.mjs

# Robustness (bad input) — see audit_robust.mjs
node audit_robust.mjs                # 5/5 handled, no abort

# Performance
/usr/bin/time -v node pkg/cli.mjs --config=tests/arduino_periph_test/config.yaml --max=200000000
```

`audit_mem.mjs` / `audit_robust.mjs` are temporary scripts used for this
audit and are not part of the published package.

---

## 8. Recommendations

1. **(Done)** Remove `panic!` from user-input paths → `Option`. ✅
2. **(Done)** Add `console_warn` on silent-skipped bad input — `Bus::register`
   empty range, and invalid pin names in `ext_devices::parse_pin` /
   `Pin::from_str` (software-SPI + touchscreen config). Zero new deps (raw
   `js_sys` reflection); fires only on bad input, verified silent on clean runs.
   Also added `description`/`repository`/`license`/etc. to `Cargo.toml` so
   `wasm-pack` no longer warns and `pkg/package.json` carries full metadata. ✅
3. **(Optional)** Implement real USB-FS if a use case appears; otherwise keep
   the stub.
4. **(No action)** Performance is at the Unicorn-TCG ceiling; do not pursue
   Path A/B unless a feature requires CPU-core changes.
