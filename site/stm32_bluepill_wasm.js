/* @ts-self-types="./stm32_bluepill_wasm.d.ts" */

/**
 * RC sample-and-hold time constant in ADC cycles (1 instr = 1 cycle).
 * @param {number} cycles
 */
export function adc_set_rc_tau(cycles) {
    wasm.adc_set_rc_tau(cycles);
}

/**
 * @param {number} val
 */
export function adc_set_sim_value(val) {
    wasm.adc_set_sim_value(val);
}

/**
 * Add an FSMC NOR/PSRAM memory device backed by `data` (byte image).
 * `name` must be FSMC.BANK1..4 (NE1-4), FSMC.BANK5..6 (NAND), or FSMC.BANK7
 * (PC Card). Must be called before init().
 * @param {string} name
 * @param {Uint8Array} data
 */
export function add_fsmc_bank(name, data) {
    const ptr0 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passArray8ToWasm0(data, wasm.__wbindgen_malloc);
    const len1 = WASM_VECTOR_LEN;
    wasm.add_fsmc_bank(ptr0, len0, ptr1, len1);
}

/**
 * Add an SPI flash device. Must be called before init().
 * @param {string} peripheral
 * @param {number} address
 * @param {Uint8Array} data
 */
export function add_i2c_eeprom(peripheral, address, data) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passArray8ToWasm0(data, wasm.__wbindgen_malloc);
    const len1 = WASM_VECTOR_LEN;
    wasm.add_i2c_eeprom(ptr0, len0, address, ptr1, len1);
}

/**
 * Add an I2C OLED display device (e.g. SSD1306). Must be called before init().
 * @param {string} peripheral
 * @param {number} address
 * @param {number} width
 * @param {number} height
 */
export function add_i2c_oled(peripheral, address, width, height) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    wasm.add_i2c_oled(ptr0, len0, address, width, height);
}

/**
 * Add an SPI LCD display device (e.g. ST7789, ILI9341). Must be called before init().
 * @param {string} peripheral
 * @param {string | null} [cs]
 */
export function add_lcd(peripheral, cs) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    var ptr1 = isLikeNone(cs) ? 0 : passStringToWasm0(cs, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    var len1 = WASM_VECTOR_LEN;
    wasm.add_lcd(ptr0, len0, ptr1, len1);
}

/**
 * Register a software SPI device. Must be called before init().
 * @param {string} name
 * @param {string | null | undefined} cs
 * @param {string} clk
 * @param {string} miso
 * @param {string} mosi
 */
