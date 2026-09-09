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
drives the same protocol with a synthetic Node client in CI (17 checks).

## How breakpoints work

Breakpoints are 16-bit `BKPT` patches. The core faults loudly on them, and
the stub tells a hit apart from a genuine decode gap by address: on a hit
it restores the original instruction, resumes *at* it, reports `SIGTRAP`,
single-steps the original on resume, and re-inserts the patch. Anything
else reports `SIGILL` and halts. Flash patching uses a probe-style raw
write path (`rustcpu_mem_write_raw`) because normal guest stores respect
flash protection.

## Fidelity notes

- Single thread (`m1`); thread queries after the first answer `l` (done).
- `s` steps one thread instruction plus any domestically-pending IRQ
  service for that batch — an interrupt may be entered as part of a step,
  like silicon.
- 17 core registers (`r0`–`r12`, `sp`, `lr`, `pc`, `xpsr`); no FPU on M3.
  Register numbers in `p`/`P` packets are **decimal** per the RSP spec.
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
