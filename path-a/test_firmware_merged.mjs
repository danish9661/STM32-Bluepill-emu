// Path A merged-module firmware battery: boots EVERY shipped firmware through
// the merged single-wasm emulator (emulator.js run-loop path) and asserts the
// expected UART markers, plus full-run checks for the added peripherals
// (JS peripherals via createEmulator opts, FSMC bank, F105 SVD chip).
//
// Each section runs in its OWN child process: the merged module maps a
// ~1.9GB shared backing per process (uc_mem_map_ptr), so 8+ emulator
// contexts in one process hit the 2GB wasm memory ceiling. Spawning per
// section also isolates any crash to one section. Run the file with no args
// to execute all sections sequentially as children, or with `--section N`
// to run one section in-process (used by the parent).
import { readFileSync } from 'fs';
import { mkdirSync, writeFileSync } from 'fs';
import { tmpdir } from 'os';
import { join } from 'path';
import { createEmulator } from './emulator-merged.mjs';

const REPO = decodeURIComponent(new URL('..', import.meta.url).pathname + '/');
const site = (f) => `${REPO}site/${f}`;
const svdPath = `${REPO}svd/STM32F105xx.svd`;

// eeprom/spi_flash images (gitignored in tests/*/build — recreate per AGENTS.md)
const BIN = join(tmpdir(), 'path-a-bins');
mkdirSync(BIN, { recursive: true });
const mkBin = (f, size, seed = (b) => {}) => {
    const b = Buffer.alloc(size);
    seed(b);
    writeFileSync(join(BIN, f), b);
    return join(BIN, f);
};
mkBin('eeprom.bin', 65536);
mkBin('eeprom2.bin', 65536, (b) => { b[0] = 0x42; b[1] = 0x24; });
mkBin('spi_flash.bin', 65536);
mkBin('spi_flash2.bin', 65536);

const read = (f) => readFileSync(`${REPO}${f}`);
const CHUNK = 5000000;

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

async function runFirmware(name, elf, { budget, ext_devices, uart = [], opts = {} } = {}) {
    const emu = await createEmulator({
        firmware: read(elf),
        ext_devices,
        ...opts,
    });
    for (const b of uart) emu.uartRx(b);
    let done = 0, stopped = false;
    const t0 = Date.now();
    while (done < budget) {
        const n = Math.min(CHUNK, budget - done);
        const r = await emu.run(n);
        done += n;
        stopped = r.stopped;
        if (stopped) break;
    }
    const secs = ((Date.now() - t0) / 1000).toFixed(1);
    return { emu, done, stopped, secs, out: String(emu.getUartOutput() || '') };
}

const has = (out, s) => out.includes(s);
const failLines = (out) => out.split('\n').filter((l) => l.includes('FAIL')).length;

const SECTIONS = [];

// ---------------------------------------------------------------
// 1. Echo: banner + echo of injected bytes
// ---------------------------------------------------------------
SECTIONS.push({ name: 'echo', fn: async () => {
    // delay(50) per char = 3.6M instructions each; 15M covers A+B + banner.
    const r = await runFirmware('echo', 'site/arduino_echo.elf', { budget: 15000000, uart: [0x41, 0x42] });
    ok(has(r.out, '=== UART Echo demo ==='), 'echo: banner');
    ok(has(r.out, 'AB'), 'echo: injected AB echoed back');
    ok(failLines(r.out) === 0, 'echo: no FAIL lines');
    console.log(`  echo: ${r.done} instr in ${r.secs}s`);
} });

// ---------------------------------------------------------------
// 2. Fade: TIM2 PWM duty lines
// ---------------------------------------------------------------
SECTIONS.push({ name: 'fade', fn: async () => {
    const r = await runFirmware('fade', 'site/arduino_fade.elf', { budget: 30000000 });
    ok(has(r.out, '=== TIM2 PWM fade demo ==='), 'fade: banner');
    ok(has(r.out, 'duty='), 'fade: duty= lines');
    ok(failLines(r.out) === 0, 'fade: no FAIL lines');
    console.log(`  fade: ${r.done} instr in ${r.secs}s`);
} });

// ---------------------------------------------------------------
// 3. Timer on UART: t= lines after ~1s emulated
// ---------------------------------------------------------------
SECTIONS.push({ name: 'timer_uart', fn: async () => {
    const r = await runFirmware('timer_uart', 'site/arduino_timer_uart.elf', { budget: 160000000 });
    ok(has(r.out, '=== TIM2 timer on UART demo ==='), 'timer_uart: banner');
    ok(has(r.out, 't='), 'timer_uart: t= lines (SysTick + TIM2 live)');
    ok(failLines(r.out) === 0, 'timer_uart: no FAIL lines');
    console.log(`  timer_uart: ${r.done} instr in ${r.secs}s`);
} });

