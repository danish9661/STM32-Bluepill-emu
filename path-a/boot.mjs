// Path A firmware boot: echo firmware through the merged module.
// Minimal emulator loop mirroring cli.mjs: mem hooks -> periph, emu_start
// batches + step_batch, interrupt dispatch, UART RX/TX.

import { readFileSync } from 'node:fs';
import { parseElf } from '../pkg/emulator.js';
import { loadMerged } from './loader.mjs';

const { Module, periph } = await loadMerged();
const uc = new Module.Unicorn(Module.ARCH_ARM, Module.MODE_THUMB | Module.MODE_LITTLE_ENDIAN);

const FLASH = 0x08000000, FLASH_SIZE = 0x20000, RAM = 0x20000000, RAM_SIZE = 0x5000;
uc.mem_map(BigInt(FLASH), BigInt(FLASH_SIZE), Module.PROT_ALL);
uc.mem_map(BigInt(RAM), BigInt(RAM_SIZE), Module.PROT_ALL);
uc.mem_map(0x40000000n, 0xB0000000n - 0x40000000n, Module.PROT_READ | Module.PROT_WRITE);
uc.mem_map(0xE0000000n, 0xE1000000n - 0xE0000000n, Module.PROT_READ | Module.PROT_WRITE);

const elfBuf = readFileSync(new URL('../site/arduino_echo.elf', import.meta.url));
const { regions } = parseElf(elfBuf);
let nWrote = 0;
for (const s of regions) {
    const base = Number(s.start);
    if (base >= FLASH && base < FLASH + FLASH_SIZE) {
        uc.mem_write(BigInt(base), s.data); nWrote++;
    } else if (base >= RAM && base < RAM + RAM_SIZE) {
        uc.mem_write(BigInt(base), s.data); nWrote++;
    } else {
        console.log('region at', base.toString(16), 'outside mapped regions, skipping');
    }
}
console.log('ELF regions written:', nWrote, '/', regions.length);

const read32 = (addr) => {
    const b = uc.mem_read(BigInt(addr), 4);
    return b[0] | (b[1] << 8) | (b[2] << 16) | (b[3] << 24);
};

periph.init();

const sp_init = read32(0x08000000);
const pc_init = read32(0x08000004);
uc.reg_write_i32(Module.ARM_REG_SP, sp_init);
uc.reg_write_i32(Module.ARM_REG_PC, pc_init | 1);
console.log(`SP=0x${sp_init.toString(16)} PC=0x${(pc_init | 1).toString(16)}`);

let hookReads = 0, hookWrites = 0;
let usartReads = 0, usartWrites = 0;
const hookLog = [];
for (const [start, end] of [[0x40000000, 0xB0000000], [0xE0000000, 0xE1000000]]) {
    uc.hook_add(Module.HOOK_MEM_READ, (handle, type, address, size, value) => {
        hookReads++;
        const addr32 = Number(address);
        if (addr32 >= 0x40013800 && addr32 < 0x40013900) { usartReads++; if (usartReads <= 4) hookLog.push('R ' + addr32.toString(16) + ' sz' + size + ' v' + value + ' →' + periph.periph_read(addr32, size)); }
        else if (hookLog.length < 6) hookLog.push('R ' + addr32.toString(16) + ' sz' + size + ' v' + value);
        let val;
        if (addr32 >= 0xE0001000 && addr32 < 0xE0001100) {
            val = addr32 === 0xE0001004 ? 0 : (addr32 === 0xE0001000 ? 1 : 0);
        } else {
            val = periph.periph_read(addr32, size) >>> 0;
        }
        const bytes = new Uint8Array(size);
        for (let i = 0; i < size; i++) bytes[i] = (val >> (i * 8)) & 0xFF;
        uc.mem_write(address, bytes);
    }, null, BigInt(start), BigInt(end));
    uc.hook_add(Module.HOOK_MEM_WRITE, (handle, type, address, size, value) => {
        hookWrites++;
        const addr32 = Number(address);
        if (addr32 >= 0x40013800 && addr32 < 0x40013900) { usartWrites++; if (usartWrites <= 4) hookLog.push('W ' + addr32.toString(16) + ' sz' + size + ' v' + value); }
        else if (hookLog.length < 6) hookLog.push('W ' + addr32.toString(16) + ' sz' + size + ' v' + value);
        periph.periph_write(addr32, size, Number(value));
    }, null, BigInt(start), BigInt(end));
}

// UART RX: echo firmware polls UART1; inject bytes on empty RX
let rxQueued = false;
const inject = (b) => {
    if (periph.uart_rx_pending(0x40013800) === 0) {
        periph.uart_rx_byte(0x40013800, b);
        rxQueued = true;
    }
};

