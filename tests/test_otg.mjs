// USB OTG FS device (STM32F105 map) regression tests: core registers,
// reset/flush, bus reset + enumeration events, SETUP/OUT/IN transfers with
// GRXSTSP/FIFO/DAINT/IRQ semantics, STALL, detach, PWRDWN, DAD filtering,
// host-range inertness. Runs on the F105 SVD map (the F103 map has no OTG).
import { readFileSync } from 'fs';
import * as periph from '../pkg/stm32_bluepill_wasm.js';
periph.initSync({ module: readFileSync(new URL('../pkg/stm32_bluepill_wasm_bg.wasm', import.meta.url)) });

const { init_svd, periph_read, periph_write, step_batch, has_pending_interrupt,
        get_next_pending_interrupt, clear_current_interrupt,
        drain_events, otg_inject_setup, otg_inject_out, otg_bus_reset, otg_detach } = periph;

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };
const eq = (a, b, name) => { if (a === b) { passed++; } else { failed++; console.log(`FAIL: ${name}: expected 0x${b.toString(16)}, got 0x${a.toString(16)}`); } };

const svd = readFileSync(new URL('../svd/STM32F105xx.svd', import.meta.url), 'utf8');
init_svd(svd);

const OTG = 0x50000000;
const R = (off, w = 4) => periph_read(OTG + off, w) >>> 0;
const W = (off, v, w = 4) => periph_write(OTG + off, w, v);
// GINTSTS bits.
const G_USBRST = 1 << 12, G_ENUMDNE = 1 << 13, G_RXFLVL = 1 << 4;
const G_IEPINT = 1 << 18, G_OEPINT = 1 << 19, G_SOF = 1 << 3;
const G_USBSUSP = 1 << 11, G_WKUP = 1 << 31, G_NPTXFE = 1 << 5, G_MMIS = 1 << 1;
// Endpoint bit helpers.
const EPENA = 0x80000000, EPDIS = 0x40000000, SNAK = 0x08000000, CNAK = 0x04000000;
const STALL = 0x00200000, NAKSTS = 0x00020000, USBAEP = 0x8000;
const XFRC = 1, EPDISD = 2, STUP = 8;
// ISER2 covers IRQ 64-95: OTG_FS_IRQn = 67 -> bit 3.
periph_write(0xE000E108, 4, 1 << 3);

// 1. Reset values (SVD-sourced).
eq(R(0x03C), 0x1000, 'CID reads 0x1000');
eq(R(0x000), 0x800, 'GOTGCTL reset 0x800');
eq(R(0x00C), 0xA00, 'GUSBCFG reset 0xA00');
eq(R(0x024), 0x200, 'GRXFSIZ reset 0x200');
eq(R(0x800), 0x02200000, 'DCFG reset 0x02200000');
eq(R(0x010) >>> 31, 1, 'GRSTCTL AHBIDL reads 1');
eq(R(0x014) & G_NPTXFE, G_NPTXFE, 'GINTSTS NPTXFE set (FIFO space)');
eq(R(0x918), 0x200, 'DTXFSTS0 generous space');
eq(R(0x02C), 0x200, 'GNPTXSTS space available');
eq(R(0x818), 0, 'DAINT reset 0');

// 2. GRSTCTL: CSFTRST self-clears and restores defaults.
W(0x800, 0x50 | (1 << 11)); // DCFG DAD=5 (plus junk high bits)
W(0x010, 1); // CSFTRST
eq(R(0x010) & 1, 0, 'GRSTCTL CSFTRST self-clears');
eq(R(0x800), 0x02200000, 'CSFTRST restores DCFG default');
// RXFFLSH/TXFFLSH self-clear.
W(0x010, (1 << 4) | (1 << 5) | (2 << 6)); // RX flush + TX flush fifo 2
eq(R(0x010) & ((1 << 4) | (1 << 5)), 0, 'GRSTCTL flush bits self-clear');

