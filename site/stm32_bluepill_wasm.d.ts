/* tslint:disable */
/* eslint-disable */

/**
 * RC sample-and-hold time constant in ADC cycles (1 instr = 1 cycle).
 */
export function adc_set_rc_tau(cycles: number): void;

export function adc_set_sim_value(val: number): void;

/**
 * Add an FSMC NOR/PSRAM memory device backed by `data` (byte image).
 * `name` must be FSMC.BANK1..4 (NE1-4), FSMC.BANK5..6 (NAND), or FSMC.BANK7
 * (PC Card). Must be called before init().
 */
export function add_fsmc_bank(name: string, data: Uint8Array): void;

/**
 * Add an SPI flash device. Must be called before init().
 */
export function add_i2c_eeprom(peripheral: string, address: number, data: Uint8Array): void;

/**
 * Add an I2C OLED display device (e.g. SSD1306). Must be called before init().
 */
export function add_i2c_oled(peripheral: string, address: number, width: number, height: number): void;

/**
 * Add an SPI LCD display device (e.g. ST7789, ILI9341). Must be called before init().
 */
export function add_lcd(peripheral: string, cs?: string | null): void;

/**
 * Register a software SPI device. Must be called before init().
 */
export function add_software_spi(name: string, cs: string | null | undefined, clk: string, miso: string, mosi: string): void;

/**
 * Add an SPI flash device. Must be called before init().
 */
export function add_spi_flash(peripheral: string, jedec_id: number, data: Uint8Array, cs?: string | null): void;

/**
 * Register a touchscreen device. Must be called before init().
 */
export function add_touchscreen(peripheral: string, touch_detected_pin?: string | null, cs?: string | null): void;

/**
 * Inject a CAN message into the CAN peripheral at the given address.
 * Returns true if the message was accepted (matched a filter and placed in a FIFO).
 */
export function can_inject_message(addr: number, tir: number, tdtr: number, tdlr: number, tdhr: number): boolean;

/**
 * Call after an ISR returns to pop the active priority stack.
 */
export function clear_current_interrupt(): void;

/**
 * DMA periph->mem pump: pop `size` bytes from the peripheral at `addr` via
 * the normal periph_read path (chunks <= 4, little-endian packed), so JS only
 * writes the result to RAM once per transfer instead of one crossing per chunk.
 */
export function dma_absorb_periph(addr: number, size: number): Uint8Array;

export function dma_get_all_pending(): Uint32Array;

export function dma_get_pending(index: number): Uint32Array;

export function dma_get_pending_count(): number;

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
 */
export function dma_pump_all(): Uint32Array;

/**
 * DMA mem->periph pump: push `data` bytes into the peripheral at `addr` via
 * the normal periph_write path (chunks <= 4, little-endian unpacked), so JS
 * only reads RAM once per transfer instead of one crossing per chunk.
 */
export function dma_push_periph(addr: number, data: Uint8Array): void;

export function dma_set_completed(stream_idx: number, success: boolean): void;

export function dma_set_completed_many(bits: number): void;

/**
 * Fetch a slice of the bytes absorbed by the last dma_pump_all() (offset,
 * length) so JS can mem_write them into RAM. Clears the whole buffer.
 */
export function dma_take_absorbed(offset: number, len: number): Uint8Array;

/**
 * Drain virtual-peripheral transaction events as a flat i32 array.
 * Encoding (discriminant first):
 *   1 SpiTransfer  [1, channel, txLen, rxLen, tx bytes..., rx bytes...]
 *   2 I2cStart     [2, channel, addr]
 *   3 I2cWrite     [3, channel, byte]
 *   4 I2cRead      [4, channel]
 *   5 I2cStop      [5, channel]
 *   6 UartTx       [6, usart, byte]
 *   7 ExtiEdge     [7, line]
 *   8 AdcDone      [8, adc, chan]
 *   9 TimUpdate    [9, tim]
 *  10 DacWrite     [10, chan, value]
 *  11 CrcResult    [11, value]
 *  12 RtcAlarm     [12, alarm]
 *  13 WdogReset    [13, which]   (1=IWDG, 2=WWDG)
 *  14 CanTx        [14, can, id, len, d0..d7]
 *  15 CanRx        [15, can, id, len, d0..d7]
 *  16 TimCapture    [16, tim, ch, value]   (input-capture latch)
 *  17 FsmcAccess    [17, bank, offset, write, size, value]
 */
