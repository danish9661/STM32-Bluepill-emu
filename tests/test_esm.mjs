// ESM smoke: the Rust peripheral wasm module must load through its wasm-pack
// ESM glue and Unicorn must boot through the native addon (same shape the
// browser page uses, minus the fetch path).
import { createRequire } from 'module';
const require = createRequire(import.meta.url);

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

async function main() {
    const periph = await import('../pkg/stm32_bluepill_wasm.js');
    ok(!!periph && typeof periph.initSync === 'function', `periph ESM glue loads (initSync present, keys: ${Object.keys(periph).length})`);

    const MUnicorn = require('../pkg/unicorn_arm.cjs');
    ok(typeof MUnicorn === 'function', `unicorn_arm.cjs loads (typeof ${typeof MUnicorn})`);
    const Module = await MUnicorn({});
    ok(typeof Module.Unicorn === 'function', 'MUnicorn() resolves a Module');
    const uc = new Module.Unicorn(Module.ARCH_ARM, Module.MODE_MCLASS | Module.MODE_LITTLE_ENDIAN);
    ok(!!uc && typeof uc.emu_start === 'function', 'Unicorn instance created (cortex-m, little-endian)');
    uc.close();

    console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
    process.exit(failed === 0 ? 0 : 1);
}

main().catch((e) => {
    console.error('ERROR:', e.name, e.message);
    console.error('Stack:', e.stack?.substring(0, 1000));
    process.exit(1);
});
