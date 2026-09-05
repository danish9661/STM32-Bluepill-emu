# CPU — Native Rust Cortex-M3 Interpreter (`src/cpu/`)

Pure-Rust ARMv7-M Thumb-2 interpreter (extracted from the F407 emulator tree,
ported M4→M3 on the `rust_cpu` branch) plus guest memory. One WASM module holds
the whole machine: this core calls the peripheral model directly, zero JS
crossings per instruction. License: MIT per author grant (vendored from a
GPL-3.0 tree — see [`docs/PATH_B.md`](PATH_B.md) "License note").

Provenance note: every encoding here was verified against
`arm-none-eabi-as`/`objdump` output for the repo's own firmware.

## Files

| File | Contents |
|---|---|
| `thumb.rs` (~2000 lines) | Decoder + executor: `exec16` / `exec32`, one arm per encoding |
| `mod.rs` | `Cpu` struct, `CpuFault`, MSP/PSP banking, `take_exception`, `exception_return`, `run()` loop |
| `mem.rs` | `Memory` trait + `FlatMemory` (flash + SRAM + auto-extending extra regions) |
| `regs.rs` | `Regs`: r0–r15, xpsr, primask, control, banked `msp`/`psp` |
| `native.rs` (top level) | WASM driver backend: owns the process-global `Cpu`+`FlatMemory`, `rustcpu_*` exports |

## Execution model

```
run(budget) → fetch u16 → len() → exec16 / exec32 → adv (next PC)
                                                    → fault (loud halt)
```

- **Loud faults, never silent wrongness.** Any unimplemented encoding or bad
  state records `CpuFault { pc, op1, op2, len }` and stops. A new fault tells
  you exactly which encoding a new firmware needs — add it, GAS-verify it.
- **Thumb PC semantics**: reads of R15 see `(pc+4)&!3` (`rr()`); `adv()` sets
  `r15 = pc+len|1`.
- **Branches** (`branch()`): `EXC_RETURN` values (`0xFFFFFFF0…`) route to
  `exception_return`; bit0-clear non-EXC_RETURN targets fault (no ARM state
  on Cortex-M).
- **Flags**: `nz` / `add_flags` / `sub_flags` maintain N/Z/C/V in xPSR. T1
  data-processing inside an IT block must NOT update APSR (except CMP/CMN/TST)
  — enforced by `it_suppress`, verified against Unicorn/silicon via the
  `availableForWrite` spin (getting this wrong hangs real firmware loops).

## Shift semantics (immediate vs register — read this before touching `shift_op`)

`shift_op(v, typ, amt, carry)` encodes the **immediate** rules: LSR#0 means
shift-by-32, ASR#0 means 32, ROR#0 means RRX, LSL#0 means no shift. **Register**
shifts are different: Rs==0 means **no shift, carry unchanged** for
LSR/ASR/ROR/LSL alike. Every register-shift call site (16-bit `sop 3/4/7`,
FA32 LSR/ASR/RORS arms) bypasses `shift_op` for `amt == 0`. The immediate
callers (16-bit imm with `im==0→32` pre-mapping, EA/EB shifted-register) use
it directly. Mixing these up zeroes `SystemCoreClock` and breaks
`HAL_GPIO_Init` — see `docs/PATH_B.md`.

## IT blocks, bitfields, divide, tables

- **IT predication** (`it_ok`): slot 1 uses `cond`; slot j≥2 uses `cond` iff
  mask bit (5−j) equals cond bit 0, else the inverse. Predicated-off
  instructions still consume their slot (2-byte NOP for PC purposes).
- **UBFX/SBFX, BFI/BFC, ADDW/SUBW, MOVW/MOVT**, ThumbExpandImm per the ARM ARM.
- **UDIV/SDIV** incl. divide-by-zero → 0 (shares shapes with UMLAL/SMLAL —
  check the `F:F` op2 shape first).
- **TBB/TBH**: table index is the VALUE in Rm; base is `pc+4` with NO word
  masking (a halfword-aligned table reads shifted if masked).
- **LDM/STM** (IA/DB, writeback, PC-load interworking), **STRD/LDRD**,
  **LDREX/STREX** (single-core: STREX always succeeds), CBZ/CBNZ, `PLD`/`PLI`
  as NOP, `BKPT`/`UDF`/`SETEND` fault loudly.

## Registers, banks, stacks

- `r[13]` always mirrors the CURRENT SP; `msp`/`psp` are the banks.
- `CONTROL.SPSEL` switches the current stack in hardware fashion (`MSR
  CONTROL` swaps `r13` with the other bank).
- After every **thread-mode** instruction, `run()` re-syncs the inactive bank
  from `r13` (PUSH/POP/ADD-SP write `r13` directly) — without this a later
  `mrs psp` (PendSV context switch) saves to a stale address. Handler mode
  is skipped: `take_exception`/`exception_return` manage the banks explicitly.
