// Firmware format tests: Intel HEX, ELF, linker map parsing (pure JS, no wasm).
import { readFileSync } from 'fs';
import { parseIntelHex, parseSymbolMap, parseElf } from '../pkg/emulator.js';

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

// --- Intel HEX vs linker map (same 5:30 PM build) ---
const hex = readFileSync('tests/comprehensive_test/build/comprehensive_test.ino.hex', 'utf8');
const mapText = readFileSync('tests/comprehensive_test/build/comprehensive_test.ino.map', 'utf8');
const p = parseIntelHex(hex);
ok(p.base === 0x08000000, 'hex base = 0x08000000');
const sp = (p.data[3] << 24) | (p.data[2] << 16) | (p.data[1] << 8) | p.data[0];
const resetVec = (p.data[7] << 24) | (p.data[6] << 16) | (p.data[5] << 8) | p.data[4];
ok(sp === 0x20005000, `hex SP matches map _estack (0x20005000, got 0x${sp.toString(16)})`);
ok(resetVec === 0x08001a79, `hex reset vector matches map Reset_Handler (0x08001a79, got 0x${resetVec.toString(16)})`);

// --- Linker map parsing ---
const syms = parseSymbolMap(mapText);
const resetSym = syms.find(s => s.name === 'Reset_Handler');
const estack = syms.find(s => s.name === '_estack');
ok(!!resetSym && resetSym.addr === 0x08001a78, `map: Reset_Handler @ 0x08001a78 (got ${resetSym && resetSym.addr.toString(16)})`);
ok(!!estack && estack.addr === 0x20005000, `map: _estack @ 0x20005000 (got ${estack && estack.addr.toString(16)})`);
ok(syms.some(s => s.name === 'main'), 'map: main found');

// --- ELF parsing ---
const elfBytes = readFileSync('tests/comprehensive_test/build/comprehensive_test.ino.elf');
const elf = parseElf(elfBytes);
const flashSeg = elf.regions.find(r => r.start === 0x08000000);
ok(!!flashSeg && flashSeg.data.length > 0, `elf: flash segment @ 0x08000000 (${flashSeg && flashSeg.data.length} bytes)`);
ok(elf.symbols.some(s => s.name === 'main'), 'elf: main symbol found');
ok(elf.symbols.some(s => s.name === 'Reset_Handler'), 'elf: Reset_Handler symbol found');
ok(elf.symbols.some(s => s.name === 'setup'), 'elf: setup symbol found');
// First vector word of ELF flash segment must equal map _estack
const elfSp = (flashSeg.data[3] << 24) | (flashSeg.data[2] << 16) | (flashSeg.data[1] << 8) | flashSeg.data[0];
ok(elfSp === 0x20005000, `elf: SP matches map (0x20005000, got 0x${elfSp.toString(16)})`);

// --- Rejects garbage ---
let threw = false;
try { parseElf(new Uint8Array([1, 2, 3, 4, 5, 6, 7, 8])); } catch (e) { threw = true; }
ok(threw, 'parseElf rejects non-ELF bytes');

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed === 0 ? 0 : 1);
