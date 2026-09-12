# Debugging Firmware with GDB

The emulator ships a GDB Remote Serial Protocol stub (`pkg/gdbstub.mjs`,
published as `stm32f1-emu/gdb`). Any ARM-aware GDB (or script) can attach
over TCP: read registers and memory, set breakpoints, step and continue.

## Quick start

```js
import { serveGdb } from 'stm32f1-emu/gdb';
import { readFileSync } from 'fs';

const srv = await serveGdb({
  firmware: readFileSync('firmware.elf'),
  chip: 'gd32f103c8',   // any builtin variant (default stm32f103c8)
  port: 1234,
});
console.log('GDB on port', srv.port);
// ... later: srv.close();
```

```
$ arm-none-eabi-gdb -ex 'target remote :1234' firmware.elf
(gdb) break loop
Breakpoint 1 at 0x8000194
(gdb) continue
Continuing.
Breakpoint 1, loop () at ...
(gdb) info registers pc sp
(gdb) x/8xw $sp
(gdb) stepi
(gdb) detach
```

A complete scripted session lives in `tests/gdb_live_session.sh` (needs the
Arduino toolchain's `arm-none-eabi-gdb`; dev-only, not CI). `tests/test_gdbstub.mjs`
drives the same protocol with a synthetic Node client in CI (35 checks:
handshake, regs, mem, step, Z0 + Z2/Z3/Z4 with live trip replies).

## How breakpoints work

Breakpoints are 16-bit `BKPT` patches. The core faults loudly on them, and
the stub tells a hit apart from a genuine decode gap by address: on a hit
it restores the original instruction, resumes *at* it, reports `SIGTRAP`,
single-steps the original on resume, and re-inserts the patch. Anything
else reports `SIGILL` and halts. Flash patching uses a probe-style raw
write path (`rustcpu_mem_write_raw`) because normal guest stores respect
flash protection.

## How watchpoints work (Z2/Z3/Z4)

Data watchpoints map onto the model's 4 DWT-style comparator slots
(`swd_add_watchpoint`, exact byte ranges on guest data accesses — debugger
memory writes use the raw path and never trip):

- `Z2,addr,len` = write, `Z3,addr,len` = read, `Z4,addr,len` = access.
- A trip halts the core *after* the matching instruction (like HW) and the
  stub reports `T05watch:addr;` / `T05rwatch:addr;` / `T05awatch:addr;`.
- `z2`/`z3`/`z4` free the slot. Slots are scarce (4): a fifth `Z` is
  rejected with `E01`.
- `c` resumes a halted core (watch trip or DHCSR halt); `s` single-steps
  past a halt via the debug stepper, otherwise a normal one-instruction
  batch.

## The debug-port slice behind it (`src/peripherals/swd.rs`)

Transaction-level, not pin-level (GPIO here is push-pull only; clocked
SWDIO edges would cost ~1B wire events per run for zero behavioral gain):

- SWD DPv1 (`DPIDR 0x2BA01477`, CTRL/STAT with power ACKs + sticky W1C,
  SELECT, RDBUFF) + MEM-AP (CSW/TAR/DRW/BD0-3/CFG/BASE/IDR, TAR
  auto-increment, RDBUFF latch) via the `swd_dp_*` / `swd_ap_*` exports.
- Cortex debug at the real addresses (`DHCSR 0xE000EDF0` with DBGKEY +
  C_DEBUGEN/C_HALT/C_STEP, `DCRSR/DCRDR` synchronous transfers,
  `DEMCR` with TRCENA + VC_HARDERR halt-on-HardFault) — routed from the
  SCB window, so both the builtin and SVD maps answer with no new bus
  window (STIR/ACTRL/IDCODE precedent).
- Minimal JTAG TAP (`IDCODE 0x4BA00477`, BYPASS, DPACC/APACC, ABORT)
  sharing the same DP/AP file (`jtag*` exports, probe helper).
- Hot-path cost: one mirror load + branch in the run loop (`DEBUG_HALT`)
  and one in each guest data access (`WATCH_ON`); fetches bypass the
  watch gate entirely. Disarmed = predicted-not-taken.

## Fidelity notes

- Single thread (`m1`); thread queries after the first answer `l` (done).
- `s` steps one thread instruction plus any domestically-pending IRQ
  service for that batch — an interrupt may be entered as part of a step,
  like silicon.
- 17 core registers (`r0`–`r12`, `sp`, `lr`, `pc`, `xpsr`); no FPU on M3.
  Register numbers in `p`/`P` packets are **hex** per the RSP spec
  (`Pf` = PC = 15 — a real GDB 15 session caught a decimal parse here);
  `G` (write-all) is implemented, `X` (binary write) falls back to `M`.
- Breakpoint addresses are masked to halfwords (Thumb); `z0` removes.
- `Ctrl-C` (0x03) halts a running continue with `SIGINT`.
- The stub intentionally does *not* advertise `qXfer:features:read`:
  GDB 15's XML parser rejects minimal docs while its default ARM layout
  matches the 17-register `g` packet exactly. Explicit `qXfer` requests
  are still served.
- Symbols stay in GDB (load the ELF there); the stub runs symbol-free so
  faults surface instead of escalating.

## Board bring-up cheat sheet

New to a board? The fastest loop, per board:

1. Pick the chip in the page selector (or `chip:` opt) — Blue Pill default,
   `maple_mini`, `nucleo_f103rb`, `stm32f103rc`, `gd32f103c8/cb/rb`.
2. Read back `DBGMCU_IDCODE @ 0xE0042000` (F103 `0x10016410`, GD32
   `0x2BA01477`) — one register read proves the variant is live. The page
   stats bar shows it after every load.
3. Check the GPIO grid aliases (`site/board_pins.json`): Nucleo LED is
   `D13`/`PA5`, Maple LED is `D33`/`PB1`, Pill LED is `D17`/`PC13`.
4. Watch the UART: Nucleo/RC `Serial` is USART2 (page `uart_addr` / test
   `uart_addr` opt selects it); Maple/Pill use USART1.
5. DFU-layout (bootloader-offset) firmware boots via `vector_table`
   (e.g. `0x08005000` on 128K parts) — proven in `tests/test_chips.mjs`.