// ---------------------------------------------------------------
// 4. ADC on UART: adc= lines
// ---------------------------------------------------------------
SECTIONS.push({ name: 'adc_uart', fn: async () => {
    const r = await runFirmware('adc_uart', 'site/arduino_adc_uart.elf', { budget: 160000000 });
    ok(has(r.out, '=== ADC on UART demo (PA0) ==='), 'adc_uart: banner');
    ok(has(r.out, 'adc='), 'adc_uart: adc= lines');
    ok(failLines(r.out) === 0, 'adc_uart: no FAIL lines');
    console.log(`  adc_uart: ${r.done} instr in ${r.secs}s`);
} });

// ---------------------------------------------------------------
// 5. Flash demo: SPI flash JEDEC + OLED probe
// ---------------------------------------------------------------
SECTIONS.push({ name: 'flash_demo', fn: async () => {
    const r = await runFirmware('flash_demo', 'site/arduino_flash_demo.elf', {
        budget: 5000000,
        ext_devices: {
            spi_flash: [{ peripheral: 'SPI1', jedec_id: 0xEF4016, data: readFileSync(join(BIN, 'spi_flash.bin')), cs: 'PA4' }],
            i2c_oled: [{ peripheral: 'I2C1', address: 0x3C }],
        },
    });
    ok(has(r.out, '=== SPI flash + I2C devices on UART ==='), 'flash_demo: banner');
    ok(has(r.out, 'JEDEC=EF4016'), 'flash_demo: SPI flash JEDEC id');
    ok(has(r.out, 'OLED=found'), 'flash_demo: I2C OLED found');
    ok(failLines(r.out) === 0, 'flash_demo: no FAIL lines');
    console.log(`  flash_demo: ${r.done} instr in ${r.secs}s`);
} });

// ---------------------------------------------------------------
// 6. WS2812: DMA + SPI1 strip, frames counter
// ---------------------------------------------------------------
SECTIONS.push({ name: 'ws2812', fn: async () => {
    // The frames= print fires at t>=2000ms emulated (~144M instructions);
    // 160M ended at exactly the boundary, so the check missed. 190M is safe.
    const r = await runFirmware('ws2812', 'site/arduino_ws2812.elf', { budget: 190000000 });
    ok(has(r.out, '=== WS2812 strip over SPI1 + DMA1 ch3 ==='), 'ws2812: banner');
    ok(has(r.out, 'WS2812=ok'), 'ws2812: first frame OK');
    ok(has(r.out, 'frames='), 'ws2812: frames counter (DMA push path)');
    ok(failLines(r.out) === 0, 'ws2812: no FAIL lines');
    console.log(`  ws2812: ${r.done} instr in ${r.secs}s`);
} });

// ---------------------------------------------------------------
// 7. Hardware showcase: OLED + LCD + 7-seg + RGB + buzzer + button
// ---------------------------------------------------------------
SECTIONS.push({ name: 'showcase', fn: async () => {
    // Heartbeat btn= lines print every ~8s emulated (>70M instructions);
    // 25M was too short. 100M matches the boot.mjs probe that passed.
    const r = await runFirmware('showcase', 'site/arduino_hw_showcase.elf', {
        budget: 100000000,
        ext_devices: {
            i2c_oled: [{ peripheral: 'I2C1', address: 0x3C }],
            lcd: [{ peripheral: 'SPI1', cs: 'PA8' }],
        },
    });
    ok(has(r.out, '=== Peripheral showcase: OLED + LCD + 7-seg + RGB + buzzer + button ==='), 'showcase: banner');
    r.emu.gpioSetInput(1, 13, true);   // press PB13
    r.emu.gpioSetInput(1, 13, false);  // release
    // The heartbeat (t=Ns btn=) prints once per second; the press must be
    // followed by enough instructions to reach the next second boundary
    // (up to ~72M), not just 2M.
    const r2 = await r.emu.run(100000000);
    const out2 = String(r2 ? r.emu.getUartOutput() || '' : '');
    const btnAfter = out2.split('\n').filter((l) => l.includes('btn=')).length;
    ok(btnAfter > 0, `showcase: button heartbeat lines (${btnAfter})`);
    ok(failLines(out2) === 0, 'showcase: no FAIL lines');
    console.log(`  showcase: ${r.done + 100000000} instr in ${r.secs}s`);
} });