export function drain_events(): Int32Array;

/**
 * Call after an ISR returns: pops the active priority stack, and for SysTick
 * (irq == -1) also drains any unconsumed 1ms debt ticks internally (re-pends
 * each), so JS needs no nvic_systick_take loop.
 */
export function finish_interrupt(irq: number): void;

/**
 * Read a byte from an FSMC backing image (bypasses the peripheral bus).
 * Returns the byte value (0..255) or -1 if the bank/offset is invalid.
 */
export function fsmc_read_byte(name: string, offset: number): number;

/**
 * Write a byte directly into an FSMC backing image (bypasses the peripheral
 * bus — no events, no side effects).  Returns true on success.
 * Use this from JS virtual peripherals to feed read-back data to the MCU.
 */
export function fsmc_write_byte(name: string, offset: number, value: number): boolean;

export function get_next_pending_interrupt(): number;

/**
 * Collect UART output since last call.
 */
export function get_uart_output(): string;

export function gpio_read_input(port: number, pin: number): boolean;

export function gpio_read_output(port: number, pin: number): boolean;

/**
 * Set an analog wire voltage on a GPIO pin (12-bit, 0xFFFF clears it).
 * ADC channels mapped to the pin then sample this voltage with an RC
 * sample-and-hold model instead of the injected simulation value.
 */
export function gpio_set_analog(port: number, pin: number, level: number): void;

export function gpio_set_input(port: number, pin: number, value: boolean): void;

/**
 * Set the GPIO output slew delay in instructions (0 = instant). Affects IDR
 * readback only; device callbacks stay instant.
 */
export function gpio_set_slew(inst: number): void;

/**
 * Drain buffered pin-change events as a flat [port, pin, level, ...] array
 * (chip-driven output level changes only). Cleared on the next init().
 */
export function gpio_take_pin_events(): Uint32Array;

/**
 * Check if any interrupt is pending, respecting PRIMASK/BASEPRI.
 */
export function has_pending_interrupt(): boolean;

/**
 * Queue injected RX bytes for an I2C channel (virtual device -> MCU).
 */
export function i2c_inject_rx(channel: number, bytes: Uint8Array): void;

/**
 * Read back an I2C OLED display's framebuffer (page-major, 1 byte per column).
 */
export function i2c_oled_fb(peripheral: string, address: number): Uint8Array;

/**
 * Debug: bytes the I2C OLED device received (should be ~1K+ for a full frame).
 */
export function i2c_oled_writes(peripheral: string, address: number): bigint;

/**
 * Initialize the emulator with hardcoded peripheral map.
 * Must be called after adding all ext devices (add_spi_flash, add_i2c_eeprom).
 * Can be called multiple times to reset emulator state.
 */
export function init(): void;

/**
 * Initialize the emulator from an SVD XML string (e.g., STM32F407.svd).
 * Must be called after adding all ext devices (add_spi_flash, add_i2c_eeprom).
 */
export function init_svd(svd_xml: string): void;

/**
 * Next pending IRQ within the batch budget (like get_next_pending_interrupt,
 * but capped at 64 per step/step_batch so one hot IRQ can't starve others).
 */
export function intr_next(): number;

/**
 * Number of SVC frames currently in flight (guard for the JS catch).
 */
export function intr_svc_depth(): number;

/**
 * Enter an SVC: push a mirror of the interrupted context (depth-capped at 8)
 * and return the 32-byte Cortex-M exception frame to write to the real stack.
 * Empty vec when the cap is hit (SVC ignored, like the old JS guard).
 * Frame layout: [xpsr, pc, lr, r12, r3, r2, r1, r0] little-endian.
 */
