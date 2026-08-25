# Performance Experiments — Path A / Browser Demo

Notes from timing and rendering work on the browser demo (`site/index.html`).

## Emulation throughput
- Dual module (emulator.js + unicorn_arm.cjs + stm32_bluepill_wasm.js): ~22 MIPS headless,
  ~24 MIPS in the browser periph37 run.
- The merged module (when it could be built) was ~24.1 MIPS at -O2 (vs ~3.4 MIPS Debug -O0),
  confirming TCI is the bottleneck and the Rust peripheral layer is not.
- HOOKLESS instruction counting: `emu_start(begin,0,0,maxBatch)` stops exactly at maxBatch,
  crediting each batch in full. Removed the per-instruction JS codeHook (~18-20% of runtime),
  lifting ~18.5 → ~22 MIPS.
- `step_batch()` ticks peripherals once per batch (instruction-delta based), ~3.15× faster than
  per-instruction `step()`; closed-form TIM advance removed the last O(ticks) loop
  (~124× on `step_batch`).

## Batch size
- DEFAULT_MAX_BATCH = 20000 (was 100K). Smaller batches cut IRQ/EXTI/UART delivery latency
  (~5.4ms → ~1.1ms) at zero measurable cost.

## Browser render throttling (site/index.html)
The expensive SVG board re-render was throttled independently of the emulation step:

- **Dirty-skip in `renderBoard`**: rebuild the board SVG only when something visible changed.
  Compute a signature `ledOn | pinActivityTotal | flashingPins` and skip `boardWrap.innerHTML`
  when unchanged. Most frames the board is static, so this avoids a full innerHTML parse
  (~60fps step vs ~12fps render).

  ```js
  let lastBoardSig = '';
  function renderBoard(ledOn) {
    const flashing = [];
    for (const k in PIN_ACTIVITY) {
      const a = PIN_ACTIVITY[k];
      if (performance.now() - a.lastToggle < 2000) flashing.push(k);
    }
    const sig = `${ledOn ? 1 : 0}|${pinActivityTotal}|${flashing.sort().join(',')}`;
    if (sig === lastBoardSig) return;
    lastBoardSig = sig;
    boardWrap.innerHTML = boardSvg(ledOn);
  }
  ```

- **`UI_THROTTLE = 5`**: the emulation steps every rAF frame (~60fps) for full throughput,
  but the visual layer renders only every `UI_THROTTLE` frames (~12fps). Stepping is never
  starved by rendering, keeping browser MIPS high without a worker.

  ```js
  const UI_THROTTLE = 5;
  // in the rAF loop: always step; render only when (uiFrame++ % UI_THROTTLE) === 0
  ```

- **Pin-activity monitor**: pins glow amber for ~2s after the chip drives them to a new level
  (`PIN_ACTIVITY` map + `onPinChange` subscriber), with cumulative per-pin toggle counts and a
  running total caption. Fed by the `onPinChange` API, drained per batch.

## Experiment artifacts (removed)
`site/emu.worker.js` and `site/perf_webworker.html` explored moving the step loop into a Web
Worker; deferred (the UI_THROTTLE + dirty-skip approach achieved the same MIPS without the
worker's message-passing complexity).
