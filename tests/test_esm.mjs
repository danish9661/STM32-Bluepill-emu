// ESM smoke: the Rust peripheral+CPU wasm module must load through its
// wasm-pack ESM glue and expose the native backend API.
import { createRequire } from 'module';
const require = createRequire(import.meta.url);

let passed = 0, failed = 0;
const ok = (cond, name) => { if (cond) { passed++; } else { failed++; console.log(`FAIL: ${name}`); } };

async function main() {
    const periph = await import('../pkg/stm32_bluepill_wasm.js');
    ok(!!periph && typeof periph.initSync === 'function', `periph ESM glue loads (initSync present, keys: ${Object.keys(periph).length})`);

    const { readFileSync } = await import('fs');
    periph.initSync({ module: readFileSync(new URL('../pkg/stm32_bluepill_wasm_bg.wasm', import.meta.url)) });
    periph.init();
    periph.rustcpu_init(0x20005000, 0x08000000, 0x10000, 0x5000);
    const regs = periph.rustcpu_regs();
    ok(regs.length === 20, `rustcpu regs vector has 20 words (${regs.length})`);
    ok((regs[13] >>> 0) === 0x20005000, `initial SP visible (${(regs[13] >>> 0).toString(16)})`);
    ok(periph.rustcpu_fault().length === 0, 'no fault on fresh backend');

    console.log(`\nResults: ${passed} passed, ${failed} failed, ${passed + failed} total`);
    process.exit(failed === 0 ? 0 : 1);
}

main().catch((e) => {
    console.error('ERROR:', e.name, e.message);
    console.error('Stack:', e.stack?.substring(0, 1000));
    process.exit(1);
});