// 3. Bus reset needs GAHBCFG.GINT + GINTMSK: events, ENUMSPD, IRQ67.
W(0x008, 0); // GAHBCFG: GINT off
W(0x018, G_USBRST | G_ENUMDNE); // GINTMSK
ok(otg_bus_reset() === true, 'bus reset accepted');
eq(R(0x014) & (G_USBRST | G_ENUMDNE), G_USBRST | G_ENUMDNE, 'USBRST+ENUMDNE latched');
eq((R(0x808) >> 1) & 3, 1, 'DSTS ENUMSPD = full speed');
ok(!has_pending_interrupt(), 'no IRQ without GAHBCFG.GINT');
W(0x008, 1); // GINT on: level-sensitive line re-fires
ok(has_pending_interrupt() && get_next_pending_interrupt() === 67, 'RESET pends IRQ 67');
clear_current_interrupt();
W(0x014, G_USBRST | G_ENUMDNE); // W1C retire
ok(!has_pending_interrupt(), 'IRQ retires after W1C');

// 4. EP0 bring-up + SETUP delivery (ST HAL Reset-handler sequence).
W(0x810, 1); // DIEPMSK XFRCM
W(0x814, 1 | 8); // DOEPMSK XFRCM|STUPM
W(0x81C, (1 << 0) | (1 << 16)); // DAINTMSK IN0+OUT0
W(0x018, G_USBRST | G_ENUMDNE | G_RXFLVL | G_IEPINT | G_OEPINT);
W(0x900, USBAEP | CNAK); // DIEPCTL0: active, NAK clear
W(0xB00, USBAEP | CNAK | EPENA); // DOEPCTL0: active + armed
W(0xB10, (3 << 29) | (1 << 19) | 8); // DOEPTSIZ0: STUPCNT=3, PKTCNT=1, 8B
ok(otg_inject_setup([0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x12, 0x00]) === true, 'SETUP accepted when armed');
// GRXSTSP pops received then completed; GRXSTSR would peek.
let st = R(0x020);
eq(st & 0xF, 0, 'SETUP status EP0');
eq((st >> 4) & 0x7FF, 8, 'SETUP status BCNT=8');
eq((st >> 17) & 0xF, 6, 'SETUP status = received(6)');
st = R(0x020);
eq((st >> 17) & 0xF, 4, 'SETUP status = completed(4)');
eq(R(0x020), 0, 'GRX queue drains to 0');
// FIFO holds the 8 setup bytes (LE words).
eq(R(0x1000), 0x01000680, 'SETUP FIFO word0');
eq(R(0x1004), 0x00120000, 'SETUP FIFO word1');
eq(R(0xB08) & (XFRC | STUP), XFRC | STUP, 'DOEPINT0 XFRC+STUP');
eq(R(0x818) & (1 << 16), 1 << 16, 'DAINT OUT0 pending (masked)');
eq(R(0x014) & G_OEPINT, G_OEPINT, 'GINTSTS OEPINT via DAINTMSK');
ok(has_pending_interrupt() && get_next_pending_interrupt() === 67, 'SETUP CTR pends IRQ 67');
clear_current_interrupt();
W(0xB08, XFRC | STUP); // W1C retire
eq(R(0x818), 0, 'DAINT clears with endpoint flags');
eq(R(0x014) & G_OEPINT, 0, 'OEPINT clears with DAINT');
eq(R(0xB00) & NAKSTS, NAKSTS, 'EP0-OUT NAKSTS set after transfer');
W(0x014, G_OEPINT);

