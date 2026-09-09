// I2C slave regression test: the arduino_i2c_slave firmware is a Wire
// slave @ 0x42 (onReceive prints, onRequest replies "Hi"). The test plays
// host over i2c_inject_*: NACK on wrong address, a master-write (bytes
// land in the firmware printout), then a master-read (firmware bytes come
// back), exercising slave ADDR match, RXNE/TXE sequencing, STOPF and IRQs.
//
// NOTE: the STM32duino HAL needs one priming transaction before the first
// slave-read serves bytes (identical on silicon — HAL RAM state, not the
// model), so the read follows a write like every register-addressed read.
import { readFileSync } from 'fs';
import { STM32F1 } from '../pkg/stm32f1.js';

const ELF = 'site/arduino_i2c_slave.elf';

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const mcu = await STM32F1.fromELF(readFileSync(ELF));
const emu = mcu._emu;
const step = (n) => { for (let i = 0; i < n; i++) mcu.step(1000000); };
const out = () => String(emu.getUartOutput() || '');

step(5); // boot
ok(out().includes('slave ready'), 'slave banner printed');

// 1. Wrong address NACKs, matched address ACKs (write).
ok(emu.i2cInjectStart(1, 0x43, false) === false, 'NACK on wrong address');
ok(emu.i2cInjectStart(1, 0x42, false) === true, 'ACK on OAR1 match (write)');

// 2. Master-write: bytes with ISR-paced retries (NACK while RXNE unread).
const sent = [0x01, 0x02];
for (const b of sent) {
    let done = false;
    for (let i = 0; i < 10 && !done; i++) {
        if (emu.i2cInjectWrite(1, b)) done = true;
        else mcu.step(200000);
    }
    ok(done, `host byte 0x${b.toString(16)} accepted`);
    mcu.step(200000);
}
ok(emu.i2cInjectStop(1) === true, 'STOP accepted');
step(2);
ok(out().includes('rx=2: 1 2'), `firmware received both bytes (${JSON.stringify(out().slice(-40))})`);

// 3. Master-read (primed by the write above): firmware replies "Hi".
ok(emu.i2cInjectStart(1, 0x42, true) === true, 'ACK on OAR1 match (read)');
const got = [];
for (let i = 0; i < 8 && got.length < 2; i++) {
    mcu.step(300000);
    const b = emu.i2cInjectRead(1);
    if (b >= 0) got.push(b);
}
ok(got.join(',') === '72,105', `slave reply exact "Hi" (${got})`);
ok(emu.i2cInjectStop(1) === true, 'STOP after read');
// Bus idle again: a fresh write transaction still works.
ok(emu.i2cInjectStart(1, 0x42, false) === true, 'bus reusable after read');

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
