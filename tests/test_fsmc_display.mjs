// End-to-end Wokwi-style virtual peripheral: an FSMC-backed LCD driven through
// the STM32F1 wrapper's onFsmcAccess callback.
//
// The MCU writes LCD commands/data over FSMC BANK1 (NE1). Each access is
// delivered to a JS virtual "FsmcLcd" peripheral via onFsmcAccess, exactly as a
// Wokwi virtual device would be: RS (command/data) is decoded from the address
// line, and the virtual display accumulates its command register + framebuffer.
// This is the same path real firmware takes (the register writes below are what
// compiled C would emit).
import { readFileSync } from 'fs';
import { STM32F1 } from '../pkg/stm32f1.js';

const ELF = 'site/arduino_ws2812.elf';

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

/// Virtual FSMC LCD (Wokwi-style). RS is on FSMC_A0 (16-bit bus => MCU byte
/// address bit 1): even offset = command, odd offset = data.
class FsmcLcd {
    constructor() {
        this.command = 0;
        this.regs = {};
        this.framebuffer = [];
    }
    onAccess(bank, offset, write, size, value) {
        if (bank !== 1 || !write) return; // BANK1, write-only display
        const isData = (offset & 0x2) !== 0;
        if (isData) {
            this.framebuffer.push(value & 0xFFFF);
        } else {
            this.command = value & 0xFFFF;
            this.regs[this.command] = (this.regs[this.command] || 0) + 1;
        }
    }
}

const lcd = new FsmcLcd();
const mcu = await STM32F1.fromELF(readFileSync(ELF));
mcu.onFsmcAccess = (bank, offset, write, size, value) => lcd.onAccess(bank, offset, write, size, value);

// Enable FSMC clock + BANK1 (MBKEN | WREN).
mcu._emu.periphWrite(0x4002_1014, 4, 1 << 8); // RCC AHBENR FSMCEN
mcu._emu.periphWrite(0xA000_0000, 4, 0x03);   // FSMC_BCR1
mcu.execute(1000);

// "Firmware" drives the LCD over FSMC (the transactions compiled C would emit):
const CMD = 0x6000_0000, DAT = 0x6000_0002;
mcu._emu.periphWrite(CMD, 2, 0x0001); // command: reset
mcu._emu.periphWrite(DAT, 2, 0x1234); // data (pixel)
mcu._emu.periphWrite(DAT, 2, 0x5678); // data (pixel)
mcu._emu.periphWrite(DAT, 2, 0x9ABC); // data (pixel)
mcu.execute(1000); // drain the queued accesses into the virtual display

ok(lcd.command === 0x0001, `LCD received reset command (got 0x${lcd.command.toString(16)})`);
ok(lcd.regs[0x0001] === 1, `reset command seen once (got ${lcd.regs[0x0001]})`);
ok(lcd.framebuffer.length === 3, `LCD received 3 data writes (got ${lcd.framebuffer.length})`);
ok(lcd.framebuffer[0] === 0x1234 && lcd.framebuffer[2] === 0x9ABC,
    `LCD framebuffer captured in order (got ${lcd.framebuffer.map(v => '0x' + v.toString(16)).join(',')})`);

console.log(`virtual FSMC LCD: cmd=0x${lcd.command.toString(16)} framebuffer=${lcd.framebuffer.length} px`);
console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed === 0 ? 0 : 1);
