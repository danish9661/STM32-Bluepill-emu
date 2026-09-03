// Browser speed benchmark: runs a fixed instruction window in real Chromium
// via the shipped WASM stack (emulator.js + stm32_bluepill_wasm_bg.wasm +
// unicorn_arm.js) and reports MIPS for hardcoded vs SVD bus, same firmware +
// devices + gated UART/CAN feed as the Node runs.
import { test, expect } from '@playwright/test';

const SPEED_JS = async (useSvd) => {
  const { createEmulator, parseElf } = await import('./emulator.js');
  const load = async (name) => {
    const r = await fetch(name);
    return new Uint8Array(await r.arrayBuffer());
  };
  const [fw, e1, e2, sf1, sf2, svd] = await Promise.all([
    load('arduino_periph_test.elf'),
    load('arduino_eeprom.bin'),
    load('arduino_eeprom2.bin'),
    load('arduino_spi_flash.bin'),
    load('arduino_spi_flash2.bin'),
    useSvd ? load('STM32F103.svd').then(async (r) => new TextDecoder().decode(r)) : null,
  ]);
  const emu = await createEmulator({
    firmware: fw,
    ...(useSvd ? { svd } : {}),
    ext_devices: {
      i2c_eeprom: [
        { peripheral: 'I2C1', address: 0x50, data: e1 },
        { peripheral: 'I2C2', address: 0x51, data: e2 },
      ],
      i2c_oled: [{ peripheral: 'I2C1', address: 0x3C, width: 128, height: 64 }],
      spi_flash: [
        { peripheral: 'SPI1', jedec_id: 0xEF4016, data: sf1, cs: 'PA4' },
        { peripheral: 'SPI2', jedec_id: 0xEF4017, data: sf2, cs: 'PB12' },
      ],
      lcd: [{ peripheral: 'SPI1', cs: 'PA1' }],
      touchscreen: [{ peripheral: 'SPI1', cs: 'PA2', touch_detected_pin: 'PA3' }],
    },
  });
  // Gated UART feed (A reserved for DMA RX, B for UART RX) + CAN autopilot,
  // mirroring pkg/cli.mjs so the firmware takes the same path as Node runs.
  // canRxArmed is resolved from the ELF symbols — never hardcode it (a stale
  // address silently costs a 3M-iteration RF0R spin storm per run).
  const canSym = parseElf(fw).symbols.find((s) => s.name === 'canRxArmed');
  if (!canSym) throw new Error('canRxArmed symbol missing from ELF');
  const CAN_RAM_FLAG = canSym.addr;
  const queue = [0x41, 0x42];
  let canInjected = false;
  const N = 20_000_000;
  let done = 0;
  const t0 = performance.now();
  while (done < N) {
    while (queue.length && emu.rxPending() === 0) emu.uartRx(queue.shift());
    const r = emu.run(1_000_000);
    done += 1_000_000;
    if (!canInjected && emu.memRead32(CAN_RAM_FLAG) !== 0) {
      canInjected = emu.canInjectMessage(0x40006400, 0 << 21, 2, 0xdead, 0);
    }
    if (r.stopped) break;
  }
  const elapsed = (performance.now() - t0) / 1000;
  const out = String(emu.getUartOutput() || '');
  emu.close();
  return {
    done, elapsed: +elapsed.toFixed(2), mips: +(done / elapsed / 1e6).toFixed(2),
    canInjected, uartBytes: out.length,
    fails: out.split('\n').filter((s) => s.includes('FAIL')),
    sab: (typeof crossOriginIsolated !== 'undefined' && crossOriginIsolated) ? 'ON' : 'OFF',
  };
};

test.describe('Browser speed (WASM)', () => {
  test('hardcoded bus: 20M window MIPS', async ({ page }) => {
    page.on('console', (msg) => {
      if (msg.type() === 'error') console.log('BROWSER ERR:', msg.text());
    });
    page.on('pageerror', (err) => console.log('PAGE ERROR:', String(err)));
    await page.goto('http://localhost:8765/');
    const r = await page.evaluate(SPEED_JS, false);
    console.log(`browser hardcoded: ${r.done} instr in ${r.elapsed}s = ${r.mips} MIPS (SAB ${r.sab}, CAN=${r.canInjected}, uart=${r.uartBytes}B)`);
    expect(r.done).toBe(20_000_000);
    expect(r.mips).toBeGreaterThan(1);
    expect(r.uartBytes).toBeGreaterThan(0);
    expect(r.fails).toHaveLength(0);
  });

  test('SVD bus: 20M window MIPS', async ({ page }) => {
    page.on('console', (msg) => {
      if (msg.type() === 'error') console.log('BROWSER ERR:', msg.text());
    });
    page.on('pageerror', (err) => console.log('PAGE ERROR:', String(err)));
    await page.goto('http://localhost:8765/');
    const r = await page.evaluate(SPEED_JS, true);
    console.log(`browser svd: ${r.done} instr in ${r.elapsed}s = ${r.mips} MIPS (SAB ${r.sab}, CAN=${r.canInjected}, uart=${r.uartBytes}B)`);
    expect(r.done).toBe(20_000_000);
    expect(r.mips).toBeGreaterThan(1);
    expect(r.uartBytes).toBeGreaterThan(0);
    expect(r.fails).toHaveLength(0);
  });
});
