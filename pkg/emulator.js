const PERIPH_RANGES = [
    [0x40000000, 0xB0000000],
    [0xE0000000, 0xE1000000],
];

const DEFAULT_MAX_BATCH = 20000;
const LARGE_BATCH = 50000;
// Poll-aware shrinking (see run()): a tight `while(!(REG & FLAG))` spin shows
// up as many consecutive memReadHook hits on one address. Peripheral flags
// refresh only between batches, so a spin wastes ~B/2 instructions per awaited
// event at batch size B — shrink to POLL_BATCH while polling. Sustained
// polling means an external wait (UART RX/CAN from outside: smaller batches
// can't hurry those), so back off to normal batches after POLL_BACKOFF_AFTER.
const POLL_BATCH = 5000;
const POLL_THRESHOLD = 8;
const POLL_BACKOFF_AFTER = 8;

/**
 * Load the Rust peripheral WASM module.
 * - Node.js: import the wasm-pack glue directly (ESM wasm import works there)
 * - Browser: fetch + WebAssembly.instantiate (MIME-agnostic, works on GitHub Pages
 *   and any static host without strict module-MIME support)
 */
let periphPromise;
function getPeriph() {
    if (!periphPromise) {
        // The high-level glue loads the wasm itself: initSync(module) in Node,
        // or default() which fetches stm32_bluepill_wasm_bg.wasm relative to the
        // module URL in the browser (no static wasm import -> works on GitHub Pages).
        periphPromise = import('./stm32_bluepill_wasm.js').then(async (p) => {
            if (typeof process !== 'undefined' && process.versions?.node) {
                const { readFileSync } = await import('fs');
                p.initSync({ module: readFileSync(new URL('./stm32_bluepill_wasm_bg.wasm', import.meta.url)) });
            } else {
                await p.default();
            }
            return p;
        });
    }
    return periphPromise;
}