// ---------------------------------------------------------------
// 8. JS peripheral registered through createEmulator opts (full run)
// ---------------------------------------------------------------
SECTIONS.push({ name: 'js_periph', fn: async () => {
    const JS_BASE = 0x40006800; // gap between CAN1 (0x40006400) and BKP (0x40006C00) on F103
    let reads = 0, writes = 0;
    const r = await runFirmware('js_periph', 'site/arduino_echo.elf', {
        budget: 2000000,
        opts: {
            js_peripherals: [{
                base: JS_BASE, size: 0x400,
                read: (addr, size) => { reads++; return addr === JS_BASE ? 0x42 : 0; },
                write: (addr, value, size) => { writes++; },
            }],
        },
    });
    ok(r.emu.periphRead(JS_BASE, 4) === 0x42, 'js_periph: read callback value through emulator');
    ok(reads >= 1, `js_periph: read callback fired (${reads})`);
    r.emu.periphWrite(JS_BASE + 4, 4, 0xDEADBEEF);
    ok(writes === 1, 'js_periph: write callback fired through emulator');
    ok(failLines(r.out) === 0, 'js_periph: no FAIL lines');
    console.log(`  js_periph: ${r.done} instr in ${r.secs}s`);
} });

// ---------------------------------------------------------------
// 9. FSMC bank NOR on the merged module (added peripheral)
// ---------------------------------------------------------------
SECTIONS.push({ name: 'fsmc', fn: async () => {
    const { loadMerged } = await import('./loader.mjs');
    const { periph } = await loadMerged();
    const data = new Uint8Array(4096);
    data[0] = 0xAA; data[1] = 0xBB;
    periph.add_fsmc_bank('FSMC.BANK1', data);
    periph.init();
    periph.periph_write(0xA0000000, 4, 0x00000003); // BCR1: MBKEN + WREN
    periph.periph_write(0x60000000 + 4, 2, 0x1234); // NOR write (16-bit)
    const r = periph.periph_read(0x60000000 + 4, 2);
    ok(r === 0x1234, `fsmc: NOR bank1 read-back (0x${r.toString(16)})`);
    const rd = periph.periph_read(0x60000000, 1);
    ok(rd === 0xAA, `fsmc: NOR bank1 byte read (0x${rd.toString(16)})`);
    console.log('  fsmc: ok');
} });

// ---------------------------------------------------------------
// 10. F105 SVD chip: echo boots on the connectivity-line map (CAN2 etc.)
// ---------------------------------------------------------------
SECTIONS.push({ name: 'f105', fn: async () => {
    const svd = readFileSync(svdPath, 'utf8');
    const r = await runFirmware('f105', 'site/arduino_echo.elf', {
        budget: 2000000,
        opts: { chip: 'stm32f105', svd },
    });
    ok(has(r.out, '=== UART Echo demo ==='), 'f105: banner on SVD chip');
    r.emu.periphWrite(0x40006800, 4, 0x00000041); // CAN2 MCR (F105-only)
    const mcr = r.emu.periphRead(0x40006800, 4);
    ok(mcr === 0x41, `f105: CAN2 register live (MCR=0x${mcr.toString(16)})`);
    ok(failLines(r.out) === 0, 'f105: no FAIL lines');
    console.log(`  f105: ${r.done} instr in ${r.secs}s`);
} });

// ---------------------------------------------------------------
const isSection = process.argv.includes('--section');
if (isSection) {
    const idx = Number(process.argv[process.argv.indexOf('--section') + 1]);
    const sec = SECTIONS[idx];
    try {
        await sec.fn();
    } catch (e) {
        console.log(`SECTION ${sec.name} THREW:`, e);
        failed++;
    }
    console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
    process.exit(failed ? 1 : 0);
}

// Parent: run each section in its own child process (memory ceiling + crash
// isolation). Child output is printed as-is; results are parsed from the
// child's trailing "Results:" line.
const { execFileSync } = await import('child_process');
const { fileURLToPath } = await import('url');
let totalPass = 0, totalFail = 0;
for (let i = 0; i < SECTIONS.length; i++) {
    console.log(`\n=== Section ${i + 1}/${SECTIONS.length}: ${SECTIONS[i].name} ===`);
    try {
        const out = execFileSync(process.execPath, [fileURLToPath(import.meta.url), '--section', String(i)], {
            encoding: 'utf8', timeout: 600000, maxBuffer: 16 * 1024 * 1024,
        });
        process.stdout.write(out);
        const m = out.match(/Results: (\d+) passed, (\d+) failed/);
        if (m) { totalPass += Number(m[1]); totalFail += Number(m[2]); }
    } catch (e) {
        const out = String(e.stdout || '');
        process.stdout.write(out);
        process.stdout.write(String(e.stderr || ''));
        totalFail += 99;
        console.log(`  section ${SECTIONS[i].name} FAILED (${e.status ?? e.message})`);
    }
}
console.log(`\nResults: ${totalPass} passed, ${totalFail} failed, ${totalPass + totalFail} total`);
process.exit(totalFail ? 1 : 0);
