const PERIPH_RANGES = [
    [0x40000000, 0xB0000000],
    [0xE0000000, 0xE1000000],
];

const DEFAULT_MAX_BATCH = 100000;

/**
 * Load the Rust peripheral WASM module.
 * - Node.js: import the wasm-pack glue directly (ESM wasm import works there)
 * - Browser: fetch + WebAssembly.instantiate (MIME-agnostic, works on GitHub Pages
 *   and any static host without strict module-MIME support)
 */
let periphPromise;
function getPeriph() {
    if (!periphPromise) {
        if (typeof process !== 'undefined' && process.versions?.node) {
            periphPromise = import('./stm32_bluepill_wasm.js');
        } else {
            periphPromise = (async () => {
                const wbg = await import('./stm32_bluepill_wasm_bg.js');
                const resp = await fetch(new URL('./stm32_bluepill_wasm_bg.wasm', import.meta.url));
                const bytes = await resp.arrayBuffer();
                const mod = new WebAssembly.Module(bytes);
                const imports = {};
                for (const { module, name } of WebAssembly.Module.imports(mod)) {
                    (imports[module] ??= {})[name] = wbg[name] ?? (() => {});
                }
                const { instance } = await WebAssembly.instantiate(bytes, imports);
                wbg.__wbg_set_wasm(instance.exports);
                if (typeof instance.exports.__wbindgen_start === 'function') {
                    instance.exports.__wbindgen_start();
                }
                return wbg;
            })();
        }
    }
    return periphPromise;
}

function getMUnicorn() {
    const browserGlobal = () => (typeof window !== 'undefined' && window.MUnicorn) ? window.MUnicorn : null;
    // Node.js: prefer the CJS Emscripten glue, fall back to a browser-global shim
    if (typeof process !== 'undefined' && process.versions?.node) {
        return import('module').then(({ createRequire }) => {
            const require = createRequire(import.meta.url);
            try {
                const m = require('./unicorn_arm.cjs');
                return m || browserGlobal();
            } catch (e) {
                return browserGlobal();
            }
        }).then((m) => {
            if (!m) throw new Error('unicorn_arm module not found (missing unicorn_arm.cjs).');
            return m;
        });
    }
    // Browser: unicorn_arm.js loaded via <script src="unicorn_arm.js"></script>
    return Promise.resolve(browserGlobal())
        .then((m) => {
            if (!m) throw new Error('unicorn_arm module not found. Add <script src="unicorn_arm.js"></script> before this module.');
            return m;
        });
}

function parseHex(v) { return typeof v === 'number' ? v : parseInt(v, 16); }

/**
 * Parse an Intel HEX (ihex) text blob into a byte array.
 *
 * @param {string} text Intel HEX records (record types 00/01/02/04)
 * @returns {{data: Uint8Array, base: number}} Bytes + lowest address they belong to
 */
export function parseIntelHex(text) {
    let minAddr = Infinity, maxAddr = 0;
    let base = 0;
    const recs = [];
    for (const raw of String(text).split(/\r?\n/)) {
        const line = raw.trim();
        if (!line || line[0] !== ':') continue;
        const hex = line.slice(1);
        if (hex.length < 10) continue;
        const count = parseInt(hex.slice(0, 2), 16);
        const addr = parseInt(hex.slice(2, 6), 16);
        const type = parseInt(hex.slice(6, 8), 16);
        if (hex.length < 8 + count * 2) continue;
        const data = new Uint8Array(count);
        for (let i = 0; i < count; i++) data[i] = parseInt(hex.slice(8 + i * 2, 10 + i * 2), 16);
        if (type === 0x04) base = ((data[0] << 8) | data[1]) << 16;
        else if (type === 0x02) base = ((data[0] << 8) | data[1]) << 4;
        else if (type === 0x00) recs.push([base + addr, data]);
        else if (type === 0x01) break;
    }
    if (!recs.length) return { data: new Uint8Array(0), base: 0 };
    for (const [a, d] of recs) {
        minAddr = Math.min(minAddr, a);
        maxAddr = Math.max(maxAddr, a + d.length);
    }
    const out = new Uint8Array(maxAddr - minAddr);
    for (const [a, d] of recs) out.set(d, a - minAddr);
    return { data: out, base: minAddr };
}