// 5. IN transfer: TSIZ + EPENA, then FIFO pushes complete it.
W(0x910, (1 << 19) | 18); // DIEPTSIZ0: PKTCNT=1, 18B
W(0x900, USBAEP | CNAK | EPENA); // arm
const desc = [];
for (let i = 0; i < 18; i++) desc.push(0x30 + i);
for (let i = 0; i < 18; i += 4) {
  let w = 0;
  for (let j = 0; j < 4 && i + j < 18; j++) w |= desc[i + j] << (j * 8);
  W(0x1000, w >>> 0); // DFIFO0 push
}
const ev = drain_events();
let usbIn = null;
for (let i = 0; i < ev.length;) {
  const t = ev[i++];
  if (t === 18) { const ep = ev[i++], len = ev[i++]; usbIn = [ep, len, ev.slice(i, i + len).join(',')]; i += len; }
  else break;
}
ok(usbIn !== null && usbIn[0] === 0 && usbIn[1] === 18 && usbIn[2] === desc.join(','), `IN completion drains exact UsbIn (got ${usbIn})`);
eq(R(0x908) & XFRC, XFRC, 'DIEPINT0 XFRC');
eq(R(0x900) & EPENA, 0, 'EPENA clears on completion');
eq(R(0x900) & NAKSTS, NAKSTS, 'NAKSTS set on completion');
eq(R(0x818) & 1, 1, 'DAINT IN0 pending (masked)');
eq(R(0x014) & G_IEPINT, G_IEPINT, 'GINTSTS IEPINT via DAINTMSK');
ok(has_pending_interrupt() && get_next_pending_interrupt() === 67, 'IN CTR pends IRQ 67');
clear_current_interrupt();
W(0x908, XFRC);
W(0x014, G_IEPINT);

// 6. OUT EP1: open, arm, inject, FIFO readback, completion.
W(0xB20, USBAEP | CNAK); // DOEPCTL1 active
W(0xB30, (1 << 19) | 64); // DOEPTSIZ1: PKTCNT=1, 64B
W(0xB20, USBAEP | CNAK | EPENA); // arm
ok(otg_inject_out(1, [9, 8, 7]) === true, 'OUT EP1 accepted when armed');
eq(R(0x1000), 0x00070809, 'OUT bytes land in RXFIFO (LE)');
st = R(0x020);
eq(st & 0xF, 1, 'OUT status EP1');
eq((st >> 17) & 0xF, 2, 'OUT status = received(2)');
eq((R(0x020) >> 17) & 0xF, 3, 'OUT status = completed(3)');
eq(R(0xB28) & XFRC, XFRC, 'DOEPINT1 XFRC');
eq(R(0xB20) & NAKSTS, NAKSTS, 'EP1-OUT NAKSTS after transfer');
eq(R(0xB20) & EPENA, 0, 'EP1-OUT EPENA clears on completion');
clear_current_interrupt();
let _dd = 0;
while (get_next_pending_interrupt() !== -255 && _dd++ < 100) { clear_current_interrupt(); }
W(0xB28, XFRC);
W(0x014, G_OEPINT | G_RXFLVL);

// 7. STALL suppresses IN completion (EPENA stays, no event).
W(0x910, (1 << 19) | 4);
W(0x900, USBAEP | CNAK | EPENA | STALL);
W(0x1000, 0x11223344);
drain_events();
ok(!has_pending_interrupt() || get_next_pending_interrupt() !== 67, 'no IN IRQ while STALLed');
W(0x900, USBAEP | CNAK | EPENA); // clear STALL: armed transfer completes
let ev2 = drain_events(), saw = false;
for (let i = 0; i < ev2.length;) {
  const t = ev2[i++];
  if (t === 18) { const ep = ev2[i++], len = ev2[i++]; saw = saw || (ep === 0 && len === 4); i += len; }
  else break;
}
ok(saw, 'IN completes after STALL clear');
clear_current_interrupt();
W(0x908, XFRC);
W(0x014, G_IEPINT);

// 8. EPDIS tears down with EPDISD.
W(0x910, (1 << 19) | 4);
W(0x900, USBAEP | CNAK | EPENA);
W(0x900, USBAEP | EPDIS);
eq(R(0x900) & EPENA, 0, 'EPDIS clears EPENA');
eq(R(0x908) & EPDISD, EPDISD, 'EPDISD event raised');
W(0x908, EPDISD);

