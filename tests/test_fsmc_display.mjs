// End-to-end Wokwi-style bidirectional FSMC virtual peripheral.
//
// The MCU writes LCD commands/data over FSMC BANK1 (NE1) — delivered to a JS
// virtual "FsmcLcd" via onFsmcAccess.  The virtual LCD stores its register
// state into the FSMC NOR backing image via fsmcWriteByte so the MCU can
// read it back.  This proves the full read-write cycle that real Wokwi
// virtual peripherals use: MCU writes → model updates state → MCU reads back.
import { readFileSync } from 'fs';
import { STM32F1 } from '../pkg/stm32f1.js';

const ELF = 'site/arduino_ws2812.elf';
const BANK = 'FSMC.BANK1';

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

/// 64KB backing image for FSMC BANK1.
/// Layout: [0x00]=status reg, [0x02..03]=ID reg (16-bit), [0x10]=cmd buffer.
const backing = new Uint8Array(65536);
backing[0] = 0x42; // status: ready
backing[2] = 0x55; // ID low
backing[3] = 0xAA; // ID high

/// Virtual FSMC LCD (Wokwi-style).  RS on FSMC_A0 (16-bit bus: byte addr bit 1).
class FsmcLcd {
    constructor(mcu) { this.mcu = mcu; this.command = 0; this.framebuffer = []; this.cmdHistory = []; }
    onAccess(bank, offset, write, size, value) {
        if (bank !== 1 || !write) return;
        const isData = (offset & 0x2) !== 0;
        if (isData) {
            this.framebuffer.push(value & 0xFFFF);
        } else {
            this.command = value & 0xFFFF;
            this.cmdHistory.push(this.command);
            // Virtual peripheral updates backing image so MCU reads get data back:
            if (this.command === 0x01) { this.mcu.fsmcWriteByte(BANK, 0x00, 0x00); }       // reset → clear busy
            if (this.command === 0x02) { this.mcu.fsmcWriteByte(BANK, 0x00, 0x80); }       // get status → busy
            if (this.command === 0x04) { this.mcu.fsmcWriteByte(BANK, 0x10, 0x01); }       // write pixel → mark in cmd buf
        }
    }
}

const mcu = await STM32F1.fromELF(readFileSync(ELF), {
    ext_devices: { fsmc_bank: [{ name: BANK, data: backing }] },
});
const lcd = new FsmcLcd(mcu);
mcu.onFsmcAccess = (bank, offset, write, size, value) => lcd.onAccess(bank, offset, write, size, value);

// Enable FSMC clock + BANK1 (MBKEN | WREN).
mcu._emu.periphWrite(0x4002_1014, 4, 1 << 8); // RCC AHBENR FSMCEN
mcu._emu.periphWrite(0xA000_0000, 4, 0x03);   // FSMC_BCR1
mcu.execute(1000);

const CMD = 0x6000_0000, DAT = 0x6000_0002;

// 1. MCU reads status register → should get initial value 0x42.
const s0 = mcu._emu.periphRead(CMD, 2);
ok(s0 === 0x42, `1. status before any command: 0x${s0.toString(16)} (expected 0x42)`);

// 2. MCU sends command 0x02 (get status) → virtual LCD sets busy flag in backing.
mcu._emu.periphWrite(CMD, 2, 0x0002);
mcu.execute(1000);
const s1 = mcu._emu.periphRead(CMD, 2);
ok(s1 === 0x80, `2. status after "get status" cmd: 0x${s1.toString(16)} (expected 0x80)`);

// 3. MCU sends command 0x01 (reset) → virtual LCD clears busy flag in backing.
mcu._emu.periphWrite(CMD, 2, 0x0001);
mcu.execute(1000);
const s2 = mcu._emu.periphRead(CMD, 2);
ok(s2 === 0x00, `3. status after reset cmd: 0x${s2.toString(16)} (expected 0x00)`);

// 4. MCU reads ID register → should get initial value 0xAA55.
const id = mcu._emu.periphRead(DAT, 2);
ok(id === 0xAA55, `4. ID register: 0x${id.toString(16)} (expected 0xAA55)`);

// 5. MCU sends command 0x04 (write pixel) then data.
mcu._emu.periphWrite(CMD, 2, 0x0004);
mcu._emu.periphWrite(DAT, 2, 0xBEEF);
mcu.execute(1000);
ok(lcd.cmdHistory.length === 3, `5. LCD saw 3 commands (got ${lcd.cmdHistory.length})`);
ok(lcd.framebuffer.length === 1 && lcd.framebuffer[0] === 0xBEEF,
    `5. LCD framebuffer: ${lcd.framebuffer.map(v => '0x' + v.toString(16)).join(',')}`);

// 6. Virtual LCD wrote 0x01 into cmd buffer at [0x10] for command 0x04.
// Read back through the WASM backing-image accessor (bypassing peripheral bus).
const cmdBuf = mcu.fsmcReadByte(BANK, 0x10);
ok(cmdBuf === 0x01, `6. cmd buffer [0x10]: 0x${cmdBuf.toString(16)} (expected 0x01)`);

// 7. Verify MCU can read the same value through the FSMC peripheral bus.
const cmdBufPeriph = mcu._emu.periphRead(0x6000_0010, 1);
ok(cmdBufPeriph === 0x01, `7. cmd buffer via periphRead: 0x${cmdBufPeriph.toString(16)} (expected 0x01)`);

console.log(`virtual FSMC LCD: cmd=0x${lcd.command.toString(16)} cmds=${lcd.cmdHistory.length} px=${lcd.framebuffer.length}`);
console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed === 0 ? 0 : 1);
