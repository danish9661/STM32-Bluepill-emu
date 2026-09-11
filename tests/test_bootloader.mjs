// System-memory bootloader test: full AN3155 USART session against the
// responder (autobaud, GET/version/ID, READ, GO, WRITE, ERASE, checksums),
// all headless with no CPU run — responses are synchronous on RX inject.
import { readFileSync } from 'fs';
import { createEmulator } from '../pkg/emulator.js';

const U1 = 0x40013800;
const ACK = 0x79, NAK = 0x1F;

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

const emu = await createEmulator({ firmware: readFileSync('site/arduino_echo.elf') });
const out = () => {
    const s = String(emu.getUartOutput() || '');
    return [...s].map((c) => c.charCodeAt(0) & 0xFF);
};
// Feed bytes, return everything the responder transmitted since last drain.
const send = (...bytes) => { for (const b of bytes) emu.uartRxAddr(U1, b); return out(); };
const xor = (arr) => arr.reduce((a, b) => a ^ b, 0);
// One command frame: returns the immediate ACK/NACK.
const cmd = (c) => send(c, (~c) & 0xFF);
const eq = (a, b) => JSON.stringify(a) === JSON.stringify(b);

// Disabled: bytes fall through to USART RX, no responder output.
emu.bootloaderEnable(false);
send(0x7F);
ok(out().length === 0, 'disabled: autobaud byte goes to USART RX, no reply');
ok(emu.rxPending() === 1, 'disabled: byte queued in USART RX');
emu.bootloaderEnable(true);

// Wrong autobaud byte NACKs; 0x7F handshakes.
ok(eq(send(0x41), [NAK]), 'bad autobaud byte NACKs');
ok(eq(send(0x7F), [ACK]), 'autobaud 0x7F ACKs');
// Bad command frame (broken complement) and unknown command NACK.
ok(eq(send(0x00, 0x00), [NAK]), 'bad command checksum NACKs');
ok(eq(cmd(0x77), [NAK]), 'unknown command NACKs');
// GET: ACK, N=7, version 0x22, 7 codes, ACK.
ok(eq(cmd(0x00), [ACK, 7, 0x22, 0x00, 0x01, 0x02, 0x11, 0x21, 0x31, 0x43, ACK]), 'GET list exact');
// GET_VERSION and GET_ID (STM32F103 PID 0x0410).
ok(eq(cmd(0x01), [ACK, 0x22, 0, 0, ACK]), 'GET_VERSION exact');
ok(eq(cmd(0x02), [ACK, 1, 4, 16, ACK]), 'GET_ID PID 0x0410');
// READ vector table (SP init + Reset): addr frame, then length frame.
// echo.elf is a Blue Pill (20K RAM) build: SP = 0x20005000.
const A = [0x08, 0x00, 0x00, 0x00];
ok(eq(cmd(0x11), [ACK]), 'READ command ACKs');
ok(eq(send(...A, xor(A)), [ACK]), 'READ addr ACKs');
const rd = send(7, 0xF8);
ok(rd.length === 9 && rd[0] === ACK, 'READ len ACKs + 8 bytes');
ok(JSON.stringify(rd.slice(1, 5)) === JSON.stringify([0, 0x50, 0, 0x20]), 'READ SP init 0x20005000');
ok((rd[5] & 1) === 1, 'READ Reset vector thumb bit set');
// READ outside flash/RAM NACKs at the address frame.
const BAD = [0x50, 0x00, 0x00, 0x00];
ok(eq(cmd(0x11), [ACK]), 'READ command re-ACKs');
ok(eq(send(...BAD, xor(BAD)), [NAK]), 'READ bad address NACKs');
// READ length cap: N=0xFF reads 256 bytes.
ok(eq(cmd(0x11), [ACK]), 'READ command re-ACKs');
ok(eq(send(...A, xor(A)), [ACK]), 'READ addr re-ACKs');
const L256 = send(0xFF, 0x00);
ok(L256.length === 257 && L256[0] === ACK, 'READ N=0xFF returns 256 bytes');
// GO records its address; bad address NACKs and changes nothing.
const G = [0x08, 0x00, 0x20, 0x01];
ok(eq(cmd(0x21), [ACK]), 'GO command ACKs');
ok(eq(send(...G, xor(G)), [ACK]), 'GO addr ACKs');
ok(emu.bootloaderGoAddr() === 0x08002001, `GO addr recorded (${emu.bootloaderGoAddr().toString(16)})`);
ok(eq(cmd(0x21), [ACK]), 'GO command re-ACKs');
ok(eq(send(...BAD, xor(BAD)), [NAK]), 'GO bad address NACKs');
ok(emu.bootloaderGoAddr() === 0x08002001, 'GO addr unchanged after NACK');
// WRITE 4 bytes to RAM + read back.
const W = [0x20, 0x00, 0x00, 0x00];
ok(eq(cmd(0x31), [ACK]), 'WRITE command ACKs');
ok(eq(send(...W, xor(W)), [ACK]), 'WRITE addr ACKs');
const D = [0xDE, 0xAD, 0xBE, 0xEF];
ok(eq(send(3, ...D, xor([3, ...D])), [ACK]), 'WRITE data ACKs');
ok(eq(cmd(0x11), [ACK]), 'READ-back command ACKs');
ok(eq(send(...W, xor(W)), [ACK]), 'READ-back addr ACKs');
ok(eq(send(3, 0xFC).slice(1).join(), D.join()), 'RAM write-verify round-trip');
// WRITE with broken data checksum NACKs, memory untouched.
ok(eq(cmd(0x31), [ACK]), 'WRITE command re-ACKs');
ok(eq(send(...W, xor(W)), [ACK]), 'WRITE addr re-ACKs');
ok(eq(send(3, ...D, 0x00), [NAK]), 'WRITE bad data checksum NACKs');
// WRITE to flash programs the image (ROM privilege); use the last 1KB
// page of the 64K image so the erase test below stays inside too.
const F = [0x08, 0x00, 0x3F, 0x00];
ok(eq(cmd(0x31), [ACK]), 'WRITE flash command ACKs');
ok(eq(send(...F, xor(F)), [ACK]), 'WRITE flash addr ACKs');
ok(eq(send(3, ...D, xor([3, ...D])), [ACK]), 'WRITE flash data ACKs');
ok(eq(cmd(0x11), [ACK]), 'READ flash command ACKs');
ok(eq(send(...F, xor(F)), [ACK]), 'READ flash addr ACKs');
ok(eq(send(3, 0xFC).slice(1).join(), D.join()), 'flash write-verify round-trip');
// ERASE the 1KB page holding F (0x08003F00 is in page 15), verify 0xFF.
ok(eq(cmd(0x43), [ACK]), 'ERASE command ACKs');
ok(eq(send(0x00, 15, xor([0, 15])), [ACK]), 'ERASE page ACKs');
ok(eq(cmd(0x11), [ACK]), 'READ erased command ACKs');
ok(eq(send(...F, xor(F)), [ACK]), 'READ erased addr ACKs');
ok(send(3, 0xFC).slice(1).every((b) => b === 0xFF), 'erased page reads 0xFF');

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
