# Cortex-M3 CPU core (ported from the STM32F407 emulator)

A pure-Rust ARMv7-M Thumb-2 interpreter, extracted verbatim from
`stm32-periph-wasm/src/cpu/` at commit `3df073e` (`feature/wasm-cpu`).
It was built and battle-tested as a **Cortex-M4** core (boots FreeRTOS,
ethernet firmware, and DOOM to the title screen), and this folder is the
starting point for a **Cortex-M3** port (e.g. Blue Pill / STM32F103).

The `cpu/` files are byte-identical copies (`diff -r` clean). They do
**not** compile standalone — `mod.rs` uses `crate::system::WasmSystem`
(see §4, seam 1). Either vendor them into a crate that provides it, or do
the small decoupling described in §4.

## 1. File map

| File | Lines | Contents |
|---|---|---|
| `cpu/thumb.rs` | ~1950 | The decoder + executor. `exec16` / `exec32`, one arm per encoding, GAS-verified against `arm-none-eabi-as`/`objdump` output. |
| `cpu/mod.rs` | ~340 | `Cpu` struct, `CpuFault`, MSP/PSP banking (`read/write_msp/psp`), `take_exception`, `exception_return`, `run()` loop with inline IRQ delivery. |
| `cpu/mem.rs` | ~190 | `Memory` trait + `FlatMemory` (flash + SRAM + auto-extending extra regions). Peripheral addresses route to the model. |
| `cpu/regs.rs` | ~25 | `Regs`: r0–r15, xpsr, primask, control, banked `msp`/`psp`. |
| `cpu/tests.rs` | ~500 | Native tests (boot + peripheral + ISA unit tests). Needs firmware `.bin`s; see §5. |

## 2. How it works

```
run(budget) → fetch u16 → len() → exec16 / exec32 → adv (next PC)
                                                    → fault (loud halt)
```

- **Loud faults, never silent wrongness.** Any unimplemented encoding or
  bad state records `CpuFault { pc, op1, op2, len }` and stops. When
  porting, a new fault tells you exactly which encoding the new firmware
  needs — add it, GAS-verify it, move on.
- **Memory hooks.** `mem.rs` routes peripheral addresses into single
  width-correct model calls (one guest store = exactly one model call —
  a byte-split RMW once emitted 4 UART chars per store).
- **IT blocks** (`it_ok`, `thumb.rs:149`): slot `j>=2` uses `cond` iff
  mask bit `(5-j)` equals cond bit 0. Predicated **T1 MOVS-imm preserves
  flags** (matches Unicorn/observed silicon behavior; bare `movs` still
  sets N/Z) — do not "simplify" this, the DOOM title hangs without it.
- **Branches** (`branch`, `thumb.rs:66`): `EXC_RETURN` values go through
  `exception_return`; bit0-clear non-EXC_RETURN faults (no ARM state).
- **Exceptions** (`mod.rs:141/205`): hardware-shaped stacking onto the
  current stack, handler on MSP, `EXC_RETURN` selects the return stack,
  `CONTROL.SPSEL` kept coherent, `r13` and the banks re-synced after every
  thread-mode instruction. Sleeping: `WFI`/`WFE` set `sleeping` when
  delivery is on; the driver advances virtual time and wakes on pending IRQ.
