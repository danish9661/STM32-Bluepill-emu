import { readFileSync } from 'fs';
import { createRequire } from 'module';
const require = createRequire(import.meta.url);
const periph = require('./stm32_periph_wasm.js');
const {
    periph_read, periph_write, tick, init_svd, init,
    add_spi_flash, add_i2c_eeprom,
    has_pending_interrupt, get_next_pending_interrupt, get_uart_output,
} = periph;

const rl = require('readline').createInterface({ input: process.stdin, terminal: false });

function send(obj) {
    process.stdout.write(JSON.stringify(obj) + '\n');
}

send({ type: 'ready' });

rl.on('line', (line) => {
    if (!line.trim()) return;
    const msg = JSON.parse(line);

    try {
        switch (msg.cmd) {
            case 'init_svd':
                init_svd(msg.data);
                send({ type: 'ok' });
                break;

            case 'add_spi_flash': {
                const data = msg.data_b64 ? Buffer.from(msg.data_b64, 'base64') : null;
                add_spi_flash(msg.peripheral, msg.jedec_id, data, null);
                send({ type: 'ok' });
                break;
            }

            case 'add_i2c_eeprom': {
                const data = msg.data_b64 ? Buffer.from(msg.data_b64, 'base64') : null;
                add_i2c_eeprom(msg.peripheral, msg.addr, data);
                send({ type: 'ok' });
                break;
            }

            case 'init':
                init();
                send({ type: 'ok' });
                break;

            case 'read': {
                const val = periph_read(msg.addr, msg.size) >>> 0;
                send({ type: 'value', value: val });
                break;
            }

            case 'write':
                periph_write(msg.addr, msg.size, msg.value);
                send({ type: 'ok' });
                break;

            case 'tick':
                tick();
                send({ type: 'ok', has_pending_interrupt: has_pending_interrupt() });
                break;

            case 'has_pending_interrupt':
                send({ type: 'value', value: has_pending_interrupt() });
                break;

            case 'get_next_pending_interrupt':
                send({ type: 'value', value: get_next_pending_interrupt() });
                break;

            case 'get_uart_output': {
                const out = get_uart_output();
                send({ type: 'data', data: out || '' });
                break;
            }

            case 'quit':
                process.exit(0);
                break;

            default:
                send({ type: 'error', msg: `Unknown cmd: ${msg.cmd}` });
        }
    } catch (e) {
        send({ type: 'error', msg: e.message || String(e) });
    }
});