export function add_software_spi(name, cs, clk, miso, mosi) {
    const ptr0 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    var ptr1 = isLikeNone(cs) ? 0 : passStringToWasm0(cs, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    var len1 = WASM_VECTOR_LEN;
    const ptr2 = passStringToWasm0(clk, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len2 = WASM_VECTOR_LEN;
    const ptr3 = passStringToWasm0(miso, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len3 = WASM_VECTOR_LEN;
    const ptr4 = passStringToWasm0(mosi, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len4 = WASM_VECTOR_LEN;
    wasm.add_software_spi(ptr0, len0, ptr1, len1, ptr2, len2, ptr3, len3, ptr4, len4);
}

/**
 * Add an SPI flash device. Must be called before init().
 * @param {string} peripheral
 * @param {number} jedec_id
 * @param {Uint8Array} data
 * @param {string | null} [cs]
 */
export function add_spi_flash(peripheral, jedec_id, data, cs) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passArray8ToWasm0(data, wasm.__wbindgen_malloc);
    const len1 = WASM_VECTOR_LEN;
    var ptr2 = isLikeNone(cs) ? 0 : passStringToWasm0(cs, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    var len2 = WASM_VECTOR_LEN;
    wasm.add_spi_flash(ptr0, len0, jedec_id, ptr1, len1, ptr2, len2);
}

/**
 * Register a touchscreen device. Must be called before init().
 * @param {string} peripheral
 * @param {string | null} [touch_detected_pin]
 * @param {string | null} [cs]
 */
export function add_touchscreen(peripheral, touch_detected_pin, cs) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    var ptr1 = isLikeNone(touch_detected_pin) ? 0 : passStringToWasm0(touch_detected_pin, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    var len1 = WASM_VECTOR_LEN;
    var ptr2 = isLikeNone(cs) ? 0 : passStringToWasm0(cs, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    var len2 = WASM_VECTOR_LEN;
    wasm.add_touchscreen(ptr0, len0, ptr1, len1, ptr2, len2);
}

/**
 * Inject a CAN message into the CAN peripheral at the given address.
 * Returns true if the message was accepted (matched a filter and placed in a FIFO).
 * @param {number} addr
 * @param {number} tir
 * @param {number} tdtr
 * @param {number} tdlr
 * @param {number} tdhr
 * @returns {boolean}
 */
export function can_inject_message(addr, tir, tdtr, tdlr, tdhr) {
    const ret = wasm.can_inject_message(addr, tir, tdtr, tdlr, tdhr);
    return ret !== 0;
}

/**
 * Call after an ISR returns to pop the active priority stack.
 */
export function clear_current_interrupt() {
    wasm.clear_current_interrupt();
}

/**
 * DMA periph->mem pump: pop `size` bytes from the peripheral at `addr` via
 * the normal periph_read path (chunks <= 4, little-endian packed), so JS only
 * writes the result to RAM once per transfer instead of one crossing per chunk.
 * @param {number} addr
 * @param {number} size
 * @returns {Uint8Array}
 */
export function dma_absorb_periph(addr, size) {
    const ret = wasm.dma_absorb_periph(addr, size);
    var v1 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
    return v1;
}

/**
 * @returns {Uint32Array}
 */
export function dma_get_all_pending() {
    const ret = wasm.dma_get_all_pending();
    var v1 = getArrayU32FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
    return v1;
}

/**
 * @param {number} index
 * @returns {Uint32Array}
 */
export function dma_get_pending(index) {
    const ret = wasm.dma_get_pending(index);
    var v1 = getArrayU32FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
    return v1;
}

/**
 * @returns {number}
 */
export function dma_get_pending_count() {
    const ret = wasm.dma_get_pending_count();
    return ret >>> 0;
}

/**
 * Rust-side DMA pump: pops ALL pending transfers, performs the peripheral
 * byte absorb/push internally (periph_read/periph_write chunked), and returns
 * a flat op plan for JS:
 *   [op, a, b, c] quadruples:
 *     op 0 = RAM memcpy (a=src, b=dst, c=size)          -> JS mem_read + mem_write
 *     op 1 = write absorbed bytes (a=dst, b=size, c=off) -> JS mem_write(dma_take_absorbed(off,size))
 *     op 2 = read RAM then push to periph (a=src, b=size, c=periAddr) -> JS mem_read + dma_push_periph
 *     op 3 = done (a=completed stream bits)             -> JS dma_set_completed_many(a)
 * The plan is built in queue order; absorbed bytes land in a side buffer
 * fetched with dma_take_absorbed(). Completion is signaled LAST so DMA IRQs
 * fire only after every RAM move has landed.
 * @returns {Uint32Array}
 */
export function dma_pump_all() {
    const ret = wasm.dma_pump_all();
    var v1 = getArrayU32FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
    return v1;
}

/**
 * DMA mem->periph pump: push `data` bytes into the peripheral at `addr` via
 * the normal periph_write path (chunks <= 4, little-endian unpacked), so JS
 * only reads RAM once per transfer instead of one crossing per chunk.
 * @param {number} addr
 * @param {Uint8Array} data
 */
export function dma_push_periph(addr, data) {
    const ptr0 = passArray8ToWasm0(data, wasm.__wbindgen_malloc);
    const len0 = WASM_VECTOR_LEN;
    wasm.dma_push_periph(addr, ptr0, len0);
}

/**
 * @param {number} stream_idx
 * @param {boolean} success
 */
export function dma_set_completed(stream_idx, success) {
    wasm.dma_set_completed(stream_idx, success);
}

/**
 * @param {number} bits
 */
export function dma_set_completed_many(bits) {
    wasm.dma_set_completed_many(bits);
}

/**
 * Fetch a slice of the bytes absorbed by the last dma_pump_all() (offset,
 * length) so JS can mem_write them into RAM. Clears the whole buffer.
 * @param {number} offset
 * @param {number} len
 * @returns {Uint8Array}
 */
export function dma_take_absorbed(offset, len) {
    const ret = wasm.dma_take_absorbed(offset, len);
    var v1 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
    return v1;
}

/**
 * Call after an ISR returns: pops the active priority stack, and for SysTick
 * (irq == -1) also drains any unconsumed 1ms debt ticks internally (re-pends
 * each), so JS needs no nvic_systick_take loop.
 * @param {number} irq
 */
export function finish_interrupt(irq) {
    wasm.finish_interrupt(irq);
}

/**
 * @returns {number}
 */
export function get_next_pending_interrupt() {
    const ret = wasm.get_next_pending_interrupt();
    return ret;
}

/**
 * Collect UART output since last call.
 * @returns {string}
 */
export function get_uart_output() {
    let deferred1_0;
    let deferred1_1;
    try {
        const ret = wasm.get_uart_output();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
    }
}

/**
 * @param {number} port
 * @param {number} pin
 * @returns {boolean}
 */
export function gpio_read_input(port, pin) {
    const ret = wasm.gpio_read_input(port, pin);
    return ret !== 0;
}

/**
 * @param {number} port
 * @param {number} pin
 * @returns {boolean}
 */
export function gpio_read_output(port, pin) {
    const ret = wasm.gpio_read_output(port, pin);
    return ret !== 0;
}

/**
 * Set an analog wire voltage on a GPIO pin (12-bit, 0xFFFF clears it).
 * ADC channels mapped to the pin then sample this voltage with an RC
 * sample-and-hold model instead of the injected simulation value.
 * @param {number} port
 * @param {number} pin
 * @param {number} level
 */
export function gpio_set_analog(port, pin, level) {
    wasm.gpio_set_analog(port, pin, level);
}

/**
 * @param {number} port
 * @param {number} pin
 * @param {boolean} value
 */
export function gpio_set_input(port, pin, value) {
    wasm.gpio_set_input(port, pin, value);
}

/**
 * Set the GPIO output slew delay in instructions (0 = instant). Affects IDR
 * readback only; device callbacks stay instant.
 * @param {number} inst
 */
export function gpio_set_slew(inst) {
    wasm.gpio_set_slew(inst);
}

/**
 * Drain buffered pin-change events as a flat [port, pin, level, ...] array
 * (chip-driven output level changes only). Cleared on the next init().
 * @returns {Uint32Array}
 */
export function gpio_take_pin_events() {
    const ret = wasm.gpio_take_pin_events();
    var v1 = getArrayU32FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
    return v1;
}

/**
 * Check if any interrupt is pending, respecting PRIMASK/BASEPRI.
 * @returns {boolean}
 */
export function has_pending_interrupt() {
    const ret = wasm.has_pending_interrupt();
    return ret !== 0;
}

/**
 * Read back an I2C OLED display's framebuffer (page-major, 1 byte per column).
 * @param {string} peripheral
 * @param {number} address
 * @returns {Uint8Array}
 */
export function i2c_oled_fb(peripheral, address) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.i2c_oled_fb(ptr0, len0, address);
    var v2 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
    return v2;
}

/**
 * Debug: bytes the I2C OLED device received (should be ~1K+ for a full frame).
 * @param {string} peripheral
 * @param {number} address
 * @returns {bigint}
 */
export function i2c_oled_writes(peripheral, address) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.i2c_oled_writes(ptr0, len0, address);
    return BigInt.asUintN(64, ret);
}

/**
 * Initialize the emulator with hardcoded peripheral map.
 * Must be called after adding all ext devices (add_spi_flash, add_i2c_eeprom).
 * Can be called multiple times to reset emulator state.
 */
export function init() {
    wasm.init();
}

/**
 * Initialize the emulator from an SVD XML string (e.g., STM32F407.svd).
 * Must be called after adding all ext devices (add_spi_flash, add_i2c_eeprom).
 * @param {string} svd_xml
 */
export function init_svd(svd_xml) {
    const ptr0 = passStringToWasm0(svd_xml, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    wasm.init_svd(ptr0, len0);
}

/**
 * Next pending IRQ within the batch budget (like get_next_pending_interrupt,
 * but capped at 64 per step/step_batch so one hot IRQ can't starve others).
 * @returns {number}
 */
export function intr_next() {
    const ret = wasm.intr_next();
    return ret;
}

/**
 * Number of SVC frames currently in flight (guard for the JS catch).
 * @returns {number}
 */
export function intr_svc_depth() {
    const ret = wasm.intr_svc_depth();
    return ret >>> 0;
}

/**
 * Enter an SVC: push a mirror of the interrupted context (depth-capped at 8)
 * and return the 32-byte Cortex-M exception frame to write to the real stack.
 * Empty vec when the cap is hit (SVC ignored, like the old JS guard).
 * Frame layout: [xpsr, pc, lr, r12, r3, r2, r1, r0] little-endian.
 * @param {number} r0
 * @param {number} r1
 * @param {number} r2
 * @param {number} r3
 * @param {number} r12
 * @param {number} lr
 * @param {number} pc
 * @param {number} xpsr
 * @param {number} sp
 * @returns {Uint8Array}
 */
export function intr_svc_enter(r0, r1, r2, r3, r12, lr, pc, xpsr, sp) {
    const ret = wasm.intr_svc_enter(r0, r1, r2, r3, r12, lr, pc, xpsr, sp);
    var v1 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
    return v1;
}

/**
 * Pop the top SVC mirror: [r0, r1, r2, r3, r12, lr, pc, sp]. Empty vec when
 * the stack is empty (no SVC in flight).
 * @returns {Uint32Array}
 */
export function intr_svc_leave() {
    const ret = wasm.intr_svc_leave();
    var v1 = getArrayU32FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
    return v1;
}

/**
 * @returns {boolean}
 */
export function is_watchdog_reset_requested() {
    const ret = wasm.is_watchdog_reset_requested();
    return ret !== 0;
}

/**
 * Read back an SPI LCD display's framebuffer (128x64, 1 byte per pixel).
 * @param {string} peripheral
 * @returns {Uint8Array}
 */
export function lcd_fb(peripheral) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.lcd_fb(ptr0, len0);
    var v2 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
    return v2;
}

/**
 * Re-pend SysTick if unconsumed 1ms ticks remain (fast millis/delay()).
 * @returns {boolean}
 */
export function nvic_systick_take() {
    const ret = wasm.nvic_systick_take();
    return ret !== 0;
}

/**
 * @param {number} addr
 * @param {number} width
 * @returns {number}
 */
export function periph_read(addr, width) {
    const ret = wasm.periph_read(addr, width);
    return ret >>> 0;
}

/**
 * @param {number} addr
 * @param {number} width
 * @param {number} value
 */
export function periph_write(addr, width, value) {
    wasm.periph_write(addr, width, value);
}

/**
 * Current PWM duty (0-100) of a timer channel; 0 if addr is not a timer.
 * @param {number} addr
 * @param {number} channel
 * @returns {number}
 */
export function pwm_duty(addr, channel) {
    const ret = wasm.pwm_duty(addr, channel);
    return ret >>> 0;
}

/**
 * Raise a fault (kind: 0=fetch, 1=data read, 2=data write, 3=undef instruction).
 * Sets SCB CFSR/HFSR/BFAR and pends the fault exception (with SHCSR escalation
 * to HardFault when the specific fault handler is disabled).
 * @param {number} kind
 * @param {number} addr
 */
export function raise_fault(kind, addr) {
    wasm.raise_fault(kind, addr);
}

/**
 * Register a peripheral implemented entirely in JS (rp2040js-style custom
 * chip). Callbacks are invoked with `(addr, size)` / `(addr, value, size)`
 * where addr is the ABSOLUTE access address. Requires init()/init_svd() first;
 * last registration wins on overlap. Returns false if not initialized.
 * @param {number} base
 * @param {number} size
 * @param {Function} read
 * @param {Function} write
 * @returns {boolean}
 */
export function register_js_peripheral(base, size, read, write) {
    const ret = wasm.register_js_peripheral(base, size, read, write);
    return ret !== 0;
}

/**
 * Clear all registered ext devices (spi flash, eeprom, oled, lcd, touchscreen,
 * fsmc, software spi). Call BEFORE adding devices for a new emulator instance —
 * otherwise devices from a previous init (stale CS pins reading low on the
 * fresh GPIO) shadow the new ones during SPI/I2C device selection.
 */
export function reset_ext_devices() {
    wasm.reset_ext_devices();
}

/**
 * Set PRIMASK and BASEPRI values from Unicorn CPU state.
 * @param {number} primask
 * @param {number} basepri
 */
export function set_intr_masks(primask, basepri) {
    wasm.set_intr_masks(primask, basepri);
}

/**
 * Combined per-instruction step: sets masks, ticks peripherals, checks conditions.
 * Returns: 0=continue, 1=watchdog reset, 2=DMA pending, 3=interrupt pending.
 * @param {number} primask
 * @param {number} basepri
 * @returns {number}
 */
export function step(primask, basepri) {
    const ret = wasm.step(primask, basepri);
    return ret >>> 0;
}

/**
 * Process a batch of N instructions in one WASM call.
 * Peripheral ticks are instruction-delta based (each reads INSTRUCTION_COUNT
 * and accumulates elapsed time), so one tick after advancing the count by N
 * is equivalent to N per-instruction ticks — but ~N× cheaper (tick() was
 * ~55% of runtime via 100K iterations per batch).
 * Returns: 0=continue, 1=watchdog reset.
 * @param {number} count
 * @returns {number}
 */
export function step_batch(count) {
    const ret = wasm.step_batch(count);
    return ret >>> 0;
}

export function tick() {
    wasm.tick();
}

/**
 * Set touch coordinates on a touchscreen device. Must be called after init().
 * @param {string} peripheral
 * @param {number} x
 * @param {number} y
 * @param {number} pressure
 */
export function touchscreen_set_touch(peripheral, x, y, pressure) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    wasm.touchscreen_set_touch(ptr0, len0, x, y, pressure);
}

/**
 * Inject a received byte into the UART at the given peripheral base address.
 * Returns true if a peripheral was found at that address.
 * @param {number} addr
 * @param {number} byte
 * @returns {boolean}
 */
export function uart_rx_byte(addr, byte) {
    const ret = wasm.uart_rx_byte(addr, byte);
    return ret !== 0;
}

/**
 * Number of unread bytes still queued in the UART RX buffer at addr.
 * @param {number} addr
 * @returns {number}
 */
export function uart_rx_pending(addr) {
    const ret = wasm.uart_rx_pending(addr);
    return ret >>> 0;
}
function __wbg_get_imports() {
    const import0 = {
        __proto__: null,
        __wbg___wbindgen_number_get_394265ed1e1b84ee: function(arg0, arg1) {
            const obj = arg1;
            const ret = typeof(obj) === 'number' ? obj : undefined;
            getDataViewMemory0().setFloat64(arg0 + 8 * 1, isLikeNone(ret) ? 0 : ret, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, !isLikeNone(ret), true);
        },
        __wbg___wbindgen_throw_344f42d3211c4765: function(arg0, arg1) {
            throw new Error(getStringFromWasm0(arg0, arg1));
        },
        __wbg_call_44b7209e1e252e6a: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4) {
            const ret = arg0.call(arg1, arg2, arg3, arg4);
            return ret;
        }, arguments); },
        __wbg_call_e3b662382210db98: function() { return handleError(function (arg0, arg1, arg2, arg3) {
            const ret = arg0.call(arg1, arg2, arg3);
            return ret;
        }, arguments); },
        __wbg_error_a6fa202b58aa1cd3: function(arg0, arg1) {
            let deferred0_0;
            let deferred0_1;
            try {
                deferred0_0 = arg0;
                deferred0_1 = arg1;
                console.error(getStringFromWasm0(arg0, arg1));
            } finally {
                wasm.__wbindgen_free(deferred0_0, deferred0_1, 1);
            }
        },
        __wbg_new_227d7c05414eb861: function() {
            const ret = new Error();
            return ret;
        },
        __wbg_stack_3b0d974bbf31e44f: function(arg0, arg1) {
            const ret = arg1.stack;
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbindgen_cast_0000000000000001: function(arg0) {
            // Cast intrinsic for `F64 -> Externref`.
            const ret = arg0;
            return ret;
        },
        __wbindgen_init_externref_table: function() {
            const table = wasm.__wbindgen_externrefs;
            const offset = table.grow(4);
            table.set(0, undefined);
            table.set(offset + 0, undefined);
            table.set(offset + 1, null);
            table.set(offset + 2, true);
            table.set(offset + 3, false);
        },
    };
    return {
        __proto__: null,
        "./stm32_bluepill_wasm_bg.js": import0,
    };
}

