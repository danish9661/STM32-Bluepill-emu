// Spec-vector suite: independent cross-checks that don't trust the model.
// 1. CRC-32/MPEG-2 known answers (python oracle validated against the
//    published check value 0x0376E6E7 for "123456789").
// 2. CAN filter differential: a from-scratch JS reference matcher
//    (RM0008 mask/list semantics) vs the model over seeded-random
//    filter configs + IDs, incl. FIFO assignment. Guards the shared-bank
//    remodel: any divergence is a real regression.
import { readFileSync } from 'fs';
import * as periph from '../pkg/stm32_bluepill_wasm.js';
periph.initSync({ module: readFileSync(new URL('../pkg/stm32_bluepill_wasm_bg.wasm', import.meta.url)) });

const { init, periph_read, periph_write, can_inject_message } = periph;

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

// ---------- CRC-32/MPEG-2 (poly 0x04C11DB7, init 0xFFFFFFFF, no refin) ----------
{
    const CRC = 0x40023000;
    init();
    periph_write(CRC + 0x08, 4, 1); // reset
    periph_write(CRC + 0x00, 4, 0x31323334); // '1234'
    periph_write(CRC + 0x00, 4, 0x35363738); // '5678'
    ok(periph_read(CRC + 0x00, 4) >>> 0 === 0x49E3C2FB, 'CRC-32/MPEG-2("12345678") = 0x49E3C2FB');
    periph_write(CRC + 0x08, 4, 1); // reset
    periph_write(CRC + 0x00, 4, 0xDEADBEEF);
    ok(periph_read(CRC + 0x00, 4) >>> 0 === 0x81DA1A18, 'CRC single word 0xDEADBEEF = 0x81DA1A18');
    // Naive bit-by-bit reimplementation (independent code shape) over words.
    const ref = (words) => {
        let c = 0xFFFFFFFF;
        for (const w of words) {
            c ^= w >>> 0;
            for (let i = 0; i < 32; i++) c = (c & 0x80000000) ? (((c << 1) ^ 0x04C11DB7) >>> 0) : ((c << 1) >>> 0);
        }
        return c >>> 0;
    };
    const words = [0x12345678, 0x9ABCDEF0, 0x0F1E2D3C, 0xFFFFFFFF, 0x00000000, 0x80000001];
    init();
    periph_write(CRC + 0x08, 4, 1);
    for (const w of words) periph_write(CRC + 0x00, 4, w);
    ok(periph_read(CRC + 0x00, 4) >>> 0 === ref(words), 'CRC matches naive reimplementation (6 words)');
}

// ---------- CAN filter differential vs JS reference ----------
{
    const CAN1 = 0x40006400;
    // mulberry32: deterministic across runs.
    let st = 0xC0FFEE;
    const rnd = () => {
        st |= 0; st = (st + 0x6D2B79F5) | 0;
        let t = Math.imul(st ^ (st >>> 15), 1 | st);
        t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
        return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
    const ri = (n) => Math.floor(rnd() * n);
    // Reference matcher: RM0008 semantics over banks [lo, hi).
    const refMatch = (regs, tir, lo, hi) => {
        let best = null;
        const identifier = tir >>> 21;
        for (let bank = lo; bank < hi; bank++) {
            if (!((regs.fa1r >> bank) & 1)) continue;
            const scale = (regs.fm1r >> bank) & 1;
            const mode = (regs.fs1r >> bank) & 1;
            const f0 = regs.f[bank * 2] >>> 0, f1 = regs.f[bank * 2 + 1] >>> 0;
            let hit = false;
            if (scale === 0 && mode === 0) {
                const id = identifier & 0x7FF;
                hit = ((id & (f0 & 0xFFFF)) === ((f0 >>> 16) & (f0 & 0xFFFF))) ||
                      ((id & (f1 & 0xFFFF)) === ((f1 >>> 16) & (f1 & 0xFFFF)));
            } else if (scale === 0 && mode === 1) {
                const id = identifier & 0x7FF;
                hit = id === (f0 >>> 16) || id === (f0 & 0xFFFF) || id === (f1 >>> 16) || id === (f1 & 0xFFFF);
            } else if (scale === 1 && mode === 0) {
                hit = ((identifier & f1) >>> 0) === ((f0 & f1) >>> 0);
            } else {
                hit = identifier === f0;
            }
            if (hit) best = (regs.ffa1r >> bank) & 1;
        }
        return best;
    };
    // Hand-computed anchors first.
    init();
    const W = (o, v) => periph_write(CAN1 + o, 4, v);
    W(0x200, 1);
    W(0x204, 1 << 2); // bank 2: 32-bit
    W(0x20C, 0xFFFFFFFF & ~(1 << 2)); // bank 2: mask mode (rest: list)
    W(0x240 + 2 * 8, 0); W(0x240 + 2 * 8 + 4, 0); // bank2 id=0 mask=0: accept all
    W(0x21C, 1 << 2);
    W(0x200, 14 << 8);
    ok(can_inject_message(CAN1, ((0x7FF << 21) | 1) >>> 0, 8, 0, 0) === true, 'hand: 32-bit mask accept-all hits 0x7FF');
    ok(can_inject_message(CAN1, ((0x123 << 21) | 1) >>> 0, 8, 0, 0) === true, 'hand: 32-bit mask accept-all hits 0x123');
    // Randomized differential, banks 0..3, both scales/modes, FIFO mix.
    let divs = 0;
    for (let t = 0; t < 150; t++) {
        init();
        const regs = { fa1r: 0, fm1r: 0, fs1r: 0, ffa1r: 0, f: new Array(8).fill(0) };
        W(0x200, 1);
        for (let b = 0; b < 4; b++) {
            if (rnd() < 0.6) regs.fa1r |= 1 << b;
            if (rnd() < 0.5) regs.fm1r |= 1 << b;
            if (rnd() < 0.5) regs.fs1r |= 1 << b;
            if (rnd() < 0.3) regs.ffa1r |= 1 << b;
            regs.f[b * 2] = ri(0x100000000);
            regs.f[b * 2 + 1] = ri(0x100000000);
        }
        W(0x204, regs.fm1r); W(0x20C, regs.fs1r); W(0x214, regs.ffa1r);
        for (let b = 0; b < 4; b++) { W(0x240 + b * 8, regs.f[b * 2]); W(0x240 + b * 8 + 4, regs.f[b * 2 + 1]); }
        W(0x21C, regs.fa1r);
        W(0x200, 14 << 8);
        const id = ri(0x800);
        const tir = ((id << 21) | 1) >>> 0;
        const want = refMatch(regs, tir, 0, 14);
        const gotAcc = can_inject_message(CAN1, tir, 8, 0x11223344, 0);
        const r0 = periph_read(CAN1 + 0x0C, 4) & 0x3;
        const r1 = periph_read(CAN1 + 0x10, 4) & 0x3;
        const gotFifo = gotAcc ? (r0 === 1 ? 0 : r1 === 1 ? 1 : -1) : null;
        if ((want === null) !== (!gotAcc) || (want !== null && want !== gotFifo)) {
            divs++;
            if (divs <= 3) console.log(`  div seed-case t=${t} id=${id.toString(16)} want=${want} got=${gotFifo}`);
        }
    }
    ok(divs === 0, `CAN filter differential 150/150 agree (divs=${divs})`);
}

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