/**
 * Create a full STM32F103C8 (Bluepill) emulator instance.
 *
 * @param {object} opts
 * @param {Uint8Array|string} opts.firmware     Firmware to load at flash base: raw binary
 *                                              (Uint8Array) or Intel HEX text (string,
 *                                              auto-detected if bytes start with ':')
 * @param {number}    [opts.flash_size=0x10000] Flash region size (64KB default)
 * @param {number}    [opts.ram_size=0x5000]    SRAM size (20KB default)
 * @param {number}    [opts.vector_table=0x08000000] Vector table base address
 * @param {string}    [opts.svd]                SVD XML string (optional; defaults to hardcoded map)
 * @param {number}    [opts.uart_addr=0x40013800] USART used for uartRx()
 * @param {object}    [opts.ext_devices={}]     External devices (see below)
 * @param {boolean}   [opts.verbose=false]      Print init info to console
 *
 * ext_devices shape:
 *   { spi_flash: [{peripheral, jedec_id, data, cs?}],
 *     i2c_eeprom: [{peripheral, address, data}],
 *     i2c_oled:   [{peripheral, address, width, height}],
 *     lcd:        [{peripheral, cs}],
 *     touchscreen:[{peripheral, touch_detected_pin, cs}],
 *     software_spi:[{name, cs, clk, miso, mosi}] }
 *
 * @returns {Promise<BluepillEmulator>}
 */
