// Extended virtual-peripheral event test (the Wokwi-style queue, types 10-15):
// DAC write, CRC result, RTC alarm, watchdog reset, and CAN TX/RX.
//
// DAC/CRC/RTC all fire naturally while running periph_test. CAN TX/RX are driven
// deterministically (configure a filter that accepts everything, then inject / send)
// on the same live emulator after the firmware run.
import { readFileSync } from 'fs';
import { STM32F1 } from '../pkg/stm32f1.js';

const ELF = 'site/arduino_periph_test.elf';

const ext_devices = {
    i2c_eeprom: [
        { peripheral: 'I2C1', address: '0x50', data: new Uint8Array(256) },
        { peripheral: 'I2C2', address: '0x51', data: new Uint8Array(256) },
    ],
    i2c_oled: [{ peripheral: 'I2C1', address: '0x3C', width: 128, height: 64 }],
    spi_flash: [
        { peripheral: 'SPI1', jedec_id: '0xEF4016', data: new Uint8Array(4096), cs: 'PA4' },
        { peripheral: 'SPI2', jedec_id: '0xEF4017', data: new Uint8Array(4096), cs: 'PB12' },
    ],
    lcd: [{ peripheral: 'SPI1', cs: 'PA1' }],
    touchscreen: [{ peripheral: 'SPI1', touch_detected_pin: 'PA3', cs: 'PA2' }],
};

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const mcu = await STM32F1.fromELF(readFileSync(ELF), { ext_devices });

const counts = { dac: 0, crc: 0, rtc: 0, wdog: 0, cantx: 0, canrx: 0 };
const canrx = [], cantx = [];
mcu.onDacWrite = () => counts.dac++;
mcu.onCrcResult = () => counts.crc++;
mcu.onRtcAlarm = () => counts.rtc++;
mcu.onWdogReset = () => counts.wdog++;
mcu.onCanRx = (can, id, len, data) => { counts.canrx++; canrx.push({ can, id, len, data: data.slice() }); };
mcu.onCanTx = (can, id, len, data) => { counts.cantx++; cantx.push({ can, id, len, data: data.slice() }); };

// --- Part A: run periph_test, exercising DAC/CRC/RTC ---
const t0 = Date.now();
let done = 0;
for (let i = 0; i < 20; i++) {
    const r = mcu.execute(5_000_000);
    done += 5_000_000;
    if (counts.dac > 0 && counts.crc > 0 && counts.rtc > 0) break;
    if (r.stopped) break;
}
const elapsed = ((Date.now() - t0) / 1000).toFixed(2);

ok(counts.dac > 0, `DacWrite events fired (${counts.dac})`);
ok(counts.crc > 0, `CrcResult events fired (${counts.crc})`);
ok(counts.rtc > 0, `RtcAlarm events fired (${counts.rtc})`);

// --- Part B: deterministic CAN RX (filter accepts all, then inject) ---
// CAN1 base 0x40006400. FMR(0x200) FINIT=1, FA1R(0x21C) enable bank0,
// filter[0](0x240)=0, filter[1](0x244)=0 (mask mode -> accept all), then FINIT=0.
mcu._emu.periphWrite(0x40006400 + 0x200, 4, 1);
mcu._emu.periphWrite(0x40006400 + 0x21C, 4, 1);
mcu._emu.periphWrite(0x40006400 + 0x240, 4, 0);
mcu._emu.periphWrite(0x40006400 + 0x244, 4, 0);
mcu._emu.periphWrite(0x40006400 + 0x200, 4, 0);
mcu.execute(1000);
canrx.length = 0; // isolate our injected message
mcu._emu.canInjectMessage(0x40006400, 0 << 21, 2, 0xDEAD, 0); // STDID 0, len 2, 0xDEAD
mcu.execute(1000); // drain

ok(canrx.length > 0, `CanRx event fired (${canrx.length})`);
if (canrx.length > 0) {
    const e = canrx[0];
    ok(e.id === 0, `CanRx id STDID=0 (got ${e.id})`);
    ok(e.len === 2, `CanRx len=2 (got ${e.len})`);
    ok(e.data[0] === 0xAD && e.data[1] === 0xDE, `CanRx data=DE AD (got ${e.data.slice(0, 2).map(b => b.toString(16)).join(' ')})`);
}

// --- Part C: deterministic CAN TX (submit mailbox 0) ---
// Mailbox 0: TIR(0x180), TDTR(0x184)=len, TDLR(0x188)=data, TDHR(0x18C)=0, then TIR TXRQ.
cantx.length = 0; // isolate our transmitted message
mcu._emu.periphWrite(0x40006400 + 0x184, 4, 2);
mcu._emu.periphWrite(0x40006400 + 0x188, 4, 0xBEEF);
mcu._emu.periphWrite(0x40006400 + 0x18C, 4, 0);
mcu._emu.periphWrite(0x40006400 + 0x180, 4, (0 << 21) | 1); // STDID 0 + TXRQ
mcu.execute(1000); // drain

ok(cantx.length > 0, `CanTx event fired (${cantx.length})`);
if (cantx.length > 0) {
    const e = cantx[0];
    ok(e.id === 0, `CanTx id STDID=0 (got ${e.id})`);
    ok(e.len === 2, `CanTx len=2 (got ${e.len})`);
    ok(e.data[0] === 0xEF && e.data[1] === 0xBE, `CanTx data=BE EF (got ${e.data.slice(0, 2).map(b => b.toString(16)).join(' ')})`);
}

console.log(`more events (periph_test, ${done} instr / ${elapsed}s): DAC=${counts.dac} CRC=${counts.crc} RTC=${counts.rtc} WDOG=${counts.wdog} CANtx=${counts.cantx} CANrx=${counts.canrx}`);
console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed === 0 ? 0 : 1);