export function intr_svc_enter(r0: number, r1: number, r2: number, r3: number, r12: number, lr: number, pc: number, xpsr: number, sp: number): Uint8Array;

/**
 * Pop the top SVC mirror: [r0, r1, r2, r3, r12, lr, pc, sp, xpsr]. Empty vec
 * when the stack is empty (no SVC in flight).
 *
 * xPSR is part of the restore for the same reason it is on the IRQ path: the
 * handler's own emu_start clobbers APSR, so a cmp/beq pair split across the
 * SVC would otherwise evaluate with the handler's flags.
 */
export function intr_svc_leave(): Uint32Array;

export function is_watchdog_reset_requested(): boolean;

/**
 * Read back an SPI LCD display's framebuffer (128x64, 1 byte per pixel).
 */
export function lcd_fb(peripheral: string): Uint8Array;

export function periph_read(addr: number, width: number): number;

export function periph_write(addr: number, width: number, value: number): void;

/**
 * One-call batch processor: advance the instruction count, reset the IRQ
 * dispatch budget, tick all peripherals, then report watchdog status and
 * whether any IRQ is pending — so JS needs one crossing per batch instead of
 * three (step_batch + a pending probe + the watchdog poll).
 * Returns:
 *   0x8000_0000  = watchdog reset requested (stop the run)
 *   0x4000_0000  = at least one IRQ pending (dispatch via intr_next loop)
 *   0            = nothing pending
 * The pending probe is EXACTLY equivalent to the first intr_next() call
 * (same INTR_MASK statics + same find_highest_pending), minus the pop —
 * the actual pop still happens in JS after processDma, preserving dispatch
 * order. Watchdog requests made *during* IRQ handlers are still caught by
 * the JS is_watchdog_reset_requested() check after processInterrupts.
 */
export function process_batch(count: number): number;

/**
 * Current PWM duty (0-100) of a timer channel; 0 if addr is not a timer.
 */
export function pwm_duty(addr: number, channel: number): number;

/**
 * Raise a fault (kind: 0=fetch, 1=data read, 2=data write, 3=undef instruction).
 * Sets SCB CFSR/HFSR/BFAR and pends the fault exception (with SHCSR escalation
 * to HardFault when the specific fault handler is disabled).
 */
export function raise_fault(kind: number, addr: number): void;

/**
 * Register a peripheral implemented entirely in JS (rp2040js-style custom
 * chip). Callbacks are invoked with `(addr, size)` / `(addr, value, size)`
 * where addr is the ABSOLUTE access address. Requires init()/init_svd() first;
 * last registration wins on overlap. Returns false if not initialized.
 */
export function register_js_peripheral(base: number, size: number, read: Function, write: Function): boolean;

/**
 * Clear all registered ext devices (spi flash, eeprom, oled, lcd, touchscreen,
 * fsmc, software spi). Call BEFORE adding devices for a new emulator instance —
 * otherwise devices from a previous init (stale CS pins reading low on the
 * fresh GPIO) shadow the new ones during SPI/I2C device selection.
 */
export function reset_ext_devices(): void;

/**
 * Set PRIMASK and BASEPRI values from Unicorn CPU state.
 */
export function set_intr_masks(primask: number, basepri: number): void;

/**
 * Queue injected MISO bytes for a SPI channel (virtual device -> MCU).
 */
export function spi_inject_miso(channel: number, bytes: Uint8Array): void;

/**
 * Combined per-instruction step: sets masks, ticks peripherals, checks conditions.
 * Returns: 0=continue, 1=watchdog reset, 2=DMA pending, 3=interrupt pending.
 */
export function step(primask: number, basepri: number): number;

/**
 * Process a batch of N instructions in one WASM call.
 * Peripheral ticks are instruction-delta based (each reads INSTRUCTION_COUNT
 * and accumulates elapsed time), so one tick after advancing the count by N
 * is equivalent to N per-instruction ticks — but ~N× cheaper (tick() was
 * ~55% of runtime via 100K iterations per batch).
 * Returns: 0=continue, 1=watchdog reset.
 */
export function step_batch(count: number): number;

export function tick(): void;