function getMUnicorn() {
    const browserGlobal = () => (typeof globalThis !== 'undefined' && globalThis.MUnicorn) ? globalThis.MUnicorn : null;
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
 * Parse a GNU ld linker map file into symbol entries.
 *
 * @param {string} text GNU ld .map output
 * @returns {Array<{name: string, addr: number}>}
 */
export function parseSymbolMap(text) {
    const syms = [];
    const re = /^\s*0x([0-9a-fA-F]{8,16})\s+([A-Za-z_.$][\w.$]*)(?:\s*=|\s*$)/gm;
    let m;
    while ((m = re.exec(text)) !== null) {
        if (m[2] === '.') continue; // ld location counter, not a real symbol
        syms.push({ name: m[2], addr: parseInt(m[1], 16) });
    }
    return syms;
}

/**
 * Parse an ELF32 executable (ARM, little-endian) into loadable regions + symbols.
 *
 * @param {Uint8Array|ArrayBuffer} buffer ELF file bytes
 * @returns {{regions: Array<{start: number, data: Uint8Array}>, symbols: Array<{name: string, addr: number}>}}
 */
export function parseElf(buffer) {
    const b = buffer instanceof ArrayBuffer ? new Uint8Array(buffer) : buffer;
    if (b.length < 52 || b[0] !== 0x7F || b[1] !== 0x45 || b[2] !== 0x4C || b[3] !== 0x46) {
        throw new Error('Not an ELF file');
    }
    const dv = new DataView(b.buffer, b.byteOffset, b.byteLength);
    const u32 = (off) => dv.getUint32(off, true);
    const u16 = (off) => dv.getUint16(off, true);
    const e_phoff = u32(28), e_phentsize = u16(42), e_phnum = u16(44);
    const regions = [];
    for (let i = 0; i < e_phnum; i++) {
        const off = e_phoff + i * e_phentsize;
        if (off + 32 > b.length) break;
        const p_type = u32(off), p_offset = u32(off + 4), p_vaddr = u32(off + 8), p_paddr = u32(off + 12), p_filesz = u32(off + 16);
        if (p_type !== 1 || p_filesz === 0) continue;
        const data = b.slice(p_offset, p_offset + p_filesz);
        regions.push({ start: p_vaddr >>> 0, data });
        // The firmware startup copies .data from its load (LMA) address; the
        // emulator must provide that copy too, not just the VMA.
        if (p_paddr !== p_vaddr) {
            regions.push({ start: p_paddr >>> 0, data });
        }
    }
    const e_shoff = u32(32), e_shentsize = u16(46), e_shnum = u16(48);
    const sections = [];
    for (let i = 0; i < e_shnum; i++) {
        const off = e_shoff + i * e_shentsize;
        if (off + 40 > b.length) break;
        sections.push({ type: u32(off + 4), offset: u32(off + 16), size: u32(off + 20), link: u32(off + 24) });
    }
    const symbols = [];
    for (const sh of sections) {
        if (sh.type !== 2 || sh.size === 0) continue; // SHT_SYMTAB
        const strOff = sections[sh.link] ? sections[sh.link].offset : 0;
        const count = Math.min(Math.floor(sh.size / 16), Math.floor((b.length - sh.offset) / 16));
        for (let i = 0; i < count; i++) {
            const o = sh.offset + i * 16;
            const st_name = u32(o), st_value = u32(o + 4), st_info = b[o + 12];
            if (st_value === 0 || st_name === 0) continue;
            const type = st_info & 0xF;
            if (type !== 1 && type !== 2) continue; // OBJECT or FUNC
            let name = '';
            let p = strOff + st_name;
            while (p < b.length && b[p] !== 0) name += String.fromCharCode(b[p++]);
            if (name) symbols.push({ name, addr: st_value >>> 0 });
        }
    }
    return { regions, symbols };
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
 * @param {string}    [opts.svd]                SVD XML string (optional; defaults to hardcoded F103C8 map)
 * @param {string|object} [opts.chip]           'stm32f103c8' (builtin hardcoded map, default) or
 *                                              { name, svd } to build the peripheral map from an SVD
 *                                              (any F1-family chip, e.g. STM32F105 with CAN2)
 * @param {Array}     [opts.js_peripherals=[]]  rp2040js-style custom peripherals:
 *                                              [{ base, size, read(addr,size), write(addr,value,size) }]
 * @param {number}    [opts.uart_addr=0x40013800] USART used for uartRx()
 * @param {string}    [opts.cpu='unicorn']        CPU backend: 'unicorn' (TCG,
 *                                              default) or 'rust' (native Path B
 *                                              interpreter in WASM — benchmarking
 *                                              hook; same peripherals/DMA/IRQ.
 *                                              Skips Unicorn, mrs/i2c patches;
 *                                              no hook poll-shrink on this path)
 * @param {object}    [opts.ext_devices={}]     External devices (see below)
 * @param {boolean}   [opts.verbose=false]      Print init info to console
 *
 * ext_devices shape:
 *   { spi_flash: [{peripheral, jedec_id, data, cs?}],
 *     i2c_eeprom: [{peripheral, address, data}],
 *     sd_card:   [{peripheral, data}],
 *     i2c_oled:   [{peripheral, address, width, height}],
 *     lcd:        [{peripheral, cs}],
 *     touchscreen:[{peripheral, touch_detected_pin, cs}],
 *     software_spi:[{name, cs, clk, miso, mosi}] }
 *
 * Page-side peripheral drivers (7-seg, buzzer, ...) are pure JS: subscribe with
 * emu.onPeriphWrite(...) to tap the peripheral bus like real hardware, and poll
 * gpioReadOutput/pwmDuty per frame. The WASM stays a faithful STM32 core plus
 * chip models that must respond on the bus (eeprom/flash/oled/lcd/touchscreen).
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
        chip = 'stm32f103c8',
        js_peripherals = [],
        uart_addr = 0x40013800,
        ext_devices = {},
        verbose = false,
        batch_size = DEFAULT_MAX_BATCH,
        cpu = 'unicorn',
    } = opts;
    const maxBatch = batch_size;
    const cpuBackend = String(cpu || 'unicorn').toLowerCase();
    if (cpuBackend !== 'unicorn' && cpuBackend !== 'rust') {
        throw new Error(`createEmulator: unknown cpu backend "${cpu}" (expected 'unicorn' or 'rust')`);
    }
    const useRust = cpuBackend === 'rust';

    const MUnicorn = useRust ? null : await getMUnicorn();
    const Module = MUnicorn ? await MUnicorn({}) : null;
    const periph = await getPeriph();

    // Pool the 3 mallocs for regsRead/regsWrite — up to 64 IRQs/batch ×2 calls each
    // used to do 3×malloc+free per IRQ (6×64=384 allocs/40M). Pool once, reuse.
    const REG_POOL = 16;
    const regIdsPtr = Module ? Module._malloc(REG_POOL * 4) : 0;
    const regValsPtr = Module ? Module._malloc(REG_POOL * 4) : 0;
    const regPtrsPtr = Module ? Module._malloc(REG_POOL * 4) : 0;
    const regsRead = (uc, regIds) => {
        const n = regIds.length;
        const handle = Module.getValue(uc.handle_ptr, '*');
        for (let i = 0; i < n; i++) {
            Module.setValue(regIdsPtr + i * 4, regIds[i], 'i32');
            Module.setValue(regPtrsPtr + i * 4, regValsPtr + i * 4, 'i32');
        }
        Module.ccall('uc_reg_read_batch', 'number', ['number', 'number', 'number', 'number'], [handle, regIdsPtr, regPtrsPtr, n]);
        const out = new Array(n);
        for (let i = 0; i < n; i++) out[i] = Module.getValue(regValsPtr + i * 4, 'i32');
        return out;
    };
    const regsWrite = (uc, regIds, values) => {
        const n = regIds.length;
        const handle = Module.getValue(uc.handle_ptr, '*');
        for (let i = 0; i < n; i++) {
            Module.setValue(regIdsPtr + i * 4, regIds[i], 'i32');
            Module.setValue(regValsPtr + i * 4, values[i], 'i32');
            Module.setValue(regPtrsPtr + i * 4, regValsPtr + i * 4, 'i32');
        }
        Module.ccall('uc_reg_write_batch', 'number', ['number', 'number', 'number', 'number'], [handle, regIdsPtr, regPtrsPtr, n]);
    };

    const { periph_read, periph_write, tick, step_batch, process_batch, get_next_pending_interrupt,
    intr_next, intr_svc_enter, intr_svc_leave, intr_svc_depth,
    dma_pump_all, dma_take_absorbed, dma_set_completed_many, dma_absorb_periph, dma_push_periph, is_watchdog_reset_requested, dma_get_pending_count,
    add_spi_flash, add_i2c_eeprom, add_touchscreen, add_lcd, add_i2c_oled, add_software_spi, reset_ext_devices,
    add_fsmc_bank, fsmc_write_byte, fsmc_read_byte,
    add_sd_card,
    register_js_peripheral,
    init, init_svd, has_pending_interrupt, get_uart_output, uart_rx_byte, uart_rx_pending, gpio_read_output,
    gpio_set_input, gpio_read_input, set_intr_masks, clear_current_interrupt, finish_interrupt,
    can_inject_message, adc_set_sim_value, gpio_set_analog, adc_set_rc_tau,
    touchscreen_set_touch, pwm_duty, raise_fault,
     i2c_oled_fb, lcd_fb, gpio_take_pin_events,     drain_events, spi_inject_miso, i2c_inject_rx, usb_inject_setup, usb_inject_out,
    rustcpu_init, rustcpu_load, rustcpu_run, rustcpu_fault, rustcpu_fault_clear, rustcpu_dispatch,
    rustcpu_regs, rustcpu_set_pc, rustcpu_mem_read, rustcpu_mem_write, rustcpu_dma_pump, rustcpu_i2c_hook_fired,
    rustcpu_write_tap, rustcpu_take_writes } = periph;

    // Register external devices BEFORE init()
    reset_ext_devices();
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
    for (const d of ext_devices.fsmc_bank || []) {
        add_fsmc_bank(d.name, d.data);
    }
    for (const d of ext_devices.sd_card || []) {
        add_sd_card(d.peripheral, d.data ?? new Uint8Array(0));
    }

    const chipSvd = (typeof chip === 'string') ? (svd ?? null) : (chip.svd ?? null);
    if (typeof chip === 'string' && chip !== 'stm32f103c8' && !chipSvd) {
        console.warn(`createEmulator: unknown chip "${chip}" (no SVD provided), using builtin STM32F103C8 map`);
    }
    if (chipSvd) {
        init_svd(chipSvd);
    } else {
        init();
    }

    // rp2040js-style custom peripherals: JS callbacks on the peripheral bus.
    for (const jp of js_peripherals || []) {
        register_js_peripheral(jp.base, jp.size, jp.read, jp.write);
    }

    const uc = useRust ? null : new Module.Unicorn(
        Module.ARCH_ARM,
        Module.MODE_THUMB | Module.MODE_LITTLE_ENDIAN
    );

    const flash_addr = vector_table & ~0x1FFFF;
    if (uc) uc.mem_map(flash_addr, flash_size, Module.PROT_ALL);
    if (firmware instanceof ArrayBuffer) firmware = new Uint8Array(firmware);
    let fwBytes = firmware;
    let fwAddr = flash_addr;
    let elfRegions = null;
    let symbolList = [];

    // Unicorn ARM cannot decode `mrs rX, msp` (used by newlib _sbrk). In thread
    // mode MSP == SP, so rewrite to `mov rX, sp` + nop (same 4-byte footprint).
    const patchMrsMsp = (data) => {
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
        if (patched > 0 && verbose) console.log(`Patched ${patched} 'mrs msp' to 'mov sp' (malloc/_sbrk support)`);
        return data;
    };

    if (typeof firmware === 'string' || (firmware instanceof Uint8Array && firmware.length > 0 && firmware[0] === 0x3A)) {
        const text = typeof firmware === 'string' ? firmware : new TextDecoder().decode(firmware);
        const parsed = parseIntelHex(text);
        fwBytes = patchMrsMsp(parsed.data);
        if (parsed.base >= flash_addr && parsed.base < flash_addr + flash_size) fwAddr = parsed.base;
    } else if (firmware instanceof Uint8Array && firmware.length > 4 &&
               firmware[0] === 0x7F && firmware[1] === 0x45 && firmware[2] === 0x4C && firmware[3] === 0x46) {
        const elf = parseElf(firmware);
        elfRegions = elf.regions;
        fwBytes = new Uint8Array(0);
        symbolList = elf.symbols;
        if (verbose) console.log(`ELF: ${elf.regions.length} load segments, ${elf.symbols.length} symbols`);
    }
    // fwBytes written after RAM is mapped below (unicorn) or loaded via
    // rustcpu_load (rust backend; the Rust core decodes `mrs` itself, so no
    // patchMrsMsp rewrite there).
    const maybePatch = useRust ? (d) => d : patchMrsMsp;

    // TEMP WORKAROUND (Unicorn-only; the Rust core runs the real `bl`s fine):
    // Unicorn skips the two `bl HAL_NVIC_EnableIRQ` in i2c_init(). Replace
    // 0x8001bbc..0x8001bdb with inline NVIC ISER0/ISER1 writes (SetPriority
    // calls preserved). Probe guards against other builds.
    if (uc) try {
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
            if (verbose) console.log('Applied i2c_init IRQ-enable patch (Unicorn bl skip workaround)');
        }
    } catch (e) {
        if (verbose) console.error('i2c_init patch failed:', e.message);
    }

    if (uc) uc.mem_map(0x20000000, ram_size, Module.PROT_ALL);

    if (!uc) {
        // Rust backend inits BEFORE loading (load needs the CPU/RAM pair);
        // SP/PC come straight from the image bytes.
        const vecAt = (off) => {
            const a = vector_table + off;
            if (elfRegions) {
                for (const reg of elfRegions) {
                    if (a >= reg.start && a + 4 <= reg.start + reg.data.length) {
                        const o = a - reg.start;
                        return (reg.data[o] | (reg.data[o + 1] << 8) | (reg.data[o + 2] << 16) | (reg.data[o + 3] << 24)) >>> 0;
                    }
                }
                return 0;
            }
            const o = a - fwAddr;
            if (!fwBytes.length || o < 0 || o + 4 > fwBytes.length) return 0;
            return (fwBytes[o] | (fwBytes[o + 1] << 8) | (fwBytes[o + 2] << 16) | (fwBytes[o + 3] << 24)) >>> 0;
        };
        rustcpu_init(vecAt(0), vecAt(4), flash_size, ram_size);
    }

    if (elfRegions) {
        let wrote = 0;
        for (const reg of elfRegions) {
            const inFlash = reg.start >= flash_addr && reg.start < flash_addr + flash_size;
            const inRam = reg.start >= 0x20000000 && reg.start < 0x20000000 + ram_size;
            if (inFlash || inRam) {
                if (uc) uc.mem_write(BigInt(reg.start), maybePatch(reg.data));
                else rustcpu_load(reg.data, reg.start >>> 0);
                wrote++;
            }
        }
        if (verbose) console.log(`ELF: ${wrote} load segments written`);
    }
    if (fwBytes.length > 0) {
        const bytes = maybePatch(fwBytes);
        if (uc) uc.mem_write(BigInt(fwAddr), bytes);
        else rustcpu_load(bytes, fwAddr >>> 0);
    }

    if (uc) for (const [start, end] of PERIPH_RANGES) {
        uc.mem_map(start, end - start, Module.PROT_READ | Module.PROT_WRITE);
    }

    const read32 = (addr) => {
        if (uc) {
            const b = uc.mem_read(BigInt(addr), 4);
            return new DataView(b.buffer, b.byteOffset, b.byteLength).getUint32(0, true);
        }
        const b = rustcpu_mem_read(addr >>> 0, 4);
        return new DataView(b.buffer, b.byteOffset, b.byteLength).getUint32(0, true);
    };
    const write32 = (addr, val) => {
        if (uc) {
            const b = new Uint8Array(4);
            new DataView(b.buffer).setUint32(0, val >>> 0, true);
            uc.mem_write(BigInt(addr), b);
            return;
        }
        const b = new Uint8Array(4);
        new DataView(b.buffer).setUint32(0, val >>> 0, true);
        rustcpu_mem_write(addr >>> 0, b);
    };

    const sp_init = read32(vector_table) >>> 0;
    const pc_init = read32(vector_table + 4) >>> 0;
    if (uc) {
        uc.reg_write_i32(Module.ARM_REG_SP, sp_init);
        uc.reg_write_i32(Module.ARM_REG_PC, pc_init | 1);
    }
    // (rust backend already inited above; read32 here just re-reads the image)

    if (verbose) {
        console.log(`SP=0x${sp_init.toString(16)} PC=0x${(pc_init | 1).toString(16)}`);
    }

    // Unicorn-only: peripheral bus hooks (the Rust backend executes model
    // writes in-Rust; write watchers are fed from rustcpu_take_writes instead).
    if (uc) {
    const memReadHook = (handle, type, address, size, value, user_data) => {
        const addr32 = Number(address);
        // Poll detector: consecutive reads of one address = firmware spinning
        // on a status flag. Any other address ends the streak (progress).
        if (addr32 === lastPollAddr) {
            if (++pollStreak >= POLL_THRESHOLD) polling = true;
        } else {
            lastPollAddr = addr32;
            pollStreak = 1;
            polling = false;
        }
        let val;
        if (addr32 >= 0xE0001000 && addr32 < 0xE0001100) {
            // SysTick: Rust never decrements CVR; fake a counting-down value
            val = addr32 === 0xE0001004
                ? instCount & 0xFFFFFFFF
                : (addr32 === 0xE0001000 ? 1 : 0);
        } else {
            val = periph_read(addr32, size) >>> 0;
        }
        const bytes = new Uint8Array(size);
        for (let i = 0; i < size; i++) bytes[i] = (val >> (i * 8)) & 0xFF;
        uc.mem_write(address, bytes);
    };

    const memWriteHook = (handle, type, address, size, value, user_data) => {
        const addr32 = Number(address);
        const valueNum = Number(value);
        periph_write(addr32, size, valueNum);
        // TEMP WORKAROUND: HAL I2C1 ISR requires hi2c->Mode == 0x22 (MASTER_RX)
        // before reading DR; patch it in RAM when a read-request is written to DR.
        if (addr32 === 0x40005410 && (valueNum & 1) === 1) {
            try {
                const hi2c1Ptr = read32(0x200002d8);
                if (hi2c1Ptr && hi2c1Ptr !== 0xFFFFFFFF) {
                    uc.mem_write(BigInt(hi2c1Ptr + 0x3D), new Uint8Array([0x22]));
                }
            } catch (_) {}
        }
        // External bus observers: page-side peripheral drivers (7-seg, buzzer, ...) tap
        // the peripheral bus exactly like real hardware taps the pins.
        // Pin events first: a CS-level change recorded by periph_write must be
        // visible to the write watchers of the NEXT hook call (SPI DR while CS low).
        // Fast-path: skip the WASM crossing when no one is listening.
        if (pinWatchers.length) drainPinEvents();
        if (writeWatchers.length) {
            for (let wi = 0; wi < writeWatchers.length; wi++) {
                try { writeWatchers[wi](addr32, size, valueNum); } catch (e) {}
            }
        }
    };

    for (const [start, end] of PERIPH_RANGES) {
        uc.hook_add(Module.HOOK_MEM_READ, memReadHook, null, start, end);
        uc.hook_add(Module.HOOK_MEM_WRITE, memWriteHook, null, start, end);
    }
    } // end Unicorn-only hooks

    let stopRequested = false;
    let instCount = 0;
    let batchInstCount = 0;
    // Poll-aware batch state (written by memReadHook, consumed by run()).
    let lastPollAddr = 0, pollStreak = 0, polling = false;
    let smallBatchStreak = 0, pollBackoff = 0;
    const writeWatchers = [];
    const pinWatchers = [];

    // Drain buffered GPIO pin-change events (flat [port, pin, level, ...]) into
    // the pin watchers. Pull-style: runs at the top of each memWriteHook (so a
    // CS-low event is visible before the next DR write's watcher runs) and once
    // per batch in run()/step(). No JS callback ever runs reentrantly inside Rust.
    const drainPinEvents = () => {
        if (!pinWatchers.length) return;
        const ev = gpio_take_pin_events();
        for (let i = 0; i + 2 < ev.length; i += 3) {
            const port = ev[i], pin = ev[i + 1], level = ev[i + 2];
            for (let wi = 0; wi < pinWatchers.length; wi++) {
                try { pinWatchers[wi](port, pin, level); } catch (e) {}
            }
        }
    };

    // Hookless instruction counting: emu_start(begin, 0, 0, maxBatch) stops exactly at
    // maxBatch instructions except on a fault (unmapped access, ~0.01% of batches),
    // where the faulting instruction is skipped and the batch credited in full.
    // Counting in JS per instruction cost ~20% of runtime; a full-batch credit is exact
    // for normal batches and off by <1 batch on rare faults. Handler runs (inside
    // processInterrupts) are not credited — instruction-delta peripherals self-correct.

    // SVC frames live in Rust (src/interrupts.rs, shared with cli.mjs — the
    // same mirror used to be duplicated here and in cli.mjs and they kept
    // drifting). The frame is also written to the real stack so handler code
    // can inspect it (Cortex-M ABI). Unicorn-only (Rust dispatches inline).
    if (uc) {
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
            uc.reg_write_i32(Module.ARM_REG_XPSR, sv.getUint32(0, true));
            uc.reg_write_i32(Module.ARM_REG_SP, sp + 32);
        } else if (intno === 2) {
            // SVC: stack the interrupted context (mirror + real stack frame,
            // both built in Rust), enter handler mode (LR = EXC_RETURN), jump
            // to the SVCall vector. Return happens when the handler executes
            // `bx lr` (Unicorn faults fetching 0xFFFFFFFx, caught in
            // handleFault, which pops the mirror via intr_svc_leave).
            const sp = uc.reg_read_i32(Module.ARM_REG_SP);
            const frame = intr_svc_enter(
                uc.reg_read_i32(Module.ARM_REG_R0),
                uc.reg_read_i32(Module.ARM_REG_R1),
                uc.reg_read_i32(Module.ARM_REG_R2),
                uc.reg_read_i32(Module.ARM_REG_R3),
                uc.reg_read_i32(Module.ARM_REG_R12),
                uc.reg_read_i32(Module.ARM_REG_LR),
                uc.reg_read_i32(Module.ARM_REG_PC),
                uc.reg_read_i32(Module.ARM_REG_XPSR),
                sp,
            );
            if (frame.length) {
                uc.mem_write(BigInt(sp - 32), frame);
                uc.reg_write_i32(Module.ARM_REG_SP, sp - 32);
                const control = uc.reg_read_i32(Module.ARM_REG_CONTROL);
                uc.reg_write_i32(Module.ARM_REG_LR, control & 1 ? 0xFFFFFFFD : 0xFFFFFFF9);
                uc.reg_write_i32(Module.ARM_REG_PC, read32(vector_table + 4 * 11));
            }
        }
    };
    uc.hook_add(Module.HOOK_INTR, intrHook, null);
    } // end Unicorn-only intrHook

    // ---- backend primitives: run()/step() bodies below are shared ----
    // Unicorn path: emu_start + hooks + JS DMA RAM moves + JS IRQ dispatch.
    // Rust path: rustcpu_* exports (DMA pump + dispatch run fully in Rust
    // against Rust RAM; no Unicorn instance, hooks, or RAM crossings).
    const pumpDma = () => {
        if (uc) processDma();
        else rustcpu_dma_pump();
    };
    // Execute one CPU batch; returns credited instructions (unicorn: full
    // batch credit; rust: exact executed count incl. handlers).
    const execBatch = (n) => {
        if (uc) {
            const curPc = uc.reg_read_i32(Module.ARM_REG_PC);
            try {
                uc.emu_start(curPc | 1, 0, 0, n);
            } catch (e) {
                if (!handleFault(String(e))) throw e;
            }
            return n;
        }
        const done = rustcpu_run(n);
        const fault = rustcpu_fault();
        if (fault.length) {
            const fpc = fault[0] >>> 0, op1 = fault[1] >>> 0;
            const sym = resolveSym(fpc);
            if (verbose) console.log(`FAULT @${sym || ('0x' + fpc.toString(16))} op=0x${op1.toString(16)} (rust cpu decode gap)`);
            if (symbolList.length) raise_fault(3, fpc); // UNDEFINSTR; runs via dispatch
            rustcpu_set_pc((fpc + 2) | 1);
            rustcpu_fault_clear();
        }
        return done;
    };
    // Feed write watchers from the Rust write tap (unicorn path is hook-fed).
    const feedWriteTap = () => {
        if (uc || !writeWatchers.length) return;
        const w = rustcpu_take_writes();
        for (let i = 0; i + 2 < w.length; i += 3) {
            const a = w[i], s = w[i + 1], v = w[i + 2];
            for (let wi = 0; wi < writeWatchers.length; wi++) {
                try { writeWatchers[wi](a, s, v); } catch (e) {}
            }
        }
    };
    const dispatchBatch = (anyPending) => {
        if (uc) {
            processInterrupts(anyPending);
            return;
        }
        // hi2c->Mode RAM patch BEFORE dispatch (the ISR reads Mode): same
        // condition as the Unicorn memWriteHook, drained per batch.
        if (rustcpu_i2c_hook_fired()) {
            try {
                const p = read32(0x200002d8);
                if (p && p !== 0xFFFFFFFF) rustcpu_mem_write((p + 0x3D) >>> 0, new Uint8Array([0x22]));
            } catch (_) {}
        }
        if (anyPending) {
            rustcpu_dispatch();
            const hf = rustcpu_fault();
            if (hf.length) throw new Error(`rust CPU fault in IRQ handler at 0x${(hf[0] >>> 0).toString(16)}`);
        }
    };

    const processDma = () => {
        if (dma_get_pending_count() === 0) return;
        const plan = dma_pump_all();
        if (plan.length === 0) return;
        for (let i = 0; i + 4 <= plan.length; i += 4) {
            const op = plan[i], a = plan[i + 1], b = plan[i + 2], c = plan[i + 3];
            try {
                if (op === 0) {
                    uc.mem_write(BigInt(b), uc.mem_read(BigInt(a), c));
                } else if (op === 1) {
                    uc.mem_write(BigInt(a), dma_take_absorbed(c, b));
                } else if (op === 2) {
                    const pushed = uc.mem_read(BigInt(a), b);
                    dma_push_periph(c, pushed);
                    // DMA writes the peripheral register, not the CPU — feed the
                    // bus watchers (onPeriphWrite) one byte at a time like real
                    // hardware, else page-side decoders never see DMA traffic.
                    if (writeWatchers.length) {
                        for (let bi = 0; bi < b; bi++) {
                            for (let wi = 0; wi < writeWatchers.length; wi++) {
                                try { writeWatchers[wi](c, 1, pushed[bi]); } catch (e) {}
                            }
                        }
                    }
                } else if (op === 3) {
                    dma_set_completed_many(a);
                }
            } catch (e) { /* ignore per-transfer errors */ }
        }
    };

    const processInterrupts = (anyPending) => {
        if (!anyPending) return;
        while (true) {
            const irq = intr_next();
            if (irq <= -100) break;

            const regs = regsRead(uc, [
                Module.ARM_REG_SP, Module.ARM_REG_PC, Module.ARM_REG_LR, Module.ARM_REG_XPSR,
                Module.ARM_REG_R0, Module.ARM_REG_R1, Module.ARM_REG_R2, Module.ARM_REG_R3,
                Module.ARM_REG_R12,
            ]);
            const [savedAt, pc, lr, xpsr, r0, r1, r2, r3, r12] = regs;
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
            const handler_pc = read32(vector_table + 4 * (16 + irq));
            regsWrite(uc, [Module.ARM_REG_SP, Module.ARM_REG_LR, Module.ARM_REG_PC], [savedAt - 32, 0xFFFFFFF9, handler_pc]);
            try {
                uc.emu_start(handler_pc, 0, 0, DEFAULT_MAX_BATCH);
            } catch (e) { /* BX LR EXC_RETURN handled by intrHook */ }
            finish_interrupt(irq);
            // Restore from the stacked frame (not JS locals) so a handler that
            // edits the saved context is honored. xPSR restore is REQUIRED —
            // the handler's emu_start clobbers APSR, and a cmp/beq pair split
            // across a batch boundary would evaluate with the handler's flags.
            const savedFrame = uc.mem_read(BigInt(savedAt - 32), 32);
            const savedSv = new DataView(savedFrame.buffer, savedFrame.byteOffset, savedFrame.byteLength);
            regsWrite(uc, [
                Module.ARM_REG_R0, Module.ARM_REG_R1, Module.ARM_REG_R2, Module.ARM_REG_R3,
                Module.ARM_REG_R12, Module.ARM_REG_LR, Module.ARM_REG_PC, Module.ARM_REG_XPSR,
                Module.ARM_REG_SP,
            ], [
                savedSv.getUint32(28, true), savedSv.getUint32(24, true), savedSv.getUint32(20, true),
                savedSv.getUint32(16, true), savedSv.getUint32(12, true), savedSv.getUint32(8, true),
                savedSv.getUint32(4, true) | 1, savedSv.getUint32(0, true), savedAt,
            ]);
            processDma();
        }
    };

    // Resolve an address to the nearest preceding ELF symbol (or null when
    // no symbol table is available, e.g. hex-only firmware).
    let symSorted = null;
    const resolveSym = (addr) => {
        if (!symbolList.length) return null;
        if (!symSorted) {
            symSorted = symbolList.slice().sort((a, b) => a.addr - b.addr);
        }
        const a = addr & ~1;
        let lo = 0, hi = symSorted.length - 1, best = -1;
        while (lo <= hi) {
            const mid = (lo + hi) >> 1;
            if (symSorted[mid].addr <= a) { best = mid; lo = mid + 1; } else hi = mid - 1;
        }
        if (best < 0) return null;
        const s = symSorted[best];
        const off = a - s.addr;
        if (off > 0x20000) return null;
        return off > 0 ? `${s.name}+0x${off.toString(16)}` : s.name;
    };

    // Handle an emu_start fault: SVC return (EXC_RETURN pop), the known
    // Unicorn `bl` artifact at HAL_NVIC_EnableIRQ (skip), real faults (raise
    // them — the fault handler runs via processInterrupts), or no symbol table
    // (legacy tolerant skip). Returns true if the fault was consumed.
    const handleFault = (msg) => {
        const pc2 = uc.reg_read_i32(Module.ARM_REG_PC);
        if (msg.includes('UC_ERR_FETCH_UNMAPPED') && ((pc2 & ~1) >>> 0) >= 0xFFFFFFF0 && intr_svc_depth() > 0) {
            // SVC handler returned via `bx lr` (EXC_RETURN): restore the
            // pre-SVC context from the Rust mirror.
            const st = intr_svc_leave();
            if (st.length === 9) {
                uc.reg_write_i32(Module.ARM_REG_R0, st[0]);
                uc.reg_write_i32(Module.ARM_REG_R1, st[1]);
                uc.reg_write_i32(Module.ARM_REG_R2, st[2]);
                uc.reg_write_i32(Module.ARM_REG_R3, st[3]);
                uc.reg_write_i32(Module.ARM_REG_R12, st[4]);
                uc.reg_write_i32(Module.ARM_REG_LR, st[5]);
                uc.reg_write_i32(Module.ARM_REG_PC, st[6] | 1);
                uc.reg_write_i32(Module.ARM_REG_SP, st[7]);
                // xPSR restore is REQUIRED here for the same reason as on
                // the IRQ path: the handler's emu_start clobbers APSR.
                uc.reg_write_i32(Module.ARM_REG_XPSR, st[8]);
            }
            return true;
        }
        if (msg.includes('UC_ERR_READ_UNMAPPED') || msg.includes('UC_ERR_FETCH_UNMAPPED') || msg.includes('UC_ERR_WRITE_UNMAPPED')) {
            const sym = resolveSym(pc2);
            if (sym && sym.includes('HAL_NVIC_EnableIRQ')) {
                // Known Unicorn `bl` decode artifact at HAL_NVIC_EnableIRQ+0xf:
                // not a real fault, skip the faulting instruction like before.
                uc.reg_write_i32(Module.ARM_REG_PC, (pc2 + 2) | 1);
            } else if (symbolList.length) {
                // Real fault: raise it. CFSR/HFSR/BFAR are populated and the
                // fault handler runs via processInterrupts.
                const kind = msg.includes('FETCH_UNMAPPED') ? 0 : (msg.includes('WRITE_UNMAPPED') ? 2 : 1);
                raise_fault(kind, 0);
            } else {
                // No symbol table: keep the legacy tolerant skip.
                uc.reg_write_i32(Module.ARM_REG_PC, (pc2 + 2) | 1);
            }
            return true;
        }
        return false;
    };

    return {
        uc, Module, read32, write32,

        /** Run up to maxInstructions (0 = forever). Returns {totalSteps, instCount, stopped}. */
        run(maxInstructions = 0) {
            stopRequested = false;
            const startInst = instCount;
            let totalSteps = 0;
            let anyPending = false;
            // profiling
            let t_emu=0, t_batch=0, t_dma=0, t_irq=0, t_pin=0;
            const profile = typeof process !== 'undefined' && process.env.PROFILE;
            while (!stopRequested) {
                // Adaptive batch: small (20K) when IRQs/DMA pending for low latency,
                // large (50K) when idle for throughput. Poll-aware shrink: while
                // the firmware spins on a status flag, run POLL_BATCH so the
                // batch-boundary flag refresh lands sooner (saves ~B/2 spin
                // instructions per awaited event). Back off after sustained
                // polling (external wait — small batches only add overhead).
                // If user set batch_size explicitly, respect it as fixed.
                let curBatch;
                if (maxBatch !== DEFAULT_MAX_BATCH) {
                    curBatch = maxBatch;
                } else if (polling && pollBackoff === 0 && (typeof process === 'undefined' || process.env.POLL_SHRINK !== '0')) {
                    curBatch = POLL_BATCH;
                    polling = false; // re-armed by the hook if the spin continues
                    if (++smallBatchStreak >= POLL_BACKOFF_AFTER) {
                        pollBackoff = POLL_BACKOFF_AFTER;
                        smallBatchStreak = 0;
                    }
                } else {
                    curBatch = ((anyPending || dma_get_pending_count() !== 0) ? DEFAULT_MAX_BATCH : LARGE_BATCH);
                    smallBatchStreak = 0;
                    if (pollBackoff > 0) pollBackoff--;
                }
                let t;
                if (profile) t=performance.now();
                pumpDma();
                if (profile) t_dma+=performance.now()-t;
                if (profile) t=performance.now();
                const credited = execBatch(curBatch);
                if (profile) t_emu+=performance.now()-t;
                instCount += credited;
                batchInstCount += credited;
                if (batchInstCount > 0) {
                    if (profile) t=performance.now();
                    const status = process_batch(batchInstCount);
                    if (profile) t_batch+=performance.now()-t;
                    batchInstCount = 0;
                    if (status & 0x80000000) { stopRequested = true; break; }
                    anyPending = (status & 0x40000000) !== 0;
                }
                if (profile) t=performance.now();
                pumpDma();
                if (profile) t_dma+=performance.now()-t;
                if (profile) t=performance.now();
                dispatchBatch(anyPending);
                if (profile) t_irq+=performance.now()-t;
                if (profile) t=performance.now();
                drainPinEvents();
                feedWriteTap();
                if (profile) t_pin+=performance.now()-t;
                totalSteps++;
                if (is_watchdog_reset_requested()) break;
                if (maxInstructions > 0 && instCount - startInst >= maxInstructions) break;
            }
            if (profile) {
                const total = t_emu+t_batch+t_dma+t_irq+t_pin;
                const done = instCount - startInst;
                console.error(`[profile] emu ${(t_emu/total*100).toFixed(1)}% batch ${(t_batch/total*100).toFixed(1)}% dma ${(t_dma/total*100).toFixed(1)}% irq ${(t_irq/total*100).toFixed(1)}% pin ${(t_pin/total*100).toFixed(1)}%  total ${total.toFixed(1)}ms for ${done} instr  MIPS ${(done/total/1000).toFixed(1)}`);
            }
            return {
                totalSteps,
                instCount,
                stopped: stopRequested || is_watchdog_reset_requested(),
            };
        },

        /** Run one batch and return after processing DMA/interrupts. Default-size
         *  steps shrink while the firmware spins on a status flag (same
         *  poll-aware policy as run() — worker.js / page runLoop land here);
         *  explicit sizes, or an explicit batch_size opt, always run exact. */
        step(count = maxBatch) {
            let n = count;
            if (count === maxBatch && maxBatch === DEFAULT_MAX_BATCH
                && (typeof process === 'undefined' || process.env.POLL_SHRINK !== '0')) {
                if (polling && pollBackoff === 0) {
                    n = POLL_BATCH;
                    polling = false; // re-armed by the hook if the spin continues
                    if (++smallBatchStreak >= POLL_BACKOFF_AFTER) {
                        pollBackoff = POLL_BACKOFF_AFTER;
                        smallBatchStreak = 0;
                    }
                } else {
                    smallBatchStreak = 0;
                    if (pollBackoff > 0) pollBackoff--;
                }
            }
            pumpDma();
            const credited = execBatch(n);
            instCount += credited;
            batchInstCount += credited;
            let anyPending = false;
            if (batchInstCount > 0) {
                const status = process_batch(batchInstCount);
                batchInstCount = 0;
                if (status & 0x80000000) stopRequested = true;
                anyPending = (status & 0x40000000) !== 0;
            }
            pumpDma();
            dispatchBatch(anyPending);
            drainPinEvents();
            feedWriteTap();
            return {
                pc: uc ? uc.reg_read_i32(Module.ARM_REG_PC) : (rustcpu_regs()[15] >>> 0),
                instCount,
                stopped: stopRequested || is_watchdog_reset_requested(),
            };
        },

        stop() {
            stopRequested = true;
            if (uc) try { uc.emu_stop(); } catch (e) { /* ignore */ }
        },

        getRegisters() {
            if (uc) {
                const regs = {};
                for (let i = 0; i <= 12; i++) regs[`R${i}`] = uc[`reg_read_i32`](Module[`ARM_REG_R${i}`]);
                regs.SP = uc.reg_read_i32(Module.ARM_REG_SP);
                regs.LR = uc.reg_read_i32(Module.ARM_REG_LR);
                regs.PC = uc.reg_read_i32(Module.ARM_REG_PC);
                regs.xPSR = uc.reg_read_i32(Module.ARM_REG_XPSR);
                return regs;
            }
            // rustcpu_regs(): [r0..r12, sp, lr, pc, xpsr, primask, control, ipsr]
            const r = rustcpu_regs();
            const regs = {};
            for (let i = 0; i <= 12; i++) regs[`R${i}`] = r[i] >>> 0;
            regs.SP = r[13] >>> 0;
            regs.LR = r[14] >>> 0;
            regs.PC = r[15] >>> 0;
            regs.xPSR = r[16] >>> 0;
            return regs;
        },

        getPc() { return uc ? uc.reg_read_i32(Module.ARM_REG_PC) : (rustcpu_regs()[15] >>> 0); },
        getSp() { return uc ? uc.reg_read_i32(Module.ARM_REG_SP) : (rustcpu_regs()[13] >>> 0); },
        setPc(pc) { if (uc) uc.reg_write_i32(Module.ARM_REG_PC, pc | 1); else rustcpu_set_pc(pc | 1); },

        /** Set symbol table (from .elf or .map) for resolveSymbol(). */
        setSymbols(list) {
            symbolList = list || [];
            symSorted = null;
        },

        getSymbolCount() { return symbolList.length; },

        /**
         * Resolve an address to the nearest symbol at or below it (e.g. 'main+0x1e').
         * @returns {string|null}
         */
        resolveSymbol(addr) {
            return resolveSym(addr);
        },

        getUartOutput() { return get_uart_output(); },

        /** Inject a byte into the UART RX (default: USART1 @ 0x40013800). */
        uartRx(byte) { return uart_rx_byte(uart_addr, byte); },
        /** Inject a received byte into a specific USART (by base address). */
        uartRxAddr(addr, byte) { return uart_rx_byte(addr, byte); },
        uartRxBytes(bytes) {
            let ok = false;
            for (const b of bytes) ok = uart_rx_byte(uart_addr, b) || ok;
            return ok;
        },

        /** Unread bytes still queued in the UART RX buffer (0 = empty). */
        rxPending() { return uart_rx_pending(uart_addr); },

        /** True while a DMA transfer is queued (mirror of cli.mjs dmaBusy gate:
         *  hold UART bytes back while DMA is busy so the DMA RX test, not the
         *  UART RX test, consumes the reserved byte). */
        dmaPending() { return dma_get_pending_count() > 0; },

        /** Read a 32-bit word from emulated memory (e.g. a RAM flag). */
        memRead32(addr) {
            if (uc) {
                const b = uc.mem_read(BigInt(addr), 4);
                const dt = new DataView(b.buffer, b.byteOffset, b.byteLength);
                return dt.getUint32(0, true);
            }
            return read32(addr) >>> 0;
        },

        canInjectMessage(addr, tir, tdtr, tdlr, tdhr) {
            return can_inject_message(addr, tir, tdtr, tdlr, tdhr);
        },

        gpioReadOutput(port, pin) { return gpio_read_output(port, pin); },
        gpioReadInput(port, pin) { return gpio_read_input(port, pin); },
        gpioSetInput(port, pin, value) { gpio_set_input(port, pin, value); },

        /** PWM duty (0-100) of a timer channel, e.g. pwmDuty(0x40000000, 0) = TIM2 CH1. */
        pwmDuty(addr, channel = 0) { return pwm_duty(addr, channel); },

        setSimAdc(value) { adc_set_sim_value(value); },
        gpioSetAnalog(port, pin, level) { gpio_set_analog(port, pin, level); },
        adcSetRcTau(cycles) { adc_set_rc_tau(cycles); },
        setTouch(peripheral, x, y, pressure) { touchscreen_set_touch(peripheral, x, y, pressure); },

        /** Watch every peripheral register write: fn(addr, width, value). Returns unsubscribe. */
        onPeriphWrite(fn) {
            writeWatchers.push(fn);
            // Rust backend has no mem hooks: enable the in-model write tap.
            if (!uc) rustcpu_write_tap(true);
            return () => {
                const i = writeWatchers.indexOf(fn);
                if (i >= 0) writeWatchers.splice(i, 1);
                if (!uc && writeWatchers.length === 0) rustcpu_write_tap(false);
            };
        },

        /** Watch chip-driven GPIO level changes: fn(port, pin, level) (port 0=GPIOA, level 0/1).
         *  Fires when the chip drives an output pin to a NEW level (ODR/BSRR/BRR writes, or
         *  CRL/CRH writes re-driving ODR). Does NOT fire for gpioSetInput (JS→chip direction).
         *  Drained automatically each batch (and before write watchers at each hook). Returns unsubscribe. */
        onPinChange(fn) {
            pinWatchers.push(fn);
            return () => {
                const i = pinWatchers.indexOf(fn);
                if (i >= 0) pinWatchers.splice(i, 1);
            };
        },

        /** Drain buffered pin-change events directly (flat [port, pin, level, ...]). */
        takePinEvents() { return gpio_take_pin_events(); },

        /** Drain virtual-peripheral transaction events as a flat i32 array (see drain_events export). */
        drainEvents() { return drain_events(); },

        /** Queue injected MISO bytes for a SPI channel (virtual device -> MCU). */
        spiInjectMiso(channel, bytes) { spi_inject_miso(channel, bytes); },

        /** Queue injected RX bytes for an I2C channel (virtual device -> MCU). */
        i2cInjectRx(channel, bytes) { i2c_inject_rx(channel, bytes); },

        /** Inject a USB SETUP packet (8 bytes) into EP0 (host -> device). Returns false when NAKed. */
        usbInjectSetup(bytes) { return usb_inject_setup(bytes); },

        /** Inject a USB OUT packet into an endpoint (host -> device). Returns false when NAKed. */
        usbInjectOut(ep, bytes) { return usb_inject_out(ep, bytes); },

        /** Current I2C OLED framebuffer, page-major (page*width + col), 1 byte per column. */
        i2cOledFb(peripheral, address = 0x3C) {
            const arr = i2c_oled_fb(peripheral, address);
            return arr && arr.length ? new Uint8Array(arr) : null;
        },

        /** Current SPI LCD framebuffer, 128x64 (y*128 + x), 1 byte per pixel. */
        lcdFb(peripheral) {
            const arr = lcd_fb(peripheral);
            return arr && arr.length ? new Uint8Array(arr) : null;
        },

        /** Low-level register access (width: 1, 2, or 4). */
        periphRead(addr, width = 4) { return periph_read(addr, width) >>> 0; },
        periphWrite(addr, width, value) { periph_write(addr, width, value); },

        /** Write a byte directly into an FSMC backing image (no bus side effects). */
        fsmcWriteByte(name, offset, value) { return fsmc_write_byte(name, offset, value); },
        /** Read a byte directly from an FSMC backing image. Returns -1 on error. */
        fsmcReadByte(name, offset) { return fsmc_read_byte(name, offset); },

        /**
         * Register an rp2040js-style custom peripheral on the bus.
         * read(addr, size) -> number; write(addr, value, size). Last registration
         * wins on overlap, so JS can shadow built-in peripherals.
         */
        addJsPeripheral(base, size, read, write) {
            return register_js_peripheral(base, size, read, write);
        },

        tick() { tick(); },
        stepBatch(count) { return step_batch(count); },
        hasPendingInterrupt() { return has_pending_interrupt(); },
        getNextPendingInterrupt() { return get_next_pending_interrupt(); },
        setIntrMasks(primask, basepri) { set_intr_masks(primask, basepri); },

        close() {
            if (uc) try { uc.close(); } catch (e) { /* ignore */ }
        },
    };
}

export default createEmulator;