function addToExternrefTable0(obj) {
    const idx = wasm.__externref_table_alloc();
    wasm.__wbindgen_externrefs.set(idx, obj);
    return idx;
}

function getArrayU32FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint32ArrayMemory0().subarray(ptr / 4, ptr / 4 + len);
}

function getArrayU8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
}

let cachedDataViewMemory0 = null;
function getDataViewMemory0() {
    if (cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true || (cachedDataViewMemory0.buffer.detached === undefined && cachedDataViewMemory0.buffer !== wasm.memory.buffer)) {
        cachedDataViewMemory0 = new DataView(wasm.memory.buffer);
    }
    return cachedDataViewMemory0;
}

function getStringFromWasm0(ptr, len) {
    return decodeText(ptr >>> 0, len);
}

let cachedUint32ArrayMemory0 = null;
function getUint32ArrayMemory0() {
    if (cachedUint32ArrayMemory0 === null || cachedUint32ArrayMemory0.byteLength === 0) {
        cachedUint32ArrayMemory0 = new Uint32Array(wasm.memory.buffer);
    }
    return cachedUint32ArrayMemory0;
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

function handleError(f, args) {
    try {
        return f.apply(this, args);
    } catch (e) {
        const idx = addToExternrefTable0(e);
        wasm.__wbindgen_exn_store(idx);
    }
}

function isLikeNone(x) {
    return x === undefined || x === null;
}

function passArray8ToWasm0(arg, malloc) {
    const ptr = malloc(arg.length * 1, 1) >>> 0;
    getUint8ArrayMemory0().set(arg, ptr / 1);
    WASM_VECTOR_LEN = arg.length;
    return ptr;
}

function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
cachedTextDecoder.decode();
const MAX_SAFARI_DECODE_BYTES = 2146435072;
let numBytesDecoded = 0;
function decodeText(ptr, len) {
    numBytesDecoded += len;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
        cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
        cachedTextDecoder.decode();
        numBytesDecoded = len;
    }
    return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

const cachedTextEncoder = new TextEncoder();

if (!('encodeInto' in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function (arg, view) {
        const buf = cachedTextEncoder.encode(arg);
        view.set(buf);
        return {
            read: arg.length,
            written: buf.length
        };
    };
}

let WASM_VECTOR_LEN = 0;

let wasmModule, wasmInstance, wasm;
function __wbg_finalize_init(instance, module) {
    wasmInstance = instance;
    wasm = instance.exports;
    wasmModule = module;
    cachedDataViewMemory0 = null;
    cachedUint32ArrayMemory0 = null;
    cachedUint8ArrayMemory0 = null;
    wasm.__wbindgen_start();
    return wasm;
}

async function __wbg_load(module, imports) {
    if (typeof Response === 'function' && module instanceof Response) {
        if (typeof WebAssembly.instantiateStreaming === 'function') {
            try {
                return await WebAssembly.instantiateStreaming(module, imports);
            } catch (e) {
                const validResponse = module.ok && expectedResponseType(module.type);

                if (validResponse && module.headers.get('Content-Type') !== 'application/wasm') {
                    console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                } else { throw e; }
            }
        }

        const bytes = await module.arrayBuffer();
        return await WebAssembly.instantiate(bytes, imports);
    } else {
        const instance = await WebAssembly.instantiate(module, imports);

        if (instance instanceof WebAssembly.Instance) {
            return { instance, module };
        } else {
            return instance;
        }
    }

    function expectedResponseType(type) {
        switch (type) {
            case 'basic': case 'cors': case 'default': return true;
        }
        return false;
    }
}

function initSync(module) {
    if (wasm !== undefined) return wasm;


    if (module !== undefined) {
        if (Object.getPrototypeOf(module) === Object.prototype) {
            ({module} = module)
        } else {
            console.warn('using deprecated parameters for `initSync()`; pass a single object instead')
        }
    }

    const imports = __wbg_get_imports();
    if (!(module instanceof WebAssembly.Module)) {
        module = new WebAssembly.Module(module);
    }
    const instance = new WebAssembly.Instance(module, imports);
    return __wbg_finalize_init(instance, module);
}

async function __wbg_init(module_or_path) {
    if (wasm !== undefined) return wasm;


    if (module_or_path !== undefined) {
        if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
            ({module_or_path} = module_or_path)
        } else {
            console.warn('using deprecated parameters for the initialization function; pass a single object instead')
        }
    }

    if (module_or_path === undefined) {
        module_or_path = new URL('stm32_bluepill_wasm_bg.wasm', import.meta.url);
    }
    const imports = __wbg_get_imports();

    if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
        module_or_path = fetch(module_or_path);
    }

    const { instance, module } = await __wbg_load(await module_or_path, imports);

    return __wbg_finalize_init(instance, module);
}

export { initSync, __wbg_init as default };
