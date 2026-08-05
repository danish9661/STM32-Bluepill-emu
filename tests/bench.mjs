import * as periph from '../pkg/stm32_bluepill_wasm.js';

const { init, periph_read, periph_write, tick, step, has_pending_interrupt,
        get_next_pending_interrupt, clear_current_interrupt, gpio_read_output,
        gpio_set_input, get_uart_output, uart_rx_byte, adc_set_sim_value,
        is_watchdog_reset_requested, can_inject_message } = periph;

const RESULTS = [];

function bench(name, fn, iterations = 10) {
  const times = [];
  for (let i = 0; i < iterations; i++) {
    const t0 = process.hrtime.bigint();
    fn();
    const dt = Number(process.hrtime.bigint() - t0) / 1e6;
    if (i >= 2) times.push(dt);
  }
  const avg = times.reduce((a, b) => a + b, 0) / times.length;
  const min = Math.min(...times);
  const max = Math.max(...times);
  RESULTS.push({ name, avg_ms: avg, min_ms: min, max_ms: max });
}

console.log('=== Peripheral Performance Benchmark ===\n');

init();
periph_write(0x40021018, 4, 0xFFFF);
periph_write(0x4002101C, 4, 0xFFFF);
periph_write(0x40021014, 4, 0xFFFF);
for (let port = 0x40010800; port <= 0x40011400; port += 0x400) {
  periph_write(port + 0x00, 4, 0x33333333);
  periph_write(port + 0x04, 4, 0x33333333);
}
periph_write(0x40013808, 4, 0x341);
periph_write(0x4001380C, 4, (1<<13)|(1<<3)|(1<<2));
periph_write(0x40000000, 4, 1);
periph_write(0x4000002C, 4, 999);
periph_write(0x40013000, 4, 0x0341);
periph_write(0x40005404, 4, 0x2020);
periph_write(0x40005408, 4, 0x002D);
periph_write(0x4000541C, 4, 0x0001);
periph_write(0x40005400, 4, 0x0001);
periph_write(0x40006400, 4, 0x00010001);
periph_write(0x40002804, 4, 1);
periph_write(0x4000280C, 4, 32767);
periph_write(0x40012404, 4, 1);
periph_write(0x40012408, 4, 1);
periph_write(0x40007400, 4, 1);
periph_write(0x40010000, 4, 0);
periph_write(0x40010400, 4, 1);
periph_write(0x40006C00, 4, 0xA5A5);
periph_write(0xE000E100, 4, 0xFFFFFFFF);
periph_write(0xE000E104, 4, 0xFFFFFFFF);
periph_write(0xE000E108, 4, 0xFFFFFFFF);

console.log('Setup complete. Running benchmarks...\n');

// ============================================================
// tick() baselines
// ============================================================
bench('tick() x 1,000,000 (bare)', () => { for (let i = 0; i < 1_000_000; i++) tick(); }, 5);

bench('tick+GPIO+UART x 100,000', () => {
  for (let i = 0; i < 100_000; i++) {
    tick(); periph_write(0x40011010, 4, 1 << 13); periph_read(0x4001100C, 4);
    periph_write(0x40013804, 4, 0x41); periph_read(0x40013800, 4);
  }
}, 5);

bench('tick+NVIC+IRQ x 100,000', () => {
  for (let i = 0; i < 100_000; i++) {
    tick(); periph_write(0xE000E200, 4, 1 << 3); periph_read(0xE000E200, 4);
    if (has_pending_interrupt()) { const irq = get_next_pending_interrupt(); clear_current_interrupt(); }
    periph_write(0xE000E280, 4, 1 << 3);
  }
}, 5);

bench('tick+mixed x 50,000 (GPIO+UART+TIM+CRC+SPI+NVIC)', () => {
  for (let i = 0; i < 50_000; i++) {
    tick();
    if (has_pending_interrupt()) { get_next_pending_interrupt(); clear_current_interrupt(); }
    periph_write(0x40011010, 4, 1 << 13); periph_write(0x40011010, 4, 1 << 29);
    periph_read(0x40013800, 4); periph_read(0x40000024, 4);
    periph_write(0x40023000, 4, i * 0xDEAD); periph_read(0x40013008, 4);
    periph_write(0x40005400, 4, 0x0001); periph_read(0xE000E010, 4);
  }
}, 5);

// Realistic codeHook: tick + watchdog + dma_pending + has_pending (5 WASM calls/inst)
bench('codeHook (5 calls) x 100,000', () => {
  for (let i = 0; i < 100_000; i++) {
    tick();
    is_watchdog_reset_requested();
    has_pending_interrupt();
  }
}, 5);

