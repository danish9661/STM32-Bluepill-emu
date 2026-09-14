// Board hardware test: NRST / BOOT0 / boardInfo all live in the WASM
// (`board_*` exports via emu.reset / setBoot0 / boardInfo), so any driver
// (page, worker, ws-server) calls the same path.
// Covers: per-chip identity (LED + user button + BOOT0/NRST + clocks),
// BOOT0 strap round-trip, NRST zeroing counters + restoring the reset
// vector, and BOOT0-high reset claiming the USART1 bootloader path.
import { readFileSync } from 'fs';
import { createEmulator } from '../pkg/emulator.js';

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };
const eq = (a, b, name) => ok(a === b, `${name} (got ${JSON.stringify(a)}, want ${JSON.stringify(b)})`);

const fw = readFileSync('site/arduino_echo.elf');

// ---- identity per chip (fresh emulator per chip: the WASM holds ONE
// process-wide model, so each createEmulator re-inits — reading boardInfo
// right after its own create keeps the assertions exact) ----
const mk = async (chip) => (await createEmulator({ firmware: fw, chip })).boardInfo();
eq((await mk('stm32f103c8')).led.name, 'PC13', 'pill LED is PC13');
{
    const bi = await mk('stm32f103c8');
    eq(bi.button, null, 'pill has no user button');
    eq(bi.boot0, true, 'pill BOOT0 present');
    eq(bi.nrst, true, 'pill NRST present');
    eq(bi.crystalHz, 8000000, 'pill 8MHz crystal');
    eq(bi.maxSysclkMhz, 72, 'pill 72MHz max (instruction-budget timing)');
}
{
    const bi = await mk('maple_mini');
    eq(bi.led.name, 'PB1', 'maple LED is PB1 (D33)');
    eq(bi.button?.name, 'BUT (PB8)', 'maple user button BUT (PB8)');
    eq(bi.button?.level, 'LOW', 'maple BUT active-low');
}
{
    const bi = await mk('nucleo_f103rb');
    eq(bi.led.name, 'PA5', 'nucleo LED is PA5 (LD2/D13)');
    eq(bi.button?.name, 'B1 (PC13)', 'nucleo user button B1 (PC13)');
    eq(bi.button?.level, 'HIGH', 'nucleo B1 active-high');
}

// One live emulator for the reset/BOOT0 behavior below.
const pill = await createEmulator({ firmware: fw, chip: 'stm32f103c8' });

// ---- BOOT0 strap round-trip ----
eq(pill.getBoot0(), false, 'BOOT0 defaults low (main flash)');
pill.setBoot0(true);
eq(pill.getBoot0(), true, 'BOOT0 straps high');

// ---- NRST: counters zeroed, back at the reset vector ----
pill.run(200000);
ok(pill.getPc() !== 0x08002769, 'firmware advanced past reset');
const tookBoot = pill.reset();
eq(tookBoot, true, 'BOOT0-high reset takes the bootloader path');
eq(pill.getPc() >>> 0, 0x08002769, 'NRST restores the reset vector');
pill.setBoot0(false);
eq(pill.reset(), false, 'BOOT0-low reset boots main flash');

// ---- BOOT0-high reset claims the USART1 bootloader responder ----
// (board_nrst enables the AN3155 path; a 0x7F sync byte must ACK.
// NOTE: the responder answers on the MODEL uart tap; the echo banner is
// drained take-on-read so the sync reply must be asserted exactly.)
pill.setBoot0(true);
pill.reset();
pill.uartRx(0x7F);
pill.step(20000);
{
    // The responder ACKs the sync FIRST (0x79), then the rebooted echo
    // firmware's banner follows on the same wire — assert the head byte.
    const wire = [...pill.getUartOutput()].map((c) => c.charCodeAt(0) & 0xFF);
    ok(wire.length >= 1 && wire[0] === 0x79, `bootloader ACKs 0x7F sync after BOOT0 reset (head=0x${(wire[0] ?? -1).toString(16)}, len=${wire.length})`);
}
pill.setBoot0(false);
pill.reset();

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
