// Firmware format tests: Intel HEX, ELF, linker map parsing (pure JS, no wasm).
// Uses the arduino_periph_test build artifacts and cross-checks the three
// formats against each other (no hardcoded addresses — those drift when the
// sketch or core changes). The ELF is copied into the build dir by CI (from
// site/arduino_periph_test.elf); hex + map are committed.
import { readFileSync } from 'fs';
import { parseIntelHex, parseSymbolMap, parseElf } from '../pkg/emulator.js';

const BUILD = 'tests/arduino_periph_test/build';
const hexText = readFileSync(`${BUILD}/arduino_periph_test.ino.hex`, 'utf8');
const mapText = readFileSync(`${BUILD}/arduino_periph_test.ino.map`, 'utf8');
const elfBytes = readFileSync(`${BUILD}/arduino_periph_test.ino.elf`);

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

// --- Intel HEX ---
const p = parseIntelHex(hexText);
ok(p.base === 0x08000000, `hex base = 0x08000000 (got 0x${p.base.toString(16)})`);
const hexSp = (p.data[3] << 24) | (p.data[2] << 16) | (p.data[1] << 8) | p.data[0];
const hexReset = (p.data[7] << 24) | (p.data[6] << 16) | (p.data[5] << 8) | p.data[4];

// --- Linker map ---
const syms = parseSymbolMap(mapText);
const resetSym = syms.find(s => s.name === 'Reset_Handler');
const estack = syms.find(s => s.name === '_estack');
ok(!!resetSym && !!estack, 'map: Reset_Handler + _estack found');
ok(syms.some(s => s.name === 'main'), 'map: main found');
ok(syms.some(s => s.name === 'setup'), 'map: setup found');

// --- Cross-format consistency (hex vs map) ---
ok(!!estack && hexSp === estack.addr, `hex SP == map _estack (0x${hexSp.toString(16)})`);
ok(!!resetSym && hexReset === (resetSym.addr | 1), `hex reset == map Reset_Handler (0x${hexReset.toString(16)})`);

// --- ELF parsing ---
const elf = parseElf(elfBytes);
const flashSeg = elf.regions.find(r => r.start === 0x08000000);
ok(!!flashSeg && flashSeg.data.length > 0, `elf: flash segment @ 0x08000000 (${flashSeg && flashSeg.data.length} bytes)`);
ok(elf.symbols.some(s => s.name === 'main'), 'elf: main symbol found');
ok(elf.symbols.some(s => s.name === 'Reset_Handler'), 'elf: Reset_Handler symbol found');
ok(elf.symbols.some(s => s.name === 'setup'), 'elf: setup symbol found');
const elfSp = (flashSeg.data[3] << 24) | (flashSeg.data[2] << 16) | (flashSeg.data[1] << 8) | flashSeg.data[0];
ok(elfSp === hexSp, `elf: SP matches hex (0x${elfSp.toString(16)})`);
const elfReset = (flashSeg.data[7] << 24) | (flashSeg.data[6] << 16) | (flashSeg.data[5] << 8) | flashSeg.data[4];
ok(elfReset === hexReset, `elf: reset matches hex (0x${elfReset.toString(16)})`);

// --- Rejects garbage ---
let threw = false;
try { parseElf(new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8])); } catch (e) { threw = true; }
ok(threw, 'parseElf rejects non-ELF bytes');
threw = false;
const garbage = parseIntelHex('not a hex file at all');
ok(garbage.data.length === 0 && garbage.base === 0, 'parseIntelHex returns empty for garbage');

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed === 0 ? 0 : 1);