// ============================================================
// step() optimized (single WASM call per instruction)
// ============================================================
console.log('--- step() optimized ---\n');

bench('step() x 1,000,000 (bare)', () => { for (let i = 0; i < 1_000_000; i++) step(0, 0); }, 5);

bench('step+GPIO+UART x 100,000', () => {
  for (let i = 0; i < 100_000; i++) {
    step(0, 0); periph_write(0x40011010, 4, 1 << 13); periph_read(0x4001100C, 4);
    periph_write(0x40013804, 4, 0x41); periph_read(0x40013800, 4);
  }
}, 5);

bench('step+NVIC+IRQ x 100,000', () => {
  for (let i = 0; i < 100_000; i++) {
    step(0, 0); periph_write(0xE000E200, 4, 1 << 3); periph_read(0xE000E200, 4);
    if (has_pending_interrupt()) { get_next_pending_interrupt(); clear_current_interrupt(); }
    periph_write(0xE000E280, 4, 1 << 3);
  }
}, 5);

bench('step+mixed x 50,000 (GPIO+UART+TIM+CRC+SPI+NVIC)', () => {
  for (let i = 0; i < 50_000; i++) {
    step(0, 0);
    if (has_pending_interrupt()) { get_next_pending_interrupt(); clear_current_interrupt(); }
    periph_write(0x40011010, 4, 1 << 13); periph_write(0x40011010, 4, 1 << 29);
    periph_read(0x40013800, 4); periph_read(0x40000024, 4);
    periph_write(0x40023000, 4, i * 0xDEAD); periph_read(0x40013008, 4);
    periph_write(0x40005400, 4, 0x0001); periph_read(0xE000E010, 4);
  }
}, 5);

// ============================================================
// Report
// ============================================================
console.log('\n=== Results ===');
console.log('Benchmark                          | Method | Avg (ms)  |  IPS     | Gain vs tick');
console.log('-'.repeat(90));
const pairs = [
  ['bare x 1M', 'tick() x 1,000,000 (bare)', 'step() x 1,000,000 (bare)'],
  ['codeHook x 100K', 'codeHook (5 calls) x 100,000', 'step() x 1,000,000 (bare)'],
  ['GPIO+UART x 100K', 'tick+GPIO+UART x 100,000', 'step+GPIO+UART x 100,000'],
  ['NVIC+IRQ x 100K', 'tick+NVIC+IRQ x 100,000', 'step+NVIC+IRQ x 100,000'],
  ['mixed x 50K', 'tick+mixed x 50,000 (GPIO+UART+TIM+CRC+SPI+NVIC)', 'step+mixed x 50,000 (GPIO+UART+TIM+CRC+SPI+NVIC)'],
];
for (const [label, tickName, stepName] of pairs) {
  const t = RESULTS.find(r => r.name === tickName);
  const s = RESULTS.find(r => r.name === stepName);
  if (!t || !s) continue;
  const ops = { 'bare x 1M': 1_000_000, 'codeHook x 100K': 100_000, 'GPIO+UART x 100K': 100_000, 'NVIC+IRQ x 100K': 100_000, 'mixed x 50K': 50_000 }[label];
  const tickIps = ops / (t.avg_ms / 1000);
  const stepIps = ops / (s.avg_ms / 1000);
  const gain = ((stepIps / tickIps - 1) * 100).toFixed(1);
  console.log(`${label.padEnd(30)} | tick  | ${t.avg_ms.toFixed(3).padStart(8)} | ${tickIps.toFixed(0).padStart(8)} |`);
  console.log(`${''.padEnd(30)} | step  | ${s.avg_ms.toFixed(3).padStart(8)} | ${stepIps.toFixed(0).padStart(8)} | +${gain}%`);
  console.log('-'.repeat(90));
}

const stepBare = RESULTS.find(r => r.name === 'step() x 1,000,000 (bare)');
const tickBare = RESULTS.find(r => r.name === 'tick() x 1,000,000 (bare)');
if (stepBare && tickBare) {
  const overhead = ((tickBare.avg_ms - stepBare.avg_ms) / 1_000_000 * 1000).toFixed(3);
  console.log(`\nPer-instruction overhead saved: ~${overhead} µs (${((tickBare.avg_ms - stepBare.avg_ms) / tickBare.avg_ms * 100).toFixed(1)}%)`);
}
console.log('');