- `MRS`/`MSR` cover APSR, MSP, PSP, PRIMASK, CONTROL (priority-masked and
  FAULTMASK-gated forms fault; the core implements what firmware uses —
  newlib `_sbrk`'s `mrs rX, msp` works, no JS patch needed).

## Exceptions

- `take_exception(irq)`: saves IT state, banks the thread stack, pushes the
  8-word hardware frame (xPSR with T-bit, PC, LR, R12, R3–R0) onto the CURRENT
  stack (PSP when thread+PSP — this is what makes RTOS task stacks work),
  runs the handler on MSP with `LR` = `EXC_RETURN` selecting the origin stack,
  loads the handler through VTOR. Clears `sleeping` (exception entry wakes).
- `exception_return(exc)`: unstacks from the bank selected by EXC_RETURN
  (mid-handler `msr psp` task switches honored), restores APSR + IT state,
  keeps `CONTROL.SPSEL` coherent with the return stack (untouched on
  handler-to-handler returns), resumes the outer vector when nested,
  balances the NVIC active stack, drains SysTick debt.
- **Nesting**: a strictly-higher-priority IRQ preempts a running handler
  (inline delivery checks priority against the active stack, so same-priority
  re-pends never nest — depth is bounded by priority levels). Nested takes
  stack on MSP with `LR = 0xFFFFFFF1` and return to the preempted handler.
  NVIC accounting stays balanced: pops own the push for dispatched IRQs,
  synchronous takes (SVC) push explicitly via `push_active()`.
- **SVC**: with lazy dispatch it faults to the driver (`0xDF00` check), which
  steps past and takes exception −5 synchronously; with `deliver_irqs` the
  core takes it inline. No mirror (deleted with Unicorn).
- **WFI/WFE**: with delivery on, halt (`sleeping`) unless an IRQ is already
  pending; without delivery, NOP. The driver advances virtual time and wakes
  via dispatch.

## Memory (`FlatMemory`)

- Flash + SRAM byte arrays (Bluepill: 64K + 20K) plus auto-extending extra
  regions for ELF segments / images. Peripheral ranges
  (`0x40000000–0xB0000000`, `0xE0000000–0xE1000000`) route into the model with
  single width-correct calls (one guest store = exactly one model call).
- Flash is execute/read-only for the guest; `load()` bypasses the protection
  (firmware install only). Unmapped reads return 0 and record the address in
  `bad`; writes are dropped.
- Raw variants (`read8_raw`/`write8_raw`/`read16_raw`, defaulting to the
  checked forms) bypass MPU checks for firmware install, DMA (trusted bus
  master) and driver/debugger access.

## Cortex-M3 differences (vs the M4 origin)

- `dsp: false` faults SMLAXY/SMULXY/SMLAD/SMUAD/SMULW/SMLAW as UNDEFINED.
- No FPU: coprocessor encodings fault (correct — firmware uses soft float).
- CPUID comes from the model SCB (`0x410FC241`).

## Memory protection (MPU)

Full ARMv7-M MPU (`WasmSystem.mpu`, registers hosted in the SCB at
`0xE000ED90+`): 8 regions, RNR + VALID-latches-RNR + A1–A3 aliases, priority
(highest number wins), subregion disable, AP matrix (v7-M `0b111` == `0b110`
RO/RO), XN, background map (PRIVDEFENA), PPB always priv-only + XN.
- Data deny: read returns 0 / store dropped, MMFSR + MMFAR recorded, MemManage
  pended (HardFault without MEMFAULTENA). CFSR is true write-1-to-clear.
- Exec deny (XN or unreadable): `run()` halts loudly via `CpuFault` instead
  of running forbidden code.
- CPU privilege is published to the model on MSR CONTROL and exception
  transitions (FlatMemory has no CPU context); DMA uses raw access as a
  trusted master. Exception stacking bypasses checks (documented
  approximation — stacking faults would need precise-fault machinery).
- Gate firmware never enables it: zero behavior change when off. The off-state
  fast path is load-bearing for speed — ~1B gate evaluations per 200M run, and
  ANY per-access call/branch shape cost ~30% in V8 (measured 2.6s → 3.9s across
  five variants: method call, inlined check, Cell-field loads, atomic mirror,
  cold_path hints — none recovered it). What works: a plain-static `MPU_ON`
  mirror (synced on init + every MPU CTRL write) collapsing the gate to one
  `global.get` + branch with zero calls, ALL cold arms (`mpu_check_*_slow`,
  periph byte arms, exec-fault construction) outlined `#[cold] + #[inline(never)]`
  so the hot RAM/flash skeletons stay small enough for the JIT to keep inlining,
  and raw fetch (`exec_allow` = data-read predicate + XN, so an allowed fetch's
  bytes are data-readable by construction — re-gating fetch bytes would re-check
  a proven predicate twice per instruction). Result: 200M 39/39 in ~2.8s
  (~70 MIPS) vs 2.6s with gates compiled out; the residual ~5% is the honest
  cost of real enforcement.

## Driver contract (`src/native.rs`)

The WASM driver owns batching; per batch: DMA pump → `rustcpu_run(slice)` →
`step_batch` → DMA pump → `rustcpu_dispatch()` → watchdog check. Handler runs
are single-stepped (a chunked run overshoots past `bx lr` into thread code).
`rustcpu_run` returns exact executed counts (thread + handlers); decode gaps
surface via `rustcpu_fault()` → driver raises UNDEFINSTR with symbols, else
steps past. `INSTRUCTION_COUNT` advances only in `step_batch` (DWT CYCCNT and
all delta peripherals key off it).

## Tests

- `cargo test --release --lib cpu::smoke` — blinky, echo, periph39 39/39 in
  lazy AND inline delivery (the suite's 3 lazy-assuming races were hardened
  with critical sections — prompt delivery is safe), `firmware_gallery_native`
  (adc/timer/fade/flash/showcase/ws2812, live-digit assertions).
- `cargo test --release --lib cpu::core_tests` — WFI sleep/wake, PSP switch,
  nested preemption (0x123 order log), SVC-in-handler active balance, MPU
  register file + enforcement (AP matrix, priority, subregions, escalation).
- `cargo test --lib system::tests::mpu_*` — pure matching/AP/background/PPB
  unit tests (no SYS needed).
- Product paths: `node pkg/cli.mjs`, `tests/test_emulator_js.mjs`,
  `node tests/test_rustcpu.mjs` (200M 39/39), `tests/test_bus_tap.mjs`,
  headless/headed browser suites — see `docs/PATH_B.md` for numbers.
