import { createRequire } from 'module';
const require = createRequire(import.meta.url);

async function main() {
    try {
        const periph = require('../pkg/stm32_periph_wasm.js');
        console.log('periph loaded, keys:', Object.keys(periph).length);
        
        const MUnicorn = require('../pkg/unicorn_arm.cjs');
        console.log('typeof MUnicorn:', typeof MUnicorn);
        const Module = await MUnicorn({});
        console.log('SUCCESS! Unicorn:', typeof Module.Unicorn);
        const uc = new Module.Unicorn(Module.ARCH_ARM, Module.MODE_MCLASS | Module.MODE_LITTLE_ENDIAN);
        console.log('Unicorn instance created');
        uc.close();
    } catch (e) {
        console.error('ERROR:', e.name, e.message);
        console.error('Stack:', e.stack?.substring(0, 1000));
    }
}
main();
