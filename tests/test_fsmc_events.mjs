// FSMC memory-transaction event test (the Wokwi-style queue, type 17).
//
// Enable FSMC BANK1 (NOR, NE1 @ 0x60000000) and perform a read + write; each
// access must emit an FsmcAccess{bank, offset, write, size, value} event.
// No backing ext_device is required — the event is emitted regardless of whether
// a memory image is attached (the access simply reads 0 / writes to nothing).
import { readFileSync } from 'fs';
import { STM32F1 } from '../pkg/stm32f1.js';

const ELF = 'site/arduino_ws2812.elf'; // does not use FSMC

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const mcu = await STM32F1.fromELF(readFileSync(ELF));

let acc = [];
mcu.onFsmcAccess = (bank, offset, write, size, value) => acc.push({ bank, offset, write, size, value });

// Enable FSMC clock (RCC AHBENR bit 8) and BANK1 (MBKEN + WREN).
mcu._emu.periphWrite(0x4002_1014, 4, 1 << 8); // RCC AHBENR FSMCEN
mcu._emu.periphWrite(0xA000_0000, 4, 0x03);   // FSMC_BCR1: MBKEN | WREN
mcu.execute(1000);

// Read from BANK1 data region (0x60000000).
const r = mcu._emu.periphRead(0x6000_0000, 1);
mcu.execute(1000); // drain

// Write to BANK1 data region.
mcu._emu.periphWrite(0x6000_0000, 1, 0xAB);
mcu.execute(1000); // drain

const reads = acc.filter(a => !a.write);
const writes = acc.filter(a => a.write);
ok(reads.length > 0, `FSMC read event fired (${reads.length})`);
if (reads.length > 0) {
    ok(reads[0].bank === 1, `bank == 1 (got ${reads[0].bank})`);
    ok(reads[0].offset === 0, `offset == 0 (got ${reads[0].offset})`);
    ok(reads[0].size === 1, `size == 1 (got ${reads[0].size})`);
}
ok(writes.length > 0, `FSMC write event fired (${writes.length})`);
if (writes.length > 0) {
    ok(writes[0].bank === 1, `bank == 1 (got ${writes[0].bank})`);
    ok(writes[0].write === true, `write flag set`);
    ok(writes[0].value === 0xAB, `value == 0xAB (got 0x${writes[0].value.toString(16)})`);
}

console.log(`fsmc events: read=${reads.length} write=${writes.length} (sample read value=0x${r.toString(16)})`);
console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed === 0 ? 0 : 1);
