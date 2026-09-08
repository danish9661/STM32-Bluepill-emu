// CAN loopback chat regression test: the arduino_can_chat firmware runs
// CAN1 with LBKM (loopback) + accept-all filter, transmits incrementing
// frames and reads back its own RX FIFO, printing tx/rx id+data pairs.
//
// Guards CAN loopback delivery (TXRQ -> RX FIFO through filters) + the
// TX/RX mailbox paths. No ext devices needed.
import { readFileSync } from 'fs';
import { createEmulator } from '../pkg/emulator.js';

const ELF = 'site/arduino_can_chat.elf';
const MAX = 150000000;
const CHUNK = 10000000;

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const emu = await createEmulator({ firmware: readFileSync(ELF) });
let done = 0;
while (done < MAX) {
    const r = await emu.run(Math.min(CHUNK, MAX - done));
    done += CHUNK;
    if (r.stopped) break;
}
const out = String(emu.getUartOutput() || '');
ok(out.includes('=== CAN loopback chat demo'), 'banner printed');
const pairs = [];
for (const s of out.split('\n')) {
    const m = s.trim().match(/^tx id=([0-9A-F]+) data=([0-9A-F]+)\s+->\s+rx id=([0-9A-F]+) data=([0-9A-F]+)$/);
    if (m) pairs.push(m.slice(1));
}
ok(pairs.length >= 2, `at least 2 chat exchanges (got ${pairs.length})`);
ok(pairs.every(p => p[0] === p[2] && p[1] === p[3]), 'rx echoes tx id+data exactly');
ok(pairs[0][0] === '100' && pairs[1][0] === '101', 'frame IDs increment (0x100, 0x101, ...)');

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