// 9. Detach: tokens stop, SOF freezes, reset reattaches.
W(0x018, G_USBRST | G_ENUMDNE | G_RXFLVL | G_IEPINT | G_OEPINT | (1 << 11));
W(0x014, 0xFFFFFFFF);
ok(otg_detach() === true, 'detach accepted');
eq(R(0x014) & G_USBSUSP, G_USBSUSP, 'USBSUSP on detach');
ok(has_pending_interrupt() && get_next_pending_interrupt() === 67, 'detach SUSP pends IRQ 67');
clear_current_interrupt();
W(0xB00, USBAEP | CNAK | EPENA);
W(0xB10, (3 << 29) | (1 << 19) | 8);
ok(otg_inject_setup([0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x12, 0x00]) === false, 'SETUP dropped while detached');
const fnrD = R(0x808);
step_batch(72000);
eq(R(0x808), fnrD, 'DSTS FNSOF frozen while detached');
ok(otg_bus_reset() === true, 'bus reset reattaches');
eq(R(0x014) & (G_USBRST | G_ENUMDNE), G_USBRST | G_ENUMDNE, 'RESET+ENUMDNE on reattach');
eq((R(0x808) >> 1) & 3, 1, 'ENUMSPD full speed after reset');
clear_current_interrupt();
W(0x014, 0xFFFFFFFF);

// 10. PWRDWN (GCCFG bit 16) gates the macro.
W(0x038, 1 << 16);
W(0xB00, USBAEP | CNAK | EPENA);
W(0xB10, (3 << 29) | (1 << 19) | 8);
ok(otg_inject_setup([0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x12, 0x00]) === false, 'SETUP dropped while PWRDWN');
ok(otg_bus_reset() === false, 'bus reset ignored while PWRDWN');
const fnrP = R(0x808);
step_batch(72000);
eq(R(0x808), fnrP, 'FNSOF frozen while PWRDWN');
W(0x038, 0); // power back up

// 11. DAD address filter (DCFG DAD[10:4]).
W(0xB00, USBAEP | CNAK | EPENA);
W(0xB10, (3 << 29) | (1 << 19) | 8);
W(0x800, (5 << 4) | 0x02200000); // DAD = 5
ok(otg_inject_setup([0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x12, 0x00], 6) === false, 'wrong address filtered');
ok(otg_inject_setup([0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x12, 0x00], 5) === true, 'own address accepted');
W(0xB00, USBAEP | CNAK | EPENA);
W(0xB10, (3 << 29) | (1 << 19) | 8);
ok(otg_inject_setup([0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x12, 0x00]) === true, 'no addr = correctly-addressed host');
W(0x800, 0x02200000); // DAD = 0
W(0xB00, USBAEP | CNAK | EPENA);
W(0xB10, (3 << 29) | (1 << 19) | 8);
ok(otg_inject_setup([0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x12, 0x00], 5) === false, 'nonzero filtered while unaddressed');
clear_current_interrupt();
W(0x014, 0xFFFFFFFF);
W(0xB08, 0xFFFFFFFF);
W(0xB28, 0xFFFFFFFF);

// 12. Host block inert + MMIS; SOF engine advances FNSOF.
W(0x400, 0xDEAD); // HCFG write in device mode
ok((R(0x014) & G_MMIS) !== 0, 'MMIS flagged on host touch');
eq(R(0x400), 0, 'host registers read 0');
W(0x014, G_MMIS);
const fnr0 = R(0x808);
step_batch(72000);
ok(R(0x808) !== fnr0, 'FNSOF advances with SOF');
eq(R(0x014) & G_SOF, G_SOF, 'GINTSTS SOF set');

// 13. SDIS disconnect + RWUSIG wake.
W(0x804, 1 << 1); // SDIS
eq(R(0x808) & 1, 1, 'DSTS SUSPSTS on SDIS');
W(0xB00, USBAEP | CNAK | EPENA);
W(0xB10, (3 << 29) | (1 << 19) | 8);
ok(otg_inject_setup([0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x12, 0x00]) === false, 'SETUP dropped while SDIS');
W(0x804, 1); // RWUSIG while suspended: wake + WKUPINT
eq(R(0x808) & 1, 0, 'RWUSIG wakes (SUSPSTS clear)');
eq(R(0x014) & G_WKUP, G_WKUP, 'WKUPINT on remote wakeup');
W(0x804, 0); // SDIS clear
W(0x014, 0xFFFFFFFF);

console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
process.exit(failed ? 1 : 0);
