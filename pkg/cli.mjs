#!/usr/bin/env node
import { readFileSync } from 'fs';
import path from 'path';
import * as yaml from 'js-yaml';
import { parseIntelHex, parseSymbolMap, parseElf } from './emulator.js';
import * as periph from './stm32_bluepill_wasm.js';
const { periph_read, periph_write, tick, step, step_batch, get_next_pending_interrupt, dma_get_all_pending, 
dma_set_completed_many, is_watchdog_reset_requested, add_spi_flash, add_i2c_eeprom, add_touchscreen,
add_lcd, add_i2c_oled, can_inject_message, raise_fault,
init, init_svd, has_pending_interrupt, get_uart_output, uart_rx_byte, uart_rx_pending, gpio_read_output,
set_intr_masks, clear_current_interrupt, nvic_systick_take } = periph;

periph.initSync({ module: readFileSync(new URL('./stm32_bluepill_wasm_bg.wasm', import.meta.url)) });

const parseHex = (v) => typeof v === 'number' ? v : parseInt(v, 16);

// Unicorn ARM cannot decode `mrs rX, msp` (used by newlib _sbrk). In thread
// mode MSP == SP, so rewrite to `mov rX, sp` + nop (same 4-byte footprint).
function patchMrsMsp(data) {
    let patched = 0;
    for (let i = 0; i + 3 < data.length; i++) {
        if (data[i] === 0xEF && data[i + 1] === 0xF3
            && data[i + 2] === 0x08 && (data[i + 3] & 0xF0) === 0x80) {
            const rd = data[i + 3] & 0x0F;
            const mov = 0x4668 | rd;
            data[i] = mov & 0xFF;
            data[i + 1] = mov >> 8;
            data[i + 2] = 0x00;
            data[i + 3] = 0xBF;
            patched++;
        }
    }
    if (patched > 0) console.log(`Patched ${patched} 'mrs msp' instruction(s) to 'mov sp' (malloc/_sbrk support)`);
    return data;
}

function loadFirmware(buf) {
    if (buf.length > 4 && buf[0] === 0x7F && buf[1] === 0x45 && buf[2] === 0x4C && buf[3] === 0x46) {
        const elf = parseElf(buf);
        console.log(`Parsed ELF: ${elf.regions.length} load segments, ${elf.symbols.length} symbols`);
        for (const r of elf.regions) {
            r.data = patchMrsMsp(r.data);
        }
        return { data: new Uint8Array(0), base: null, regions: elf.regions, symbols: elf.symbols };
    }
    if (buf.length > 0 && buf[0] === 0x3A) {
        const parsed = parseIntelHex(new TextDecoder().decode(buf));
        console.log(`Parsed Intel HEX: base=0x${parsed.base.toString(16)} (${parsed.data.length} bytes)`);
        return { data: patchMrsMsp(parsed.data), base: parsed.base, regions: null, symbols: null };
    }
    return { data: patchMrsMsp(buf), base: null, regions: null, symbols: null };
}

function makeResolver(symbols) {
    if (!symbols || !symbols.length) return null;
    const sorted = symbols.slice().sort((a, b) => a.addr - b.addr);
    return (addr) => {
        const a = addr & ~1;
        let lo = 0, hi = sorted.length - 1, best = -1;
        while (lo <= hi) {
            const mid = (lo + hi) >> 1;
            if (sorted[mid].addr <= a) { best = mid; lo = mid + 1; } else hi = mid - 1;
        }
        if (best < 0) return null;
        const s = sorted[best];
        const off = a - s.addr;
        if (off > 0x20000) return null;
        return off > 0 ? `${s.name}+0x${off.toString(16)}` : s.name;
    };
}

async function getMUnicorn() {
    const { createRequire } = await import('module');
    const require = createRequire(import.meta.url);
    return require('./unicorn_arm.cjs');
}

