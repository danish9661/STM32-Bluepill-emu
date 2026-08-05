async function main() {
    const MUnicorn = require('../pkg/unicorn_arm.cjs');
    try {
        console.log('typeof MUnicorn:', typeof MUnicorn);
        const Module = await MUnicorn({});
        console.log('SUCCESS! Module.Unicorn:', typeof Module.Unicorn);
        console.log('Module.ARCH_ARM:', Module.ARCH_ARM);

        const uc = new Module.Unicorn(Module.ARCH_ARM, Module.MODE_MCLASS | Module.MODE_LITTLE_ENDIAN);
        console.log('Unicorn instance created');
        uc.close();
        console.log('Done');
    } catch (e) {
        console.error('ERROR:', e.name, e.message);
        console.error('Stack:', e.stack);
    }
}
main();
