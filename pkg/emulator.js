import initPeriph, { init, periph_read, periph_write, tick, get_next_pending_interrupt, dma_get_pending_count, dma_get_pending, dma_set_completed, gpio_read_output, gpio_set_input, gpio_read_input, is_watchdog_reset_requested, add_spi_flash, add_i2c_eeprom } from './stm32_periph_wasm.js';

const PERIPH_RANGES = [
    [0x40000000, 0xB0000000],
    [0xE0000000, 0xE1000000],
];

const MEM_READ = 16;
const MEM_WRITE = 17;

const EXC_RETURN_MSP = 0xFFFFFFF9;

function getMUnicorn() {
    if (typeof window !== 'undefined' && window.MUnicorn) return window.MUnicorn;
    throw new Error('unicorn_arm.js must be loaded before emulator.js. Add <script src="unicorn_arm.js"></script>');
}

export async function createEmulator(opts) {
    const { firmware, flash_size = 0x100000, ram_size = 0x20000, vector_table = 0x08000000, ext_devices = {} } = opts;

    const MUnicorn = getMUnicorn();
    const Module = await MUnicorn();
    await initPeriph();

    for (const flash_cfg of (ext_devices.spi_flash || [])) {
        add_spi_flash(flash_cfg.peripheral, flash_cfg.jedec_id, flash_cfg.data, flash_cfg.cs ?? null);
    }
    for (const eeprom_cfg of (ext_devices.i2c_eeprom || [])) {
        add_i2c_eeprom(eeprom_cfg.peripheral, eeprom_cfg.address, eeprom_cfg.data);
    }
    init();

    const uc = new Module.Unicorn(Module.ARCH_ARM, Module.MODE_MCLASS | Module.MODE_LITTLE_ENDIAN);

    const flash_addr = vector_table & ~0x1FFFF;
    uc.mem_map(flash_addr, flash_size, Module.PERM_ALL);
    uc.mem_write(flash_addr, firmware);

    uc.mem_map(0x20000000, ram_size, Module.PERM_ALL);

    for (const [start, end] of PERIPH_RANGES) {
        uc.mem_map(start, end - start, Module.PERM_READ | Module.PERM_WRITE);
    }

    let dt = new DataView(new ArrayBuffer(8));
    const read32 = (addr) => {
        const b = uc.mem_read(BigInt(addr), 4);
        dt = new DataView(b.buffer, b.byteOffset, b.byteLength);
        return dt.getUint32(0, true);
    };
    const write32 = (addr, val) => {
        const b = new Uint8Array(4);
        dt = new DataView(b.buffer);
        dt.setUint32(0, val, true);
        uc.mem_write(BigInt(addr), b);
    };

    const sp_init = read32(vector_table);
    const pc_init = read32(vector_table + 4);

    uc.reg_write_i32(Module.ARM_REG_SP, sp_init);
    uc.reg_write_i32(Module.ARM_REG_PC, pc_init | 1);

    const memReadHook = (handle, type, address, size, value, user_data) => {
        const addr32 = Number(address);
        const val = periph_read(addr32, size) >>> 0;
        const bytes = new Uint8Array(size);
        for (let i = 0; i < size; i++) {
            bytes[i] = (val >> (i * 8)) & 0xFF;
        }
        const buf_ptr = Module._malloc(size);
        Module.writeArrayToMemory(bytes, buf_ptr);
        Module.ccall("uc_mem_write", "number",
            ["pointer", "number", "pointer", "number"],
            [handle, address, buf_ptr, BigInt(size)]);
        Module._free(buf_ptr);
    };

    const memWriteHook = (handle, type, address, size, value, user_data) => {
        periph_write(Number(address), size, value >>> 0);
    };

    for (const [start, end] of PERIPH_RANGES) {
        uc.hook_add(Module.HOOK_MEM_READ, memReadHook, null, start, end);
        uc.hook_add(Module.HOOK_MEM_WRITE, memWriteHook, null, start, end);
    }

    let stopRequested = false;
    let instCount = 0n;

    const tickInterval = 1000;
    const maxBatchInst = 100000;

    const enterInterrupt = (irq_num) => {
        const sp = uc.reg_read_i32(Module.ARM_REG_SP);
        const pc = uc.reg_read_i32(Module.ARM_REG_PC);
        const lr = uc.reg_read_i32(Module.ARM_REG_LR);
        const xpsr = uc.reg_read_i32(Module.ARM_REG_XPSR);
        const r0 = uc.reg_read_i32(Module.ARM_REG_R0);
        const r1 = uc.reg_read_i32(Module.ARM_REG_R1);
        const r2 = uc.reg_read_i32(Module.ARM_REG_R2);
        const r3 = uc.reg_read_i32(Module.ARM_REG_R3);
        const r12 = uc.reg_read_i32(Module.ARM_REG_R12);

        const stack_frame = new Uint8Array(32);
        const sv = new DataView(stack_frame.buffer);
        sv.setUint32(0, xpsr, true);
        sv.setUint32(4, pc, true);
        sv.setUint32(8, lr, true);
        sv.setUint32(12, r12, true);
        sv.setUint32(16, r3, true);
        sv.setUint32(20, r2, true);
        sv.setUint32(24, r1, true);
        sv.setUint32(28, r0, true);

        const new_sp = sp - 32;
        uc.mem_write(BigInt(new_sp), stack_frame);
        uc.reg_write_i32(Module.ARM_REG_SP, new_sp);

        const vector_addr = vector_table + 4 + irq_num * 4;
        const handler_pc = read32(vector_addr);
        uc.reg_write_i32(Module.ARM_REG_LR, EXC_RETURN_MSP);
        uc.reg_write_i32(Module.ARM_REG_PC, handler_pc);
    };

    const returnFromInterrupt = () => {
        const sp = uc.reg_read_i32(Module.ARM_REG_SP);
        const stack_frame = uc.mem_read(BigInt(sp), 32);
        const sv = new DataView(stack_frame.buffer, stack_frame.byteOffset, stack_frame.byteLength);
        uc.reg_write_i32(Module.ARM_REG_R0, sv.getUint32(28, true));
        uc.reg_write_i32(Module.ARM_REG_R1, sv.getUint32(24, true));
        uc.reg_write_i32(Module.ARM_REG_R2, sv.getUint32(20, true));
        uc.reg_write_i32(Module.ARM_REG_R3, sv.getUint32(16, true));
        uc.reg_write_i32(Module.ARM_REG_R12, sv.getUint32(12, true));
        uc.reg_write_i32(Module.ARM_REG_LR, sv.getUint32(8, true));
        uc.reg_write_i32(Module.ARM_REG_PC, sv.getUint32(4, true) | 1);
        uc.reg_write_i32(Module.ARM_REG_SP, sp + 32);
    };

    const intrHook = (handle, intno, user_data) => {
        if (intno === 8) {
            returnFromInterrupt();
        } else {
            console.warn('Unhandled exception:', intno);
        }
    };
    uc.hook_add(Module.HOOK_INTR, intrHook, null);

    const codeHook = (handle, address, size, user_data) => {
        instCount++;
        tick();

        if (is_watchdog_reset_requested()) {
            stopRequested = true;
            uc.emu_stop();
            return;
        }

        if (instCount % BigInt(tickInterval) === 0n) {
            const irq = get_next_pending_interrupt();
            if (irq >= 0) {
                uc.emu_stop();
            }
        }
    };
    uc.hook_add(Module.HOOK_CODE, codeHook, null);

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

            const DmaDir_Read = 0;
            const DmaDir_Write = 1;
            const DmaDir_MemCopy = 2;

            try {
                if (dir === DmaDir_MemCopy) {
                    const data = uc.mem_read(BigInt(src), size);
                    uc.mem_write(BigInt(dst), data);
                } else if (dir === DmaDir_Read) {
                    const data = uc.mem_read(BigInt(src), size);
                    if (peripheral) {
                        for (let j = 0; j < size; j += 4) {
                            const chunk = Math.min(4, size - j);
                            let val = 0;
                            for (let k = 0; k < chunk; k++) {
                                val |= data[j + k] << (k * 8);
                            }
                            periph_write(peri_addr, chunk, val);
                        }
                    } else {
                        uc.mem_write(BigInt(dst), data);
                    }
                } else if (dir === DmaDir_Write) {
                    if (peripheral) {
                        for (let j = 0; j < size; j += 4) {
                            const chunk = Math.min(4, size - j);
                            const val = periph_read(peri_addr, chunk);
                            const bytes = new Uint8Array(chunk);
                            for (let k = 0; k < chunk; k++) {
                                bytes[k] = (val >> (k * 8)) & 0xFF;
                            }
                            uc.mem_write(BigInt(dst + j), bytes);
                        }
                    } else {
                        const data = uc.mem_read(BigInt(src), size);
                        uc.mem_write(BigInt(dst), data);
                    }
                }
            } catch (e) {
                console.warn('DMA transfer failed:', e);
            }
            dma_set_completed(stream, true);
        }
    };

    const processInterrupts = () => {
        while (!stopRequested) {
            const irq = get_next_pending_interrupt();
            if (irq < 0) break;
            enterInterrupt(irq);
            try {
                uc.emu_start(BigInt(uc.reg_read_i32(Module.ARM_REG_PC)), 0n, 0n, maxBatchInst);
            } catch (e) {
                if (!stopRequested) {
                    console.warn('Interrupt handler error:', e);
                    break;
                }
            }
            processDma();
        }
    };

    return {
        uc, Module, read32, write32,

        step(max_inst = maxBatchInst) {
            processDma();
            const current_pc = uc.reg_read_i32(Module.ARM_REG_PC);
            try {
                uc.emu_start(BigInt(current_pc), 0n, 0n, max_inst);
            } catch (e) {
                const msg = String(e);
                if (msg.includes('UC_ERR_READ_UNMAPPED') || msg.includes('UC_ERR_FETCH_UNMAPPED')) {
                    const pc = uc.reg_read_i32(Module.ARM_REG_PC);
                    console.warn('Unmapped access at PC=0x' + pc.toString(16));
                    uc.reg_write_i32(Module.ARM_REG_PC, (pc + 2) | 1);
                } else {
                    throw e;
                }
            }
            processDma();
            processInterrupts();
            const stopped = stopRequested || is_watchdog_reset_requested();
            return { pc: uc.reg_read_i32(Module.ARM_REG_PC), stopped, instCount };
        },

        run(max_instructions = 0) {
            stopRequested = false;
            let total = 0;
            while (!stopRequested) {
                const result = this.step(maxBatchInst);
                total++;
                if (result.stopped) break;
                if (max_instructions > 0 && total * maxBatchInst >= max_instructions) break;
            }
            return { totalSteps: total, instCount };
        },

        stop() {
            stopRequested = true;
            try { uc.emu_stop(); } catch (e) { /* ignore */ }
        },

        getRegisters() {
            return {
                R0: uc.reg_read_i32(Module.ARM_REG_R0),
                R1: uc.reg_read_i32(Module.ARM_REG_R1),
                R2: uc.reg_read_i32(Module.ARM_REG_R2),
                R3: uc.reg_read_i32(Module.ARM_REG_R3),
                R4: uc.reg_read_i32(Module.ARM_REG_R4),
                R5: uc.reg_read_i32(Module.ARM_REG_R5),
                R6: uc.reg_read_i32(Module.ARM_REG_R6),
                R7: uc.reg_read_i32(Module.ARM_REG_R7),
                R8: uc.reg_read_i32(Module.ARM_REG_R8),
                R9: uc.reg_read_i32(Module.ARM_REG_R9),
                R10: uc.reg_read_i32(Module.ARM_REG_R10),
                R11: uc.reg_read_i32(Module.ARM_REG_R11),
                R12: uc.reg_read_i32(Module.ARM_REG_R12),
                SP: uc.reg_read_i32(Module.ARM_REG_SP),
                LR: uc.reg_read_i32(Module.ARM_REG_LR),
                PC: uc.reg_read_i32(Module.ARM_REG_PC),
                xPSR: uc.reg_read_i32(Module.ARM_REG_XPSR),
            };
        },

        gpioSetInput(port, pin, value) { gpio_set_input(port, pin, value); },
        gpioReadInput(port, pin) { return gpio_read_input(port, pin); },
        gpioReadOutput(port, pin) { return gpio_read_output(port, pin); },

        close() { uc.close(); },
    };
}