async function main() {
    await null;
    const args = process.argv.slice(2);
    const configPaths = args.filter(a => a.startsWith('--config=')).map(a => a.split('=')[1]);
    const posArgs = args.filter(a => !a.startsWith('--'));
    const maxInst = parseInt(
        args.find(a => a.startsWith('--max='))?.split('=')[1]
        || (configPaths.length > 0 ? posArgs[0] : posArgs[1])
        || process.env.MAX_INST || '1000000', 10);
    const showRegs = args.includes('--regs') || process.env.SHOW_REGS === '1';
    const mapPath = args.find(a => a.startsWith('--map='))?.split('=')[1];
    let uartAddr = parseInt(args.find(a => a.startsWith('--uart='))?.split('=')[1] || process.env.UART_ADDR || '0x40013800', 16);

    let config = {};
    if (configPaths.length > 0) {
        for (const cp of configPaths) {
            const raw = yaml.load(readFileSync(cp, 'utf8'));
            const cfgDir = path.dirname(path.resolve(cp));
            if (raw.regions) raw.regions = raw.regions.map(r => ({ ...r, _dir: cfgDir }));
            if (raw.patches) raw.patches = raw.patches.map(p => ({ ...p, _dir: cfgDir }));
            raw._devices_dir = cfgDir;
            config = { ...config, ...raw, regions: [...(config.regions || []), ...(raw.regions || [])], patches: [...(config.patches || []), ...(raw.patches || [])] };
        }
        console.log(`Using config(s): ${configPaths.join(', ')}`);
    }

    const MUnicorn = await getMUnicorn();
    const Module = await MUnicorn({});

    let firmware;
    let fwBase = null;
    let fwRegions = null;
    let fwSymbols = null;
    let vector_table;
    let memRegions;

    if (config.regions) {
        memRegions = config.regions.map(r => ({ ...r, start: parseHex(r.start), size: parseHex(r.size) }));
        const romRegion = memRegions.find(r => r.load);
        if (!romRegion) { console.error('No region with load file found'); process.exit(1); }
        vector_table = parseHex(config.cpu?.vector_table || romRegion.start);
        const romFile = path.resolve(romRegion._dir || config._devices_dir, romRegion.load);
        const fw = loadFirmware(readFileSync(romFile));
        firmware = fw.data;
        fwBase = fw.base;
        fwRegions = fw.regions;
        fwSymbols = fw.symbols;
        console.log(`Loading firmware: ${romFile} (${firmware.length} bytes)`);

        // Register external devices from config BEFORE init()
        if (config.devices) {
            for (const [type, devs] of Object.entries(config.devices)) {
                for (const d of devs || []) {
                    if (type === 'i2c_eeprom') {
                        const data = d.file ? readFileSync(path.resolve(config._devices_dir, d.file)) : new Uint8Array(d.size || 0);
                        add_i2c_eeprom(d.peripheral, parseHex(d.addr), data);
                    } else if (type === 'spi_flash') {
                        const data = d.file ? readFileSync(path.resolve(config._devices_dir, d.file)) : new Uint8Array(d.size || 0);
                        add_spi_flash(d.peripheral, parseHex(d.jedec_id), data, d.cs || null);
                    } else if (type === 'usart_probe') {
                        uartAddr = parseHex(d.peripheral.match(/[0-9a-fA-F]+/)?.[0]) ? parseInt(d.peripheral, 16) : (PERIPH_ADDR[d.peripheral] || uartAddr);
                    } else if (type === 'touchscreen') {
                        add_touchscreen(d.peripheral, d.touch_detected_pin || null, d.cs || null);
                    } else if (type === 'lcd') {
                        add_lcd(d.peripheral, d.cs || null);
                    } else if (type === 'i2c_oled') {
                        add_i2c_oled(d.peripheral, parseInt(d.addr || '0x3C', 16), parseInt(d.width || '128', 10), parseInt(d.height || '64', 10));
                    }
                }
            }
        }

        if (config.cpu?.use_hardcoded) {
            init();
        } else {
            const svdPath = path.resolve(config._devices_dir, config.cpu?.svd || 'stm32f103c8.svd');
            const svdXml = readFileSync(svdPath, 'utf8');
            init_svd(svdXml);
        }

        if (config.patches) {
            for (const p of config.patches) {
                const start = BigInt(parseHex(p.start));
                const data = new Uint8Array(p.data);
                const romRegionStart = BigInt(romRegion.start);
                const relOff = Number(start - romRegionStart);
                if (relOff >= 0 && relOff + data.length <= firmware.length) {
                    data.forEach((b, i) => firmware[relOff + i] = b);
                    console.log(`Applied patch at 0x${start.toString(16)}: [${data.join(', ')}]`);
                }
            }
        }
    } else {
        const firmwarePath = posArgs[0] || process.env.FIRMWARE;
        if (!firmwarePath) {
            console.error('Usage: node cli.mjs <firmware.bin|.hex|.elf> [max_instructions] [--config=path] [--max=N] [--map=file.map]');
            console.error('  or set FIRMWARE env var');
            process.exit(1);
        }
        const fw = loadFirmware(readFileSync(firmwarePath));
        firmware = fw.data;
        fwBase = fw.base;
        fwRegions = fw.regions;
        fwSymbols = fw.symbols;
        console.log(`Loading firmware: ${firmwarePath} (${firmware.length} bytes)`);

        const fwDir = path.dirname(path.resolve(firmwarePath));
        for (const fn of ['eeprom.bin', 'spi_flash.bin']) {
            try {
                const data = readFileSync(`${fwDir}/${fn}`);
                if (fn.startsWith('eeprom')) add_i2c_eeprom("I2C1", 0x50, data);
                else add_spi_flash("SPI1", 0xef4016, data, null);
                console.log(`Loaded ext device: ${fwDir}/${fn} (${data.length} bytes)`);
            } catch (_) {}
        }

        // Try SVD file, fall back to hardcoded
        const svdFallbackPaths = [
            path.resolve(fwDir, 'STM32F103.svd'),
            path.resolve(process.cwd(), 'STM32F103.svd'),
            path.resolve(process.cwd(), 'svd', 'STM32F103.svd'),
        ];
        let svdLoaded = false;
        for (const svdPath of svdFallbackPaths) {
            try {
                const svdXml = readFileSync(svdPath, 'utf8');
                init_svd(svdXml);
                console.log(`Using SVD: ${svdPath}`);
                svdLoaded = true;
                break;
            } catch (_) {}
        }
        if (!svdLoaded) {
            init();
        }
        vector_table = 0x08000000;
        memRegions = [
            { start: 0x08000000, size: 0x10000 },
            { start: 0x20000000, size: 0x5000 },
        ];
    }

    console.log(`Max instructions: ${maxInst}`);
    console.log('Initializing Unicorn...');

    const uc = new Module.Unicorn(
        Module.ARCH_ARM,
        Module.MODE_THUMB | Module.MODE_LITTLE_ENDIAN
    );

    for (const r of memRegions) {
        uc.mem_map(r.start, r.size, Module.PROT_ALL);
    }
    if (mapPath) {
        fwSymbols = parseSymbolMap(readFileSync(path.resolve(mapPath), 'utf8'));
        console.log(`Loaded ${fwSymbols.length} symbols from map: ${mapPath}`);
    }

    const romRegion = memRegions.find(r => firmware && (r._firmware || r.load || (r.start <= vector_table && r.start + r.size > vector_table)));
    const romStart = romRegion ? romRegion.start : (memRegions[0]?.start || 0x08000000);
    if (fwRegions && fwRegions.length) {
        for (const r of fwRegions) {
            uc.mem_write(BigInt(r.start), r.data);
        }
    } else if (firmware && fwBase != null) {
        uc.mem_write(BigInt(fwBase), firmware);
    } else if (firmware) {
        uc.mem_write(BigInt(romStart), firmware);
        if (romStart !== vector_table) uc.mem_write(BigInt(vector_table), firmware);
    }

    // TEMP WORKAROUND: Unicorn skips the two `bl HAL_NVIC_EnableIRQ` in
    // i2c_init() (0x8001bb8/0x8001bcc). Replace 0x8001bbc..0x8001bdb with
    // inline NVIC ISER0/ISER1 writes (SetPriority calls preserved).
    try {
        const patchAddr = 0x8001BBCn;
        const probe = uc.mem_read(patchAddr, 4);
        if (probe[0] === 0x00 && probe[1] === 0xF0 && probe[2] === 0x92 && probe[3] === 0xFD) {
            uc.mem_write(patchAddr, new Uint8Array([
                0x00, 0xF0, 0x92, 0xFD, // bl  HAL_NVIC_SetPriority (r0=31)
                0x4E, 0xF2, 0x00, 0x13, // movw r3, #0xE100
                0xCE, 0xF2, 0x00, 0x03, // movt r3, #0xE000  -> r3 = 0xE000E100
                0x40, 0xF2, 0x00, 0x02, // movw r2, #0x0000
                0xC8, 0xF2, 0x00, 0x02, // movt r2, #0x8000  -> r2 = 0x80000000
                0x1A, 0x60,             // str  r2, [r3]     -> ISER0 |= bit31 (IRQ31)
                0x20, 0x20,             // movs r0, #32
                0x00, 0xF0, 0x86, 0xFD, // bl  HAL_NVIC_SetPriority (r0=32)
                0x01, 0x22,             // movs r2, #1
                0x5A, 0x60,             // str  r2, [r3, #4] -> ISER1 |= 1 (IRQ32)
            ]));
            console.log('Applied i2c_init IRQ-enable patch (Unicorn bl skip workaround)');
        }
    } catch (e) {
        console.error('i2c_init patch failed:', e.message);
    }

    const periphRanges = [
        [0x40000000, 0xB0000000],
        [0xE0000000, 0xE1000000],
    ];
    for (const [start, end] of periphRanges) {
        uc.mem_map(start, end - start, Module.PROT_READ | Module.PROT_WRITE);
    }

    const read32 = (addr) => {
        const b = uc.mem_read(BigInt(addr), 4);
        const dt = new DataView(b.buffer, b.byteOffset, b.byteLength);
        return dt.getUint32(0, true);
    };

    const sp_init = read32(vector_table);
    const pc_init = read32(vector_table + 4);

    uc.reg_write_i32(Module.ARM_REG_SP, sp_init);
    uc.reg_write_i32(Module.ARM_REG_PC, pc_init | 1);

    console.log(`SP=0x${sp_init.toString(16)} PC=0x${(pc_init | 1).toString(16)}`);

    const memReadHook = (handle, type, address, size, value, user_data) => {
        const addr32 = Number(address);
        let val;
        if (addr32 >= 0xE0001000 && addr32 < 0xE0001100) {
            val = addr32 === 0xE0001004
                ? Number(instCount & 0xFFFFFFFF)
                : (addr32 === 0xE0001000 ? 1 : 0);
        } else {
            val = periph_read(addr32, size) >>> 0;
        }
        const bytes = new Uint8Array(size);
        for (let i = 0; i < size; i++) {
            bytes[i] = (val >> (i * 8)) & 0xFF;
        }
        uc.mem_write(address, bytes);
    };

    const memWriteHook = (handle, type, address, size, value, user_data) => {
        const addr32 = Number(address);
        const valueNum = Number(value);
        periph_write(addr32, size, valueNum);
        if (addr32 === 0x40005410 && (valueNum & 1) === 1) {
            try {
                const hi2c1Ptr = read32(0x200002d8);
                if (hi2c1Ptr && hi2c1Ptr !== 0xFFFFFFFF) {
                    uc.mem_write(BigInt(hi2c1Ptr + 0x3D), new Uint8Array([0x22]));
                }
            } catch (_) {}
        }
    };

    /* CAN RX injection: firmware sets canRxArmed=1, then waits for a frame */
    const canArmedSym = fwSymbols?.find(s => s.name === 'canRxArmed');

    for (const [start, end] of periphRanges) {
        uc.hook_add(Module.HOOK_MEM_READ, memReadHook, null, start, end);
        uc.hook_add(Module.HOOK_MEM_WRITE, memWriteHook, null, start, end);
    }

    let instCount = 0;
    let batchInstCount = 0;
    let stopRequested = false;

    // Hookless instruction counting: emu_start(begin, 0, 0, maxBatch) stops exactly at
    // maxBatch instructions except on a fault (unmapped access, ~0.01% of batches),
    // where the faulting instruction is skipped and the batch credited in full.
    // A JS codeHook per instruction cost ~20% of runtime (10.9s -> 8.7s at 200M);
    // a full-batch credit is exact for normal batches and off by <1 batch on rare
    // faults. Handler runs (inside processInterrupts) are not credited — the
    // instruction-delta peripherals self-correct. Actual tick/interrupt processing
    // still happens in step_batch() after each Unicorn batch.

    const stdinQueue = [];
    if (process.stdin.isTTY) process.stdin.setRawMode(true);
    process.stdin.on('data', (chunk) => { for (const b of chunk) stdinQueue.push(b); });
    process.stdin.resume();
    if (process.stdin.isTTY) process.on('SIGINT', () => { process.stdin.setRawMode(false); process.exit(0); });

    // SVC frames live here while their handler runs. The frame is also written
    // to the real stack so handler code can inspect it (Cortex-M ABI).
    const svcStack = [];
    const intrHook = (handle, intno, user_data) => {
        if (intno === 8) {
            const sp = uc.reg_read_i32(Module.ARM_REG_SP);
            const frame = uc.mem_read(BigInt(sp), 32);
            const sv = new DataView(frame.buffer, frame.byteOffset, frame.byteLength);
            uc.reg_write_i32(Module.ARM_REG_R0, sv.getUint32(28, true));
            uc.reg_write_i32(Module.ARM_REG_R1, sv.getUint32(24, true));
            uc.reg_write_i32(Module.ARM_REG_R2, sv.getUint32(20, true));
            uc.reg_write_i32(Module.ARM_REG_R3, sv.getUint32(16, true));
            uc.reg_write_i32(Module.ARM_REG_R12, sv.getUint32(12, true));
            uc.reg_write_i32(Module.ARM_REG_LR, sv.getUint32(8, true));
            uc.reg_write_i32(Module.ARM_REG_PC, sv.getUint32(4, true) | 1);
            uc.reg_write_i32(Module.ARM_REG_SP, sp + 32);
        } else if (intno === 2 && svcStack.length < 8) {
            // SVC: stack the interrupted context, enter handler mode (LR = EXC_RETURN),
            // jump to the SVCall vector. Return happens when the handler executes
            // `bx lr` (Unicorn faults fetching 0xFFFFFFFx, caught in the main loop).
            const sp = uc.reg_read_i32(Module.ARM_REG_SP);
            const saved = {
                sp,
                r0: uc.reg_read_i32(Module.ARM_REG_R0),
                r1: uc.reg_read_i32(Module.ARM_REG_R1),
                r2: uc.reg_read_i32(Module.ARM_REG_R2),
                r3: uc.reg_read_i32(Module.ARM_REG_R3),
                r12: uc.reg_read_i32(Module.ARM_REG_R12),
                lr: uc.reg_read_i32(Module.ARM_REG_LR),
                pc: uc.reg_read_i32(Module.ARM_REG_PC),
                xpsr: uc.reg_read_i32(Module.ARM_REG_XPSR),
            };
            svcStack.push(saved);
            const frame = new Uint8Array(32);
            const sv = new DataView(frame.buffer);
            sv.setUint32(0, saved.xpsr, true);
            sv.setUint32(4, saved.pc, true); // return PC = instruction after svc
            sv.setUint32(8, saved.lr, true);
            sv.setUint32(12, saved.r12, true);
            sv.setUint32(16, saved.r3, true);
            sv.setUint32(20, saved.r2, true);
            sv.setUint32(24, saved.r1, true);
            sv.setUint32(28, saved.r0, true);
            uc.mem_write(BigInt(sp - 32), frame);
            uc.reg_write_i32(Module.ARM_REG_SP, sp - 32);
            const control = uc.reg_read_i32(Module.ARM_REG_CONTROL);
            uc.reg_write_i32(Module.ARM_REG_LR, control & 1 ? 0xFFFFFFFD : 0xFFFFFFF9);
            uc.reg_write_i32(Module.ARM_REG_PC, read32(vector_table + 4 * 11));
        }
    };
    uc.hook_add(Module.HOOK_INTR, intrHook, null);

    const processDma = () => {
        const flat = dma_get_all_pending();
        let doneBits = 0;
        for (let i = 0; i + 7 <= flat.length; i += 7) {
            const pending = flat.slice(i, i + 7);
            const dir = pending[0];
            const stream = pending[1];
            const src = pending[2];
            const dst = pending[3];
            const size = pending[4];
            const peri_addr = pending[5] || 0;
            const peripheral = pending[6] || 0;
            doneBits |= 1 << stream;
            try {
                if (dir === 2) {
                    const data = uc.mem_read(BigInt(src), size);
                    uc.mem_write(BigInt(dst), data);
                } else if (dir === 0) {
                    // periph -> mem (DmaDir::Read): pop bytes from peripheral, store in RAM
                    for (let j = 0; j < size; j += 4) {
                        const chunk = Math.min(4, size - j);
                        const val = periph_read(src, chunk);
                        const bytes = new Uint8Array(chunk);
                        for (let k = 0; k < chunk; k++) bytes[k] = (val >> (k * 8)) & 0xFF;
                        uc.mem_write(BigInt(dst + j), bytes);
                    }
                } else if (dir === 1) {
                    // mem -> periph (DmaDir::Write): read RAM, push bytes into peripheral
                    const data = uc.mem_read(BigInt(src), size);
                    for (let j = 0; j < size; j += 4) {
                        const chunk = Math.min(4, size - j);
                        let val = 0;
                        for (let k = 0; k < chunk; k++) val |= data[j + k] << (k * 8);
                        periph_write(dst, chunk, val);
                    }
                }
            } catch (e) {
                console.warn('DMA error:', e.message);
            }
        }
        if (doneBits) dma_set_completed_many(doneBits);
    };

    const processInterrupts = () => {
        for (let i = 0; i < 64; i++) {
            const irq = get_next_pending_interrupt();
            if (irq <= -100) return;

            const savedAt = uc.reg_read_i32(Module.ARM_REG_SP);
            const savedR0 = uc.reg_read_i32(Module.ARM_REG_R0);
            const savedR1 = uc.reg_read_i32(Module.ARM_REG_R1);
            const savedR2 = uc.reg_read_i32(Module.ARM_REG_R2);
            const savedR3 = uc.reg_read_i32(Module.ARM_REG_R3);
            const savedR12 = uc.reg_read_i32(Module.ARM_REG_R12);
            const savedLR = uc.reg_read_i32(Module.ARM_REG_LR);
            const savedPC = uc.reg_read_i32(Module.ARM_REG_PC);
            const savedXPSR = uc.reg_read_i32(Module.ARM_REG_XPSR);
            uc.reg_write_i32(Module.ARM_REG_SP, savedAt - 32);
            const handler_pc = read32(vector_table + 4 * (16 + irq));
            uc.reg_write_i32(Module.ARM_REG_LR, 0xFFFFFFF9);
            uc.reg_write_i32(Module.ARM_REG_PC, handler_pc);
            try {
                uc.emu_start(BigInt(handler_pc), 0n, 0n, 20000);
            } catch (e) {
                // Handler crashed on BX LR (EXC_RETURN not supported)
            }
            clear_current_interrupt();
            if (irq === 15) { while (nvic_systick_take()) { /* re-pended: more 1ms ticks this batch */ } }
            uc.reg_write_i32(Module.ARM_REG_R0, savedR0);
            uc.reg_write_i32(Module.ARM_REG_R1, savedR1);
            uc.reg_write_i32(Module.ARM_REG_R2, savedR2);
            uc.reg_write_i32(Module.ARM_REG_R3, savedR3);
            uc.reg_write_i32(Module.ARM_REG_R12, savedR12);
            uc.reg_write_i32(Module.ARM_REG_LR, savedLR);
            uc.reg_write_i32(Module.ARM_REG_PC, savedPC | 1);
            uc.reg_write_i32(Module.ARM_REG_SP, savedAt);
            processDma();
        }
    };

    const maxBatch = 20000;
    let totalSteps = 0;
    const startTime = Date.now();
    const traceResolve = makeResolver(fwSymbols);

    /* CAN RX injection: firmware sets canRxArmed=1, then waits for a frame */
    let canInjected = false;

    while (!stopRequested) {
        const dmaBusy = dma_get_all_pending().length > 0;
        while (stdinQueue.length > 0 && uart_rx_pending(uartAddr) === 0 && !dmaBusy) { const b = stdinQueue.shift(); uart_rx_byte(uartAddr, b); }

        processDma();
        const curPc = uc.reg_read_i32(Module.ARM_REG_PC);
        try {
            uc.emu_start(BigInt(curPc | 1), 0n, 0n, maxBatch);
        } catch (e) {
            const msg = String(e);
            const pc2 = uc.reg_read_i32(Module.ARM_REG_PC);
            if (msg.includes('UC_ERR_FETCH_UNMAPPED') && ((pc2 & ~1) >>> 0) >= 0xFFFFFFF0 && svcStack.length > 0) {
                // SVC handler returned via `bx lr` (EXC_RETURN): restore the pre-SVC context
                const st = svcStack.pop();
                uc.reg_write_i32(Module.ARM_REG_R0, st.r0);
                uc.reg_write_i32(Module.ARM_REG_R1, st.r1);
                uc.reg_write_i32(Module.ARM_REG_R2, st.r2);
                uc.reg_write_i32(Module.ARM_REG_R3, st.r3);
                uc.reg_write_i32(Module.ARM_REG_R12, st.r12);
                uc.reg_write_i32(Module.ARM_REG_LR, st.lr);
                uc.reg_write_i32(Module.ARM_REG_PC, st.pc | 1);
                uc.reg_write_i32(Module.ARM_REG_SP, st.sp);
            } else if (msg.includes('UC_ERR_READ_UNMAPPED') || msg.includes('UC_ERR_FETCH_UNMAPPED') || msg.includes('UC_ERR_WRITE_UNMAPPED')) {
                const r0 = uc.reg_read_i32(Module.ARM_REG_R0);
                const r1 = uc.reg_read_i32(Module.ARM_REG_R1);
                const lr = uc.reg_read_i32(Module.ARM_REG_LR);
                const sym = traceResolve ? (traceResolve(pc2) || pc2.toString(16)) : null;
                console.log(`FAULT @${sym} lr=${traceResolve ? (traceResolve(lr) || lr.toString(16)) : lr.toString(16)} r0=0x${r0.toString(16)} r1=0x${r1.toString(16)} [${msg}]`);
                if (traceResolve && sym && sym.includes('HAL_NVIC_EnableIRQ')) {
                    // Known Unicorn `bl` decode artifact at HAL_NVIC_EnableIRQ+0xf:
                    // not a real fault, skip the faulting instruction like before.
                    uc.reg_write_i32(Module.ARM_REG_PC, (pc2 + 2) | 1);
                } else if (traceResolve) {
                    // Real fault: raise it. The fault handler runs via
                    // processInterrupts below and CFSR/HFSR/BFAR are populated.
                    const kind = msg.includes('FETCH_UNMAPPED') ? 0 : (msg.includes('WRITE_UNMAPPED') ? 2 : 1);
                    raise_fault(kind, 0);
                } else {
                    // No symbol table: keep the legacy tolerant skip.
                    uc.reg_write_i32(Module.ARM_REG_PC, (pc2 + 2) | 1);
                }
            } else {
                console.error('Emulation error:', e.message || e);
                break;
            }
        }
        instCount += maxBatch;
        batchInstCount += maxBatch;
        if (batchInstCount > 0) {
            const status = step_batch(batchInstCount);
            batchInstCount = 0;
            if (status === 1) { stopRequested = true; break; }
        }
        processDma();
        try { processInterrupts(); } catch (irqErr) { console.error('processInterrupts error at step', totalSteps, ':', irqErr?.message || irqErr); break; }
        totalSteps++;

        if (canArmedSym && !canInjected) {
            const armedBytes = uc.mem_read(canArmedSym.addr, 4);
            const armed = armedBytes && armedBytes[0] !== 0;
            if (armed) {
                canInjected = can_inject_message(0x40006400, 0 << 21, 2, 0xDEAD, 0);
            }
        }

        try { if (stopRequested || is_watchdog_reset_requested()) break; } catch (wdErr) { console.error('WDT check error:', wdErr); break; }
        if (instCount >= maxInst) break;
        await new Promise(r => setImmediate(r));
    }

    const elapsed = ((Date.now() - startTime) / 1000).toFixed(2);
    const finalPc = uc.reg_read_i32(Module.ARM_REG_PC);
    const finalSp = uc.reg_read_i32(Module.ARM_REG_SP);

    const uartOut = get_uart_output();
    if (uartOut) {
        console.log(`\n=== UART Output ===\n${uartOut}`);
    }

    try { if (process.stdin.isTTY) process.stdin.setRawMode(false); } catch (_) {}

    console.log(`\nDone: ${totalSteps} steps, ${instCount} instructions in ${elapsed}s`);

    const resolve = makeResolver(fwSymbols);
    const pcName = resolve && resolve(finalPc);
    const spName = resolve && resolve(finalSp);
    console.log(`PC=0x${finalPc.toString(16)}${pcName ? `  → ${pcName}` : ''} SP=0x${finalSp.toString(16)}${spName ? `  → ${spName}` : ''}`);

    if (showRegs) {
        for (let i = 0; i <= 12; i++) {
            const reg = uc[`reg_read_i32`](Module[`ARM_REG_R${i}`]);
            process.stdout.write(`R${i}=0x${reg.toString(16).padStart(8, '0')} `);
            if (i % 4 === 3) console.log();
        }
        console.log(`LR=0x${uc.reg_read_i32(Module.ARM_REG_LR).toString(16).padStart(8, '0')}`);
        console.log(`xPSR=0x${uc.reg_read_i32(Module.ARM_REG_XPSR).toString(16).padStart(8, '0')}`);
    }

    uc.close();
}

main().catch(e => {
    console.error('Fatal:', e?.name || '(no name)', e?.message || '(no message)', e?.code || '');
    console.error('Stack:', e?.stack?.substring(0, 1000) || '(no stack)');
    console.error('Type:', typeof e, e);
    process.exit(1);
});