- **Vendor rebuild note:** the `.wasm`/bindings this core ships in were
  built with `wasm-pack build --release --target <nodejs|web>`; the `nodejs`
  target matters (default bundler emits ESM that `require()` can't load).

## 3. M4 → M3 port checklist

M3 and M4 share all of Thumb-2/IT/CBZ/UDIV/SDIV/CLZ/RBIT/REV/USAT/SSAT/
UBFX/SBFX/TBB/TBH/WFI/MSR-MRS. The deltas:

- [ ] **Gate the DSP extension to `fault()`** (`thumb.rs`, FB bucket arms
      `op 1/2/3`: `SMLAXY/SMULXY` ~line 1598, `SMLAD/SMUAD` ~1622,
      `SMULW/SMLAW` ~1643). These encodings are UNDEFINED on M3. Everything
      else DSP (QADD, UADD8 family, USAD8, PKH, SXTAB, SMLSD/SMLALD,
      SMMLA/S) was never implemented and already faults — correct for M3,
      leave it. Suggested: `const DSP: bool` on `Cpu`, default `true` (M4)
      so the existing suite stays green; M3 sets `false`.
- [ ] **Keep as-is (M3-legal, already verified):** `USAT`/`SSAT`
      (`thumb.rs:921/935`, note SSAT's sat field encodes N-1),
      `UDIV`/`SDIV` (+ the F:F shape checks), `ADDW`/`SUBW` (plain imm12),
      T3 register-offset (`(o2 & 0xC00) == 0` gate), `TBB`/`TBH` (index by
      Rm *value*, unmasked `pc+4` base).
- [ ] **FPU: nothing to do.** Coprocessor/FPU encodings fault loudly —
      correct for M3 (no FPU) and for M4-with-FPU-disabled.
- [ ] **MPU: nothing to do.** The core does no MPU checks (fine for the
      typical Blue Pill build with MPU off).
- [ ] **Swap the peripheral model (biggest job, outside `cpu/`):**
      F1 memory map (RCC/GPIO/USART/AFIO differ from F4), F1 NVIC IRQ
      numbers (fewer IRQs; the pump uses numbers, so update the table),
      SysTick (same programming model), 72 MHz PLL (the model clock is
      instruction-count driven — set the counts), USB device instead of
      Ethernet (Blue Pill has no ETH; it does have bxCAN).
- [ ] **Memory sizes:** `FlatMemory::new(flash, ram)` + `map_extra`
      (`mem.rs:34/73/80`) already parameterize this — e.g. 64K/20K for the
      F103C8. SRAM is zeroed at map; keep that (the DOOM port once booted
      on garbage stack because a region wasn't zeroed).

## 4. The two seams to wire (how to use it)

**Seam 1 — `WasmSystem`** (`mod.rs:8`, used at `mod.rs:141` (`take_exception`
reads VTOR + sets NVIC in-interrupt), `mod.rs:205+` (`exception_return`
clears it), `mod.rs:280` (`run()` checks `has_pending()` /
`get_and_clear_next_intr_pending()`)). Options: (a) point it at your F1
model exposing the same three methods; (b) replace the parameter with your
own `Bus` trait (`read_vtor()`, `nvic_poll()`, `nvic_clear()`,
`set_in_interrupt()`).

**Seam 2 — `Memory`.** `FlatMemory` (`mem.rs:34`) is reusable as-is
(includes flash write-protection + `load()` that bypasses it for firmware
imaging). Or implement `Memory` (`read8/16/32`, `write8/16/32`) over your
own map; peripheral accesses must stay single width-correct calls.

**Minimal boot** (adapted from `boot_doom()` in `tests.rs`):

```rust
let mut mem = FlatMemory::new(64 * 1024, 20 * 1024); // F103C8
mem.load(&firmware_bin, 0x08000000);                 // writes through flash prot
let sp = u32::from_le_bytes(firmware_bin[0..4].try_into().unwrap());
let pc = u32::from_le_bytes(firmware_bin[4..8].try_into().unwrap());
let mut cpu = Cpu::new(sp, pc | 1);                  // note: Thumb bit!
loop {
    let done = cpu.run(&sys, &mut mem, 100_000);
    drain_uart();
    if cpu.fault.is_some() { break; }                // CpuFault has pc/op
}
```

`run()` returns instructions executed; `deliver_irqs` (default false)
selects polling vs interrupt/Exception delivery (`SVC` faults loudly when
off — intentional, polling firmware never SVCs). Set it for FreeRTOS/RTC
firmware.

## 5. Validation plan (what's left to prove on M3)

Port these first — they need no firmware changes, only an F103 `blinky.bin`
(or keep the F4 one for ISA-only checks, since most never touch peripherals):

- ISA units (pure decoder, highest value): `tbb_index_by_value`,
  `sdiv_plain_and_it`, `usat_ssat_q`, `addw_subw_plain_imm`,
  `t3_reg_no_writeback`, `cmp_flags_sanity` (+ the `ite`/`bcond` shapes).
- Boot tests: adapt `boot()` (`tests.rs:20`) to your vector table +
  `no_fault()` (`tests.rs:39`) after every batch — the single most useful
  harness in this tree.
- Then: an F103 blinky (GPIO ODR round-trip), USART echo, SysTick/RTC,
  and only then interrupts/FreeRTOS (`exception_svc_roundtrip`,
  `freertos_tasks_run` are the templates).
- Known-good reference for every failure: the stock test
  (`test_doom_wasm.mjs`-style) plus `arm-none-eabi-objdump -d` of the exact
  faulting PC — every bug in §3's history was found that way (fault pc/op
  → objdump → GAS probe → fix → unit test).