/**
 * Set touch coordinates on a touchscreen device. Must be called after init().
 */
export function touchscreen_set_touch(peripheral: string, x: number, y: number, pressure: number): void;

/**
 * Inject a received byte into the UART at the given peripheral base address.
 * Returns true if a peripheral was found at that address.
 */
export function uart_rx_byte(addr: number, byte: number): boolean;

/**
 * Number of unread bytes still queued in the UART RX buffer at addr.
 */
export function uart_rx_pending(addr: number): number;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly adc_set_rc_tau: (a: number) => void;
    readonly adc_set_sim_value: (a: number) => void;
    readonly add_fsmc_bank: (a: number, b: number, c: number, d: number) => void;
    readonly add_i2c_eeprom: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly add_i2c_oled: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly add_lcd: (a: number, b: number, c: number, d: number) => void;
    readonly add_software_spi: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number) => void;
    readonly add_spi_flash: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => void;
    readonly add_touchscreen: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
    readonly can_inject_message: (a: number, b: number, c: number, d: number, e: number) => number;
    readonly clear_current_interrupt: () => void;
    readonly dma_absorb_periph: (a: number, b: number) => [number, number];
    readonly dma_get_all_pending: () => [number, number];
    readonly dma_get_pending: (a: number) => [number, number];
    readonly dma_get_pending_count: () => number;
    readonly dma_pump_all: () => [number, number];
    readonly dma_push_periph: (a: number, b: number, c: number) => void;
    readonly dma_set_completed: (a: number, b: number) => void;
    readonly dma_set_completed_many: (a: number) => void;
    readonly dma_take_absorbed: (a: number, b: number) => [number, number];
    readonly drain_events: () => [number, number];
    readonly finish_interrupt: (a: number) => void;
    readonly fsmc_read_byte: (a: number, b: number, c: number) => number;
    readonly fsmc_write_byte: (a: number, b: number, c: number, d: number) => number;
    readonly get_next_pending_interrupt: () => number;
    readonly get_uart_output: () => [number, number];
    readonly gpio_read_input: (a: number, b: number) => number;
    readonly gpio_read_output: (a: number, b: number) => number;
    readonly gpio_set_analog: (a: number, b: number, c: number) => void;
    readonly gpio_set_input: (a: number, b: number, c: number) => void;
    readonly gpio_set_slew: (a: number) => void;
    readonly gpio_take_pin_events: () => [number, number];
    readonly has_pending_interrupt: () => number;
    readonly i2c_inject_rx: (a: number, b: number, c: number) => void;
    readonly i2c_oled_fb: (a: number, b: number, c: number) => [number, number];
    readonly i2c_oled_writes: (a: number, b: number, c: number) => bigint;
    readonly init: () => void;
    readonly init_svd: (a: number, b: number) => void;
    readonly intr_next: () => number;
    readonly intr_svc_depth: () => number;
    readonly intr_svc_enter: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number) => [number, number];
    readonly intr_svc_leave: () => [number, number];
    readonly is_watchdog_reset_requested: () => number;
    readonly lcd_fb: (a: number, b: number) => [number, number];
    readonly periph_read: (a: number, b: number) => number;
    readonly periph_write: (a: number, b: number, c: number) => void;
    readonly process_batch: (a: number) => number;
    readonly pwm_duty: (a: number, b: number) => number;
    readonly raise_fault: (a: number, b: number) => void;
    readonly register_js_peripheral: (a: number, b: number, c: any, d: any) => number;
    readonly reset_ext_devices: () => void;
    readonly set_intr_masks: (a: number, b: number) => void;
    readonly spi_inject_miso: (a: number, b: number, c: number) => void;
    readonly step: (a: number, b: number) => number;
    readonly step_batch: (a: number) => number;
    readonly tick: () => void;
    readonly touchscreen_set_touch: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly uart_rx_byte: (a: number, b: number) => number;
    readonly uart_rx_pending: (a: number) => number;
    readonly __wbindgen_exn_store: (a: number) => void;
    readonly __externref_table_alloc: () => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
