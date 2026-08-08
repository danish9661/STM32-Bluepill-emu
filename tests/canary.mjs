#!/usr/bin/env node
/*
 * Quick regression canary for the Arduino firmware test.
 * Runs the full 37-check firmware suite at a reduced instruction budget
 * (~25s vs ~50s for the 200M run) and asserts a clean 37/37 SUMMARY.
 *
 * Usage:
 *   node tests/canary.mjs                 # default --max=100000000
 *   node tests/canary.mjs 60000000        # lower budget (partial async coverage)
 *   echo -n "AB" | node tests/canary.mjs  # works with piped stdin too
 */
import { spawn } from 'child_process';
import { fileURLToPath } from 'url';
import path from 'path';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const maxInst = process.argv[2] || '100000000';
const input = 'AB'; // 'A' -> DMA RX, 'B' -> UART RX

const child = spawn(process.execPath, [
    path.join(root, 'pkg', 'cli.mjs'),
    `--config=${path.join(root, 'tests', 'arduino_periph_test', 'config.yaml')}`,
    `--max=${maxInst}`,
], { cwd: root });

let stdout = '';
let stderr = '';

child.stdin.write(input);
child.stdin.end();

child.stdout.on('data', (d) => { stdout += d; });
child.stderr.on('data', (d) => { stderr += d; });

child.on('close', (code) => {
    const passCount = (stdout.match(/\[[^\]]+\] PASS/g) || []).length;
    const failCount = (stdout.match(/\[[^\]]+\] FAIL/g) || []).length;
    const summary = stdout.match(/SUMMARY pass=(\d+) fail=(\d+)/);
    const fatal = stderr.split('\n').filter(l => l && !l.startsWith('[INFO')).slice(-6).join('\n');

    let ok = code === 0 && failCount === 0 && summary && summary[1] === '37' && summary[2] === '0';

    if (ok) {
        console.log(`CANARY PASS: ${passCount} checks green, SUMMARY pass=${summary[1]} fail=${summary[2]} (${maxInst} instr)`);
        process.exit(0);
    }
    console.log(`CANARY FAIL: exit=${code} passes=${passCount} fails=${failCount} summary=${summary ? summary[0] : 'absent'}`);
    if (fatal) console.log(`stderr tail:\n${fatal}`);
    const fails = stdout.split('\n').filter(l => l.includes('FAIL'));
    if (fails.length) console.log('fails:\n' + fails.join('\n'));
    process.exit(1);
});
