import { readFileSync } from 'fs';
import path from 'path';
import * as yaml from 'js-yaml';
import * as periph from './stm32_bluepill_wasm.js';
const { periph_read, periph_write, tick, get_next_pending_interrupt, dma_get_pending_count, 
dma_get_pending, dma_set_completed, is_watchdog_reset_requested, add_spi_flash, add_i2c_eeprom, add_touchscreen,
add_lcd, add_i2c_oled,
init, init_svd, has_pending_interrupt, get_uart_output, uart_rx_byte, gpio_read_output, initSync,
set_intr_masks, clear_current_interrupt } = periph;

const parseHex = (v) => typeof v === 'number' ? v : parseInt(v, 16);

async function getMUnicorn() {
    const { createRequire } = await import('module');
    const require = createRequire(import.meta.url);
    return require('./unicorn_arm.cjs');
}

async function main() {
    const wasmBytes = readFileSync(new URL('stm32_bluepill_wasm_bg.wasm', import.meta.url));
    initSync({ module: wasmBytes });
    const args = process.argv.slice(2);
    const configPaths = args.filter(a => a.startsWith('--config=')).map(a => a.split('=')[1]);
    const posArgs = args.filter(a => !a.startsWith('--'));
    const maxInst = parseInt(
        args.find(a => a.startsWith('--max='))?.split('=')[1]
        || (configPaths.length > 0 ? posArgs[0] : posArgs[1])
        || process.env.MAX_INST || '1000000', 10);
    const showRegs = args.includes('--regs') || process.env.SHOW_REGS === '1';
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
    let vector_table;
    let memRegions;

    if (config.regions) {
        memRegions = config.regions.map(r => ({ ...r, start: parseHex(r.start), size: parseHex(r.size) }));
        const romRegion = memRegions.find(r => r.load);
        if (!romRegion) { console.error('No region with load file found'); process.exit(1); }
        vector_table = parseHex(config.cpu?.vector_table || romRegion.start);
        const romFile = path.resolve(romRegion._dir || config._devices_dir, romRegion.load);
        firmware = readFileSync(romFile);
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
                        add_spi_flash(d.peripheral, parseHex(d.jedec_id), data, null);
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
            console.error('Usage: node cli.mjs <firmware.bin> [max_instructions] [--config=path] [--max=N]');
            console.error('  or set FIRMWARE env var');
            process.exit(1);
        }
        firmware = readFileSync(firmwarePath);
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
    const romRegion = memRegions.find(r => firmware && (r._firmware || r.load || (r.start <= vector_table && r.start + r.size > vector_table)));
    const romStart = romRegion ? romRegion.start : (memRegions[0]?.start || 0x08000000);
    if (firmware) uc.mem_write(BigInt(romStart), firmware);

    if (romStart !== vector_table) uc.mem_write(BigInt(vector_table), firmware);

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
        const val = periph_read(addr32, size) >>> 0;
        const bytes = new Uint8Array(size);
        for (let i = 0; i < size; i++) {
            bytes[i] = (val >> (i * 8)) & 0xFF;
        }
        uc.mem_write(address, bytes);
    };

    const memWriteHook = (handle, type, address, size, value, user_data) => {
        periph_write(Number(address), size, Number(value));
    };

    for (const [start, end] of periphRanges) {
        uc.hook_add(Module.HOOK_MEM_READ, memReadHook, null, start, end);
        uc.hook_add(Module.HOOK_MEM_WRITE, memWriteHook, null, start, end);
    }

    let instCount = 0n;
    let stopRequested = false;

    const readIntrMask = (reg) => {
        try { return uc.reg_read_i32(Module[reg]) >>> 0; } catch (_) { return 0; }
    };

    const codeHook = (handle, address, size, user_data) => {
        instCount++;
        tick();
        set_intr_masks(readIntrMask('ARM_REG_PRIMASK'), readIntrMask('ARM_REG_BASEPRI'));
        if (is_watchdog_reset_requested()) {
            stopRequested = true;
            uc.emu_stop();
            return;
        }
        if (dma_get_pending_count() > 0) {
            uc.emu_stop();
            return;
        }
        if (has_pending_interrupt()) {
            uc.emu_stop();
        }
    };
    uc.hook_add(Module.HOOK_CODE, codeHook, null);

    const stdinQueue = [];
    if (process.stdin.isTTY) process.stdin.setRawMode(true);
    process.stdin.on('data', (chunk) => { for (const b of chunk) stdinQueue.push(b); });
    process.stdin.resume();
    if (process.stdin.isTTY) process.on('SIGINT', () => { process.stdin.setRawMode(false); process.exit(0); });

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
        }
    };
    uc.hook_add(Module.HOOK_INTR, intrHook, null);

    const processDma = () => {
        const count = dma_get_pending_count();
        for (let i = 0; i < count; i++) {
            const pending = dma_get_pending(0);
            if (pending.length < 5) continue;
            const dir = pending[0];
            const stream = pending[1];
            const src = pending[2];
            const dst = pending[3];
            const size = pending[4];
            const peri_addr = pending[5] || 0;
            const peripheral = pending[6] || 0;
            try {
                if (dir === 2) {
                    const data = uc.mem_read(BigInt(src), size);
                    uc.mem_write(BigInt(dst), data);
                } else if (dir === 0) {
                    const data = uc.mem_read(BigInt(src), size);
                    if (peripheral) {
                        for (let j = 0; j < size; j += 4) {
                            const chunk = Math.min(4, size - j);
                            let val = 0;
                            for (let k = 0; k < chunk; k++) val |= data[j + k] << (k * 8);
                            periph_write(peri_addr, chunk, val);
                        }
                    } else {
                        uc.mem_write(BigInt(dst), data);
                    }
                } else if (dir === 1) {
                    if (peripheral) {
                        for (let j = 0; j < size; j += 4) {
                            const chunk = Math.min(4, size - j);
                            const val = periph_read(peri_addr, chunk);
                            const bytes = new Uint8Array(chunk);
                            for (let k = 0; k < chunk; k++) bytes[k] = (val >> (k * 8)) & 0xFF;
                            uc.mem_write(BigInt(dst + j), bytes);
                        }
                    } else {
                        const data = uc.mem_read(BigInt(src), size);
                        uc.mem_write(BigInt(dst), data);
                    }
                }
            } catch (e) {
                console.warn('DMA error:', e.message);
            }
            dma_set_completed(stream, true);
        }
    };

    const processInterrupts = () => {
        while (!stopRequested) {
            const irq = get_next_pending_interrupt();
            if (irq <= -100) break;

            const savedAt = uc.reg_read_i32(Module.ARM_REG_SP);
            const pc = uc.reg_read_i32(Module.ARM_REG_PC);
            const lr = uc.reg_read_i32(Module.ARM_REG_LR);
            const xpsr = uc.reg_read_i32(Module.ARM_REG_XPSR);
            const r0 = uc.reg_read_i32(Module.ARM_REG_R0);
            const r1 = uc.reg_read_i32(Module.ARM_REG_R1);
            const r2 = uc.reg_read_i32(Module.ARM_REG_R2);
            const r3 = uc.reg_read_i32(Module.ARM_REG_R3);
            const r12 = uc.reg_read_i32(Module.ARM_REG_R12);
            const frame = new Uint8Array(32);
            const sv = new DataView(frame.buffer);
            sv.setUint32(0, xpsr, true);
            sv.setUint32(4, pc, true);
            sv.setUint32(8, lr, true);
            sv.setUint32(12, r12, true);
            sv.setUint32(16, r3, true);
            sv.setUint32(20, r2, true);
            sv.setUint32(24, r1, true);
            sv.setUint32(28, r0, true);
            uc.mem_write(BigInt(savedAt - 32), frame);
            uc.reg_write_i32(Module.ARM_REG_SP, savedAt - 32);
            const handler_pc = read32(vector_table + 4 * (16 + irq));
            uc.reg_write_i32(Module.ARM_REG_LR, 0xFFFFFFF9);
            uc.reg_write_i32(Module.ARM_REG_PC, handler_pc);
            try {
                uc.emu_start(BigInt(handler_pc), 0n, 0n, 100000);
            } catch (e) {
                // Handler crashed on BX LR (EXC_RETURN not supported)
            }
            clear_current_interrupt();
            const savedFrame = uc.mem_read(BigInt(savedAt - 32), 32);
            const savedSv = new DataView(savedFrame.buffer, savedFrame.byteOffset, savedFrame.byteLength);
            uc.reg_write_i32(Module.ARM_REG_R0, savedSv.getUint32(28, true));
            uc.reg_write_i32(Module.ARM_REG_R1, savedSv.getUint32(24, true));
            uc.reg_write_i32(Module.ARM_REG_R2, savedSv.getUint32(20, true));
            uc.reg_write_i32(Module.ARM_REG_R3, savedSv.getUint32(16, true));
            uc.reg_write_i32(Module.ARM_REG_R12, savedSv.getUint32(12, true));
            uc.reg_write_i32(Module.ARM_REG_LR, savedSv.getUint32(8, true));
            uc.reg_write_i32(Module.ARM_REG_PC, savedSv.getUint32(4, true) | 1);
            uc.reg_write_i32(Module.ARM_REG_SP, savedAt);
            processDma();
        }
    };

    const maxBatch = 100000;
    let totalSteps = 0;
    const startTime = Date.now();

    while (!stopRequested) {
        while (stdinQueue.length > 0) { const b = stdinQueue.shift(); uart_rx_byte(uartAddr, b); }

        processDma();
        const curPc = uc.reg_read_i32(Module.ARM_REG_PC);
        try {
            uc.emu_start(BigInt(curPc | 1), 0n, 0n, maxBatch);
        } catch (e) {
            const msg = String(e);
            if (msg.includes('UC_ERR_READ_UNMAPPED') || msg.includes('UC_ERR_FETCH_UNMAPPED')) {
                const pc2 = uc.reg_read_i32(Module.ARM_REG_PC);
                uc.reg_write_i32(Module.ARM_REG_PC, (pc2 + 2) | 1);
            } else {
                console.error('Emulation error:', e.message || e);
                break;
            }
        }
        processDma();
        processInterrupts();
        totalSteps++;

        if (stopRequested || is_watchdog_reset_requested()) break;
        if (instCount >= BigInt(maxInst)) break;
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
    console.log(`PC=0x${finalPc.toString(16)} SP=0x${finalSp.toString(16)}`);

    // Verify peripheral state
    const gpioC_BSRR = periph_read(0x40011010, 4);
    const gpioC_ODR = periph_read(0x4001100c, 4);
    const gpioC_CRH = periph_read(0x40011004, 4);
    const rcc_APB2ENR = periph_read(0x40021018, 4);
    const pc13_out = gpio_read_output(2, 13);
    console.log(`RCC_APB2ENR=0x${rcc_APB2ENR.toString(16)}`);
    console.log(`GPIOC_CRH=0x${gpioC_CRH.toString(16)}`);
    console.log(`GPIOC_BSRR=0x${gpioC_BSRR.toString(16)}`);
    console.log(`GPIOC_ODR=0x${gpioC_ODR.toString(16)}`);
    console.log(`PC13 output=${pc13_out}`);

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
    console.error('Fatal:', e.name, e.message);
    console.error('Stack:', e.stack?.substring(0, 1000));
    process.exit(1);
});
