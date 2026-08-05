const MUnicorn = require('../pkg/unicorn_arm.cjs');
const periph = require('../pkg/stm32_periph_wasm.js');
const fs = require('fs');

async function main() {
    const firmwarePath = process.argv[2] || 'C:\\Users\\Danish\\Documents\\stm32-emu\\arduino_test\\arduino_test.ino.bin';
    const firmware = fs.readFileSync(firmwarePath);
    const svdXml = fs.readFileSync('C:\\Users\\Danish\\Documents\\stm32-emu\\monox\\stm32f407.svd', 'utf8');

    console.log('Initializing Unicorn...');
    const Module = await MUnicorn({});
    const periphWasmBuf = fs.readFileSync('./stm32_periph_wasm_bg.wasm');
    await periph.default({ module_or_path: periphWasmBuf.buffer });
    periph.init_svd(svdXml);

    const uc = new Module.Unicorn(Module.ARCH_ARM, Module.MODE_MCLASS | Module.MODE_LITTLE_ENDIAN);

    console.log('Module.PERM_ALL:', Module.PERM_ALL);
    console.log('Module.PROT_READ:', Module.PROT_READ, 'PROT_WRITE:', Module.PROT_WRITE, 'PROT_EXEC:', Module.PROT_EXEC);

    uc.mem_map(0x08000000, 0x100000, Module.PERM_ALL);
    uc.mem_write(0x08000000, firmware);

    uc.mem_map(0x20000000, 0x20000, Module.PERM_ALL);

    const read32 = (addr) => {
        const b = uc.mem_read(BigInt(addr), 4);
        const dt = new DataView(b.buffer, b.byteOffset, b.byteLength);
        return dt.getUint32(0, true);
    };

    const sp_init = read32(0x08000000);
    const pc_init = read32(0x08000004);
    console.log(`SP=0x${sp_init.toString(16)} PC=0x${pc_init.toString(16)}`);

    uc.reg_write_i32(Module.ARM_REG_SP, sp_init);
    uc.reg_write_i32(Module.ARM_REG_PC, pc_init | 1);

    // Check memory regions
    console.log('Memory regions:', uc.mem_regions().length, 'regions');
    for (const r of uc.mem_regions()) {
        console.log(`  0x${r.begin.toString(16)} - 0x${r.end.toString(16)} perms=${r.perms}`);
    }

    console.log('Trying emu_start for 1 instruction...');
    try {
        uc.emu_start(BigInt(pc_init | 1), 0n, 0n, 1);
        console.log('Success! PC after:', uc.reg_read_i32(Module.ARM_REG_PC).toString(16));
    } catch (e) {
        console.error('Error:', e);
        console.log('PC after error:', uc.reg_read_i32(Module.ARM_REG_PC).toString(16));
        console.log('SP after error:', uc.reg_read_i32(Module.ARM_REG_SP).toString(16));
    }

    uc.close();
    console.log('Done');
}

main().catch(e => console.error('Fatal:', e));
