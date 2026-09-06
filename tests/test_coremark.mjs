// CoreMark CPU-semantics check: run the 200-iteration sketch and assert
// the known-answer CRCs. Cross-checks:
//   - crclist/crcmatrix/crcstate match the PUBLISHED CoreMark 1.0 values
//     (0xe714/0x1fd7/0x8e3a — iteration-independent) AND a native x86 build
//     of the same sources;
//   - crcfinal (0x382f) matches the native x86 build (it folds in the
//     iteration count, so it differs from the published 2000-iter 0x5275).
// The full 2000-iteration run (crcfinal 0x5275) was validated once headless
// during bring-up; CI runs this fast configuration (~200M instructions).
import { readFileSync } from 'fs';
import { createEmulator } from '../pkg/emulator.js';

let passed = 0, failed = 0;
function assert_eq(a, b, msg) {
  if (a === b) { passed++; }
  else { failed++; console.error(`FAIL: ${msg}: expected ${b}, got ${a}`); }
}

const elf = readFileSync(new URL('./arduino_coremark/build/arduino_coremark.ino.elf', import.meta.url));
const emu = await createEmulator({ firmware: elf });
let out = '';
for (let i = 0; i < 120 && !out.includes('CoreMark DONE'); i++) {
  const r = emu.run(5000000);
  out = emu.getUartOutput();
  if (r.stopped) break;
}
const m = (re) => { const x = out.match(re); return x ? x[1] : null; };
assert_eq(m(/\[0\]crclist\s+: 0x([0-9a-f]+)/), 'e714', 'CoreMark crclist (published 0xe714)');
assert_eq(m(/\[0\]crcmatrix\s+: 0x([0-9a-f]+)/), '1fd7', 'CoreMark crcmatrix (published 0x1fd7)');
assert_eq(m(/\[0\]crcstate\s+: 0x([0-9a-f]+)/), '8e3a', 'CoreMark crcstate (published 0x8e3a)');
assert_eq(m(/\[0\]crcfinal\s+: 0x([0-9a-f]+)/), '382f', 'CoreMark crcfinal (native x86 0x382f)');
assert_eq(out.includes('CoreMark DONE'), true, 'CoreMark ran to completion');

console.log(`Results: ${passed} passed, ${failed} failed, ${passed + failed} total`);
if (failed) process.exit(1);
