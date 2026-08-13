// Path A loader: ONE wasm module (Rust peripherals + Unicorn TCI C, emcc-linked)
// exposing the same `periph` API + Unicorn class the dual-wasm stack uses.
// Path A single-module loader (see BUILD.md). Landing artifact of the spike.

import mergedFactory from './merged4.mjs';
import { unicornClassBlock } from './unicorn_class.js';

// Old-ABI JsValue registry: JS objects cross the wasm boundary as u32 indices
// into this array (the emscripten wasm-bindgen convention).
const JSOBJ = [];
const objIdx = (o) => { JSOBJ.push(o); return JSOBJ.length - 1; };
const objAt = (i) => (i >= 0 && i < JSOBJ.length) ? JSOBJ[i] : undefined;

export async function loadMergedFromBytes(wasmBytes) {
    const Module = await mergedFactory({
        instantiateWasm(imports, success) {
            for (const imp of WebAssembly.Module.imports(new WebAssembly.Module(wasmBytes))) {
                if (imp.module === '__wbindgen_placeholder__') continue;
                const table = (imports[imp.module] ??= {});
                if (typeof table[imp.name] !== 'function') table[imp.name] = () => 0;
            }
            const dv = () => new DataView(Module.HEAPU8.buffer);
            const getStr = (p, l) => new TextDecoder().decode(new Uint8Array(Module.HEAPU8.buffer, p, l));
            imports['__wbindgen_placeholder__'] = {
                __wbindgen_object_drop_ref: (i) => { if (i >= 0 && i < JSOBJ.length) delete JSOBJ[i]; },
                __wbindgen_describe: () => 0,
                __wbindgen_describe_cast: (fn, ptr) => {
                    // Rust-side JsValue::from_f64 lands here: breaks_if_inlined
                    // packs the f64 at struct offset 0, then does
                    // ptr::read(describe_cast(fn, ptr)) to get the JsValue
                    // index. Register the number and return a scratch holding
                    // its registry index.
                    const f = dv().getFloat64(ptr, true);
                    castScratchH()[castScratch >> 2] = objIdx(f);
                    return castScratch;
                },
                __wbg_new_227d7c05414eb861: () => objIdx(new Error()),
                __wbg_stack_3b0d974bbf31e44f: (arg0, arg1) => {
                    const ret = objAt(arg1)?.stack ?? '';
                    const bytes = new TextEncoder().encode(ret);
                    const ptr = Module._malloc(bytes.length);
                    Module.HEAPU8.set(bytes, ptr);
                    Module.HEAPU32[arg0 >> 2] = ptr;
                    Module.HEAPU32[(arg0 + 4) >> 2] = bytes.length;
                },
                __wbg_error_a6fa202b58aa1cd3: (arg0, arg1) => console.error(getStr(arg0, arg1)),
                __wbg___wbindgen_number_get_394265ed1e1b84ee: (arg0, arg1) => {
                    const obj = objAt(arg1);
                    const ret = typeof obj === 'number' ? obj : undefined;
                    dv().setFloat64(arg0 + 8, ret === undefined ? 0 : ret, true);
                    dv().setInt32(arg0, ret === undefined ? 0 : 1, true);
                },
                __wbg_call_44b7209e1e252e6a: (a, b, c, d, e) => objIdx(objAt(a).call(objAt(b), objAt(c), objAt(d), objAt(e))),
                __wbg_call_e3b662382210db98: (a, b, c, d) => objIdx(objAt(a).call(objAt(b), objAt(c), objAt(d))),
            };
            WebAssembly.instantiate(wasmBytes, imports).then((r) => success(r.instance, r.module));
            return {};
        }
    });

    const H8 = () => Module.HEAPU8;
    const H32 = () => Module.HEAPU32;
    const scratch = Module._malloc(8);
    const castScratch = Module._malloc(4);
    const castScratchH = () => Module.HEAPU32;
    const castScratchV = () => castScratchH()[castScratch >> 2];
    const wbindgen_free = (ptr, len) => { if (ptr && len > 0 && ptr > 8) Module._free(ptr); };
    const wbindgen_malloc = (n) => Module._malloc(n);
    const wbindgen_realloc = (ptr, _old, n) => { const p = Module._malloc(n); if (ptr) { H8().copyWithin(p, ptr, ptr + Math.min(_old, n)); Module._free(ptr); } return p; };
    const passString = (s) => {
        const bytes = new TextEncoder().encode(s);
        const ptr = Module._malloc(bytes.length);
        H8().set(bytes, ptr);
        return [ptr, bytes.length];
    };
    const takeVec = () => { const p = H32()[scratch >> 2]; const l = H32()[(scratch + 4) >> 2]; return { ptr: p, len: l }; };
    const vecU8 = () => { const { ptr, len } = takeVec(); const out = new Uint8Array(len); out.set(H8().subarray(ptr, ptr + len)); wbindgen_free(ptr, len); return out; };
    const vecU32 = () => { const { ptr, len } = takeVec(); const out = new Uint32Array(len); out.set(new Uint32Array(Module.HEAPU8.buffer, ptr, len)); wbindgen_free(ptr, len); return out; };
    const vecU32Len = (n) => { const p = H32()[scratch >> 2]; const out = new Uint32Array(n); out.set(new Uint32Array(Module.HEAPU8.buffer, p, n)); wbindgen_free(p, n); return out; };
    const str = () => { const { ptr, len } = takeVec(); const out = new TextDecoder().decode(H8().subarray(ptr, ptr + len)); wbindgen_free(ptr, len); return out; };

    const periph = {
        adc_set_rc_tau: (cycles) => Module._adc_set_rc_tau(cycles),
        adc_set_sim_value: (val) => Module._adc_set_sim_value(val),
        add_fsmc_bank: (name, data) => {
            const [p0, l0] = passString(name);
            const p1 = Module._malloc(data.length); H8().set(data, p1);
            Module._add_fsmc_bank(p0, l0, p1, data.length);
        },
        add_i2c_eeprom: (peripheral, address, data) => {
            const [p0, l0] = passString(peripheral);
            const p1 = Module._malloc(data.length); H8().set(data, p1);
            Module._add_i2c_eeprom(p0, l0, address, p1, data.length);
        },
        add_i2c_oled: (peripheral, address, width, height) => {
            const [p0, l0] = passString(peripheral);
            Module._add_i2c_oled(p0, l0, address, width, height);
        },
        add_lcd: (peripheral, cs) => {
            const [p0, l0] = passString(peripheral);
            let p1 = 0, l1 = 0;
            if (cs != null) { [p1, l1] = passString(cs); }
            Module._add_lcd(p0, l0, p1, l1);
        },
        add_software_spi: (name, cs, clk, miso, mosi) => {
            const [p0, l0] = passString(name);
            let p1 = 0, l1 = 0;
            if (cs != null) { [p1, l1] = passString(cs); }
            const [p2, l2] = passString(clk);
            const [p3, l3] = passString(miso);
            const [p4, l4] = passString(mosi);
            Module._add_software_spi(p0, l0, p1, l1, p2, l2, p3, l3, p4, l4);
        },
        add_spi_flash: (peripheral, jedec_id, data, cs) => {
            const [p0, l0] = passString(peripheral);
            const p1 = Module._malloc(data.length); H8().set(data, p1);
            let p2 = 0, l2 = 0;
            if (cs != null) { [p2, l2] = passString(cs); }
            Module._add_spi_flash(p0, l0, jedec_id, p1, data.length, p2, l2);
        },
        add_touchscreen: (peripheral, touch_detected_pin, cs) => {
            const [p0, l0] = passString(peripheral);
            let p1 = 0, l1 = 0, p2 = 0, l2 = 0;
            if (touch_detected_pin != null) { [p1, l1] = passString(touch_detected_pin); }
            if (cs != null) { [p2, l2] = passString(cs); }
            Module._add_touchscreen(p0, l0, p1, l1, p2, l2);
        },
        can_inject_message: (addr, tir, tdtr, tdlr, tdhr) => Module._can_inject_message(addr, tir, tdtr, tdlr, tdhr) !== 0,
        clear_current_interrupt: () => Module._clear_current_interrupt(),
        dma_absorb_periph: (addr, size) => { Module._dma_absorb_periph(scratch, addr, size); return vecU8(); },
        dma_get_all_pending: () => { Module._dma_get_all_pending(scratch); return vecU32Len(H32()[(scratch + 4) >> 2]); },
        dma_get_pending: (index) => { Module._dma_get_pending(scratch, index); return vecU32Len(H32()[(scratch + 4) >> 2]); },
        dma_get_pending_count: () => Module._dma_get_pending_count(),
        dma_pump_all: () => { Module._dma_pump_all(scratch); return vecU32(); },
        dma_push_periph: (addr, data) => {
            const p0 = Module._malloc(data.length); H8().set(data, p0);
            Module._dma_push_periph(addr, p0, data.length);
        },
        dma_set_completed: (stream_idx, success) => Module._dma_set_completed(stream_idx, success ? 1 : 0),
        dma_set_completed_many: (bits) => Module._dma_set_completed_many(bits),
        dma_take_absorbed: (offset, len) => { Module._dma_take_absorbed(scratch, offset, len); return vecU8(); },
        finish_interrupt: (irq) => Module._finish_interrupt(irq),
        get_next_pending_interrupt: () => Module._get_next_pending_interrupt(),
        get_uart_output: () => { Module._get_uart_output(scratch); return str(); },
        gpio_read_input: (port, pin) => Module._gpio_read_input(port, pin) !== 0,
        gpio_read_output: (port, pin) => Module._gpio_read_output(port, pin) !== 0,
        gpio_set_analog: (port, pin, level) => Module._gpio_set_analog(port, pin, level),
        gpio_set_input: (port, pin, value) => Module._gpio_set_input(port, pin, value ? 1 : 0),
        gpio_set_slew: (inst) => Module._gpio_set_slew(inst),
        gpio_take_pin_events: () => { Module._gpio_take_pin_events(scratch); return vecU32(); },
        has_pending_interrupt: () => Module._has_pending_interrupt() !== 0,
        i2c_oled_fb: (peripheral, address) => {
            const [p0, l0] = passString(peripheral);
            Module._i2c_oled_fb(scratch, p0, l0, address);
            return vecU8();
        },
        i2c_oled_writes: (peripheral, address) => {
            const [p0, l0] = passString(peripheral);
            const ret = Module._i2c_oled_writes(p0, l0, address);
            return BigInt.asUintN(64, ret);
        },
        init: () => Module._init(),
        init_svd: (svd_xml) => {
            const [p0, l0] = passString(svd_xml);
            Module._init_svd(p0, l0);
        },
        intr_next: () => Module._intr_next(),
        intr_svc_depth: () => Module._intr_svc_depth(),
        intr_svc_enter: (r0, r1, r2, r3, r12, lr, pc, xpsr, sp) => {
            Module._intr_svc_enter(scratch, r0, r1, r2, r3, r12, lr, pc, xpsr, sp);
            return vecU8();
        },
        intr_svc_leave: () => { Module._intr_svc_leave(scratch); return vecU32(); },
        is_watchdog_reset_requested: () => Module._is_watchdog_reset_requested() !== 0,
        lcd_fb: (peripheral) => {
            const [p0, l0] = passString(peripheral);
            Module._lcd_fb(scratch, p0, l0);
            return vecU8();
        },
        nvic_systick_take: () => Module._nvic_systick_take() !== 0,
        periph_read: (addr, width) => Module._periph_read(addr, width) >>> 0,
        periph_write: (addr, width, value) => Module._periph_write(addr, width, value),
        pwm_duty: (addr, channel) => Module._pwm_duty(addr, channel) >>> 0,
        raise_fault: (kind, addr) => Module._raise_fault(kind, addr),
        register_js_peripheral: (base, size, read, write) => Module._register_js_peripheral(base, size, objIdx(read), objIdx(write)) !== 0,
        reset_ext_devices: () => Module._reset_ext_devices(),
        set_intr_masks: (primask, basepri) => Module._set_intr_masks(primask, basepri),
        step: (primask, basepri) => Module._step(primask, basepri) >>> 0,
        step_batch: (count) => Module._step_batch(count) >>> 0,
        tick: () => Module._tick(),
        touchscreen_set_touch: (peripheral, x, y, pressure) => {
            const [p0, l0] = passString(peripheral);
            Module._touchscreen_set_touch(p0, l0, x, y, pressure);
        },
        uart_rx_byte: (addr, byte) => Module._uart_rx_byte(addr, byte) !== 0,
        uart_rx_pending: (addr) => Module._uart_rx_pending(addr) >>> 0,
    };

    new Function('Module', unicornClassBlock + '; return Module;')(Module);

    // Multiple emulators per process (test batteries) would grow the wasm
    // memory by ~1.9GB PER Unicorn instance: this emscripten build physically
    // allocates the mapped range. Map the big ranges ONCE onto a shared
    // backing buffer (uc_mem_map_ptr) so every instance reuses the memory.
    const bigBackings = new Map();
    const patchBigMaps = () => {
        // The dist class assigns mem_map per-instance inside its constructor,
        // so wrapping the constructor (not the prototype) is required.
        const Orig = Module.Unicorn;
        const BIG = [
            [0x40000000, 0xB0000000], // peripheral bus hole
            [0xE0000000, 0xE1000000], // system control
        ];
        const makePatched = (orig) => function (begin, size, perms) {
            const b = Number(begin), s = Number(size);
            for (const [base, end] of BIG) {
                if (b === base && b + s === end) {
                    let backing = bigBackings.get(s);
                    if (backing === undefined) {
                        backing = Module._malloc(s);
                        bigBackings.set(s, backing);
                    }
                    const handle = Module.getValue(this.handle_ptr, '*');
                    const rc = Module.ccall('uc_mem_map_ptr', 'number',
                        ['pointer', 'number', 'number', 'number', 'number'],
                        [handle, BigInt(b), BigInt(s), perms, backing]);
                    if (rc !== 0) throw new Error(`uc_mem_map_ptr failed: ${rc}`);
                    return;
                }
            }
            return orig.call(this, begin, size, perms);
        };
        Module.Unicorn = function (arch, mode) {
            const u = new Orig(arch, mode);
            u.mem_map = makePatched(u.mem_map);
            return u;
        };
        Module.Unicorn.prototype = Orig.prototype;
    };
    patchBigMaps();

    return { Module, periph };
}