export async function createEmulator(opts = {}) {
    const {
        firmware = new Uint8Array(0),
        flash_size = 0x10000,
        ram_size = 0x5000,
        vector_table = 0x08000000,
        svd = null,
        uart_addr = 0x40013800,
        ext_devices = {},
        verbose = false,
    } = opts;

    const MUnicorn = await getMUnicorn();
    const Module = await MUnicorn({});
    const periph = await getPeriph();

    const { periph_read, periph_write, tick, step_batch, get_next_pending_interrupt,
    dma_get_pending_count, dma_get_pending, dma_set_completed, is_watchdog_reset_requested,
    add_spi_flash, add_i2c_eeprom, add_touchscreen, add_lcd, add_i2c_oled, add_software_spi,
    init, init_svd, has_pending_interrupt, get_uart_output, uart_rx_byte, gpio_read_output,
    gpio_set_input, gpio_read_input, set_intr_masks, clear_current_interrupt,
    can_inject_message, adc_set_sim_value, touchscreen_set_touch } = periph;

    // Register external devices BEFORE init()
    for (const d of ext_devices.spi_flash || []) {
        add_spi_flash(d.peripheral, parseHex(d.jedec_id), d.data ?? new Uint8Array(0), d.cs ?? null);
    }
    for (const d of ext_devices.i2c_eeprom || []) {
        add_i2c_eeprom(d.peripheral, parseHex(d.address), d.data ?? new Uint8Array(0));
    }
    for (const d of ext_devices.i2c_oled || []) {
        add_i2c_oled(d.peripheral, parseHex(d.address ?? '0x3C'), parseHex(d.width ?? '128'), parseHex(d.height ?? '64'));
    }
    for (const d of ext_devices.lcd || []) {
        add_lcd(d.peripheral, d.cs ?? null);
    }
    for (const d of ext_devices.touchscreen || []) {
        add_touchscreen(d.peripheral, d.touch_detected_pin ?? null, d.cs ?? null);
    }
    for (const d of ext_devices.software_spi || []) {
        add_software_spi(d.name, d.cs ?? null, d.clk, d.miso, d.mosi);
    }

    if (svd) {
        init_svd(svd);
    } else {
        init();
    }

    const uc = new Module.Unicorn(
        Module.ARCH_ARM,
        Module.MODE_THUMB | Module.MODE_LITTLE_ENDIAN
    );

    const flash_addr = vector_table & ~0x1FFFF;
    uc.mem_map(flash_addr, flash_size, Module.PROT_ALL);
    let fwBytes = firmware;
    let fwAddr = flash_addr;
    if (typeof firmware === 'string' || (firmware instanceof Uint8Array && firmware.length > 0 && firmware[0] === 0x3A)) {
        const text = typeof firmware === 'string' ? firmware : new TextDecoder().decode(firmware);
        const parsed = parseIntelHex(text);
        fwBytes = parsed.data;
        if (parsed.base >= flash_addr && parsed.base < flash_addr + flash_size) fwAddr = parsed.base;
    }
    if (fwBytes.length > 0) uc.mem_write(BigInt(fwAddr), fwBytes);

    uc.mem_map(0x20000000, ram_size, Module.PROT_ALL);

    for (const [start, end] of PERIPH_RANGES) {
        uc.mem_map(start, end - start, Module.PROT_READ | Module.PROT_WRITE);
    }

    const read32 = (addr) => {
        const b = uc.mem_read(BigInt(addr), 4);
        return new DataView(b.buffer, b.byteOffset, b.byteLength).getUint32(0, true);
    };
    const write32 = (addr, val) => {
        const b = new Uint8Array(4);
        new DataView(b.buffer).setUint32(0, val >>> 0, true);
        uc.mem_write(BigInt(addr), b);
    };

    const sp_init = read32(vector_table);
    const pc_init = read32(vector_table + 4);
    uc.reg_write_i32(Module.ARM_REG_SP, sp_init);
    uc.reg_write_i32(Module.ARM_REG_PC, pc_init | 1);

    if (verbose) {
        console.log(`SP=0x${sp_init.toString(16)} PC=0x${(pc_init | 1).toString(16)}`);
    }

    const memReadHook = (handle, type, address, size, value, user_data) => {
        const addr32 = Number(address);
        const val = periph_read(addr32, size) >>> 0;
        const bytes = new Uint8Array(size);
        for (let i = 0; i < size; i++) bytes[i] = (val >> (i * 8)) & 0xFF;
        uc.mem_write(address, bytes);
    };

    const memWriteHook = (handle, type, address, size, value, user_data) => {
        periph_write(Number(address), size, Number(value));
    };

    for (const [start, end] of PERIPH_RANGES) {
        uc.hook_add(Module.HOOK_MEM_READ, memReadHook, null, start, end);
        uc.hook_add(Module.HOOK_MEM_WRITE, memWriteHook, null, start, end);
    }

    let stopRequested = false;
    let instCount = 0n;
    let batchInstCount = 0n;

    const codeHook = () => { instCount++; batchInstCount++; };
    uc.hook_add(Module.HOOK_CODE, codeHook, null);

    const intrHook = (handle, intno, user_data) => {
        if (intno === 8) {
            // BX LR with EXC_RETURN: pop the interrupt frame
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
                // ignore per-transfer errors
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
                uc.emu_start(BigInt(handler_pc), 0n, 0n, DEFAULT_MAX_BATCH);
            } catch (e) { /* BX LR EXC_RETURN handled by intrHook */ }
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

    return {
        uc, Module, read32, write32,

        /** Run up to maxInstructions (0 = forever). Returns {totalSteps, instCount, stopped}. */
        run(maxInstructions = 0) {
            stopRequested = false;
            let totalSteps = 0;
            while (!stopRequested) {
                processDma();
                const curPc = uc.reg_read_i32(Module.ARM_REG_PC);
                try {
                    uc.emu_start(BigInt(curPc | 1), 0n, 0n, DEFAULT_MAX_BATCH);
                } catch (e) {
                    const msg = String(e);
                    if (msg.includes('UC_ERR_READ_UNMAPPED') || msg.includes('UC_ERR_FETCH_UNMAPPED')) {
                        const pc2 = uc.reg_read_i32(Module.ARM_REG_PC);
                        uc.reg_write_i32(Module.ARM_REG_PC, (pc2 + 2) | 1);
                    } else {
                        throw e;
                    }
                }
                if (batchInstCount > 0) {
                    const status = step_batch(Number(batchInstCount));
                    batchInstCount = 0n;
                    if (status === 1) { stopRequested = true; break; }
                }
                processDma();
                processInterrupts();
                totalSteps++;
                if (is_watchdog_reset_requested()) break;
                if (maxInstructions > 0 && instCount >= BigInt(maxInstructions)) break;
            }
            return {
                totalSteps,
                instCount: Number(instCount),
                stopped: stopRequested || is_watchdog_reset_requested(),
            };
        },

        /** Run one batch (default 100K instructions) and return after processing DMA/interrupts. */
        step(maxBatch = DEFAULT_MAX_BATCH) {
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
                    throw e;
                }
            }
            if (batchInstCount > 0) {
                const status = step_batch(Number(batchInstCount));
                batchInstCount = 0n;
                if (status === 1) stopRequested = true;
            }
            processDma();
            processInterrupts();
            return {
                pc: uc.reg_read_i32(Module.ARM_REG_PC),
                instCount: Number(instCount),
                stopped: stopRequested || is_watchdog_reset_requested(),
            };
        },

        stop() {
            stopRequested = true;
            try { uc.emu_stop(); } catch (e) { /* ignore */ }
        },

        getRegisters() {
            const regs = {};
            for (let i = 0; i <= 12; i++) regs[`R${i}`] = uc[`reg_read_i32`](Module[`ARM_REG_R${i}`]);
            regs.SP = uc.reg_read_i32(Module.ARM_REG_SP);
            regs.LR = uc.reg_read_i32(Module.ARM_REG_LR);
            regs.PC = uc.reg_read_i32(Module.ARM_REG_PC);
            regs.xPSR = uc.reg_read_i32(Module.ARM_REG_XPSR);
            return regs;
        },

        getPc() { return uc.reg_read_i32(Module.ARM_REG_PC); },
        getSp() { return uc.reg_read_i32(Module.ARM_REG_SP); },
        setPc(pc) { uc.reg_write_i32(Module.ARM_REG_PC, pc | 1); },

        getUartOutput() { return get_uart_output(); },

        /** Inject a byte into the UART RX (default: USART1 @ 0x40013800). */
        uartRx(byte) { return uart_rx_byte(uart_addr, byte); },
        uartRxBytes(bytes) {
            let ok = false;
            for (const b of bytes) ok = uart_rx_byte(uart_addr, b) || ok;
            return ok;
        },

        canInjectMessage(addr, tir, tdtr, tdlr, tdhr) {
            return can_inject_message(addr, tir, tdtr, tdlr, tdhr);
        },

        gpioReadOutput(port, pin) { return gpio_read_output(port, pin); },
        gpioReadInput(port, pin) { return gpio_read_input(port, pin); },
        gpioSetInput(port, pin, value) { gpio_set_input(port, pin, value); },

        setSimAdc(value) { adc_set_sim_value(value); },
        setTouch(peripheral, x, y, pressure) { touchscreen_set_touch(peripheral, x, y, pressure); },

        /** Low-level register access (width: 1, 2, or 4). */
        periphRead(addr, width = 4) { return periph_read(addr, width) >>> 0; },
        periphWrite(addr, width, value) { periph_write(addr, width, value); },

        tick() { tick(); },
        stepBatch(count) { return step_batch(count); },
        hasPendingInterrupt() { return has_pending_interrupt(); },
        getNextPendingInterrupt() { return get_next_pending_interrupt(); },
        setIntrMasks(primask, basepri) { set_intr_masks(primask, basepri); },

        close() {
            try { uc.close(); } catch (e) { /* ignore */ }
        },
    };
}

export default createEmulator;