const regsRead = (uc, regIds) => regIds.map((r) => uc.reg_read_i32(r));
const regsWrite = (uc, regIds, values) => regIds.forEach((r, i) => uc.reg_write_i32(r, values[i]));

const processInterrupts = () => {
    while (true) {
        const irq = periph.intr_next();
        if (irq <= -100) break;
        const regs = regsRead(uc, [Module.ARM_REG_SP, Module.ARM_REG_PC, Module.ARM_REG_LR, Module.ARM_REG_XPSR,
            Module.ARM_REG_R0, Module.ARM_REG_R1, Module.ARM_REG_R2, Module.ARM_REG_R3, Module.ARM_REG_R12]);
        const [savedAt, pc, lr, xpsr, r0, r1, r2, r3, r12] = regs;
        const frame = new Uint8Array(32);
        const sv = new DataView(frame.buffer);
        sv.setUint32(0, xpsr, true); sv.setUint32(4, pc, true); sv.setUint32(8, lr, true);
        sv.setUint32(12, r12, true); sv.setUint32(16, r3, true); sv.setUint32(20, r2, true);
        sv.setUint32(24, r1, true); sv.setUint32(28, r0, true);
        uc.mem_write(BigInt(savedAt - 32), frame);
        const handler_pc = read32(0x08000000 + 4 * (16 + irq));
        regsWrite(uc, [Module.ARM_REG_SP, Module.ARM_REG_LR, Module.ARM_REG_PC], [savedAt - 32, 0xFFFFFFF9, handler_pc]);
        try { uc.emu_start(handler_pc, 0, 0, 20000); } catch (e) {}
        periph.finish_interrupt(irq);
        const savedFrame = uc.mem_read(BigInt(savedAt - 32), 32);
        const savedSv = new DataView(savedFrame.buffer, savedFrame.byteOffset, savedFrame.byteLength);
        regsWrite(uc, [Module.ARM_REG_R0, Module.ARM_REG_R1, Module.ARM_REG_R2, Module.ARM_REG_R3,
            Module.ARM_REG_R12, Module.ARM_REG_LR, Module.ARM_REG_PC, Module.ARM_REG_XPSR, Module.ARM_REG_SP],
            [savedSv.getUint32(28, true), savedSv.getUint32(24, true), savedSv.getUint32(20, true),
             savedSv.getUint32(16, true), savedSv.getUint32(12, true), savedSv.getUint32(8, true),
             savedSv.getUint32(4, true) | 1, savedSv.getUint32(0, true), savedAt]);
    }
};

const t0 = process.hrtime.bigint();
let pc = pc_init | 1;
let out = '';
let injected = 0;
let lastInj = 0;
let ok = false;
for (let step = 0; step < 10000; step++) {
    if (injected < 2 && lastInj + 5 < step) {
        inject(injected === 0 ? 'A'.charCodeAt(0) : 'B'.charCodeAt(0));
        injected++;
        lastInj = step;
    }
    const t1 = process.hrtime.bigint();
    try {
        uc.emu_start(BigInt(pc | 1), 0n, 0n, 20000);
    } catch (e) {
        console.log('emu_start threw at step', step, 'after', Number(process.hrtime.bigint() - t1) / 1e6, 'ms');
        console.log('  type:', typeof e, 'msg:', String(e).slice(0, 300));
        console.log('emu_start error at step', step, ':', String(e).slice(0, 120));
        break;
    }
    periph.step_batch(20000);
    processInterrupts();
    out += periph.get_uart_output();
    if (out.includes('AB')) { ok = true; break; }
    if (step > 100000) { console.log('giving up at step', step); break; }
    pc = uc.reg_read_i32(Module.ARM_REG_PC);
    if (step < 3 || step % 5000 === 0) {
        console.log('step', step, 'dt', (Number(process.hrtime.bigint() - t1) / 1e6).toFixed(2) + 'ms', 'pc', pc.toString(16), 'uart len', out.length);
    }
}
console.log('loop ended: ok =', ok, 'injected =', injected, 'hookReads =', hookReads, 'hookWrites =', hookWrites, 'usartR =', usartReads, 'usartW =', usartWrites);
console.log('first hooks:', hookLog.join(' | '));
console.log('uart len =', out.length, 'hex =', Buffer.from(out).toString('hex').slice(0, 200));
const dt = Number(process.hrtime.bigint() - t0) / 1e9;
console.log('elapsed', dt.toFixed(2) + 's');
console.log('uart output:', JSON.stringify(out.slice(0, 120)));
