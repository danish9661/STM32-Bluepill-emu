# Changelog

All notable changes to this project will be documented in this file.

## [1.2.0] — 2026-09-03

### Added
- SDIO host peripheral (`src/peripherals/sdio.rs`): SDHC card model at 0x40018000, IRQ 49 — CMD0/2/3/6/7/8/9/12/13/16/17/18/24/25/55 + ACMD41 (busy-first init), 32-word FIFO, DATAEND/DBCKEND/CMDREND/CMDSENT/CTIMEOUT with MASK-gated IRQ, DCOUNT/FIFOCNT, DMA2 CH4 requests
- SD card ext device (`src/ext_devices/sd_card.rs`, `add_sd_card('SDIO', data)`): CID/CSD/OCR/RCA derived, CSD capacity from image size; `ext_devices.sd_card` in library + `sd_card:` (file/size) in CLI config

### Fixed
- DMA2 completion routing: completion streams/IRQ tables were sized 8 with local indices, so DMA1 CH4 and DMA2 CH4 both claimed stream 3 and DMA1's tick drained DMA2's bits (DMA2 ISR/CNDTR never completed). Streams are now global (DMA1 0-6, DMA2 7-11) with masked per-DMA takes; JS pump untouched

### Tests
- 41 new SDIO unit tests (277/277 total): init sequence, block R/W + read-back, IRQ49, DMA2 pump absorb of real image bytes + TCIF4/CNDTR clear, no-card timeouts, F103-SVD registration

## [1.1.0] — 2026-09-03

### Performance
- Poll-aware batch shrinking — 8+ consecutive reads of one peripheral address shrink the batch to 5K (batch-boundary flags land sooner), with backoff for external waits; ~4% on periph39 (`pkg/emulator.js`, `pkg/cli.mjs`, `POLL_SHRINK=0` disables)
- CAN-autopilot flag resolved from ELF symbols instead of a hardcoded address that went stale (`canRxArmed` moved; the stale address cost a 3M-iteration spin storm per run): emulator.js 200M 12.45s → 8.18s, browser 9.2 → 21.8 MIPS

### Added
- `dmaPending()` emulator accessor (mirrors the CLI `dmaBusy` UART gate)
- Browser speed benchmark (`tests/test_browser_speed.mjs`, wired into Playwright)
- Worker sends a GPIO snapshot per frame so the page grid renders on the worker path (previously dark); chip name in worker-ready log

### Fixed
- Browser CI suite: GPIO-grid test polls instead of fixed sleep; F105 chip test expects the worker log line
- `test_emulator_js.mjs` autopilot granularity 10M → 1M chunks

## [0.1.0] — 2026-08-22

### Added
- CLI `--help` flag with full usage documentation
- CLI `--verbose` mode for peripheral read/write tracing (debugging)
- Improved error messages for firmware load failures, config parse errors, and invalid vector tables
- CHANGELOG.md

### Fixed
- Better error diagnostics when firmware is missing, empty, or has wrong format
- Clearer messages for invalid SP/PC in vector table (wrong memory layout, non-Thumb ELF)

## [0.1.0-beta] — 2026-08-20

### Performance
- Bus::get temporal-locality cache — 99% hit rate on sequential peripheral access
- NVIC pending scan: iterate only set bits via `trailing_zeros` instead of scanning all 111 IRQs
- Trigger lookups (`exti_port_for_line`, `afio_remap_status`, etc.) converted from linear scan to cached `bus.get()`

### Added
- Pin-activity monitor on the Blue Pill board SVG — glows pins amber on level changes
- WS2812 LED strip demo (SPI1 DMA, 800kHz strip decode live in browser)
- 236 unit tests (from 224)
- 39/39 firmware checks (SVC + PendSV delivery)

### Fixed
- DMA ISR flag layout (off-by-one channel mapping)
- DMA direction inverted vs CMSIS (DIR bits mapped backwards)
- DMA pushes now fire JS write watchers (page decoders see DMA traffic)

## [0.1.0-alpha.3] — 2026-08-14

### Performance
- Closed-form timer advance — step_batch 1409ms → 11ms (124×)
- Batch register transport in IRQ dispatch — 17 crossings → 2 per IRQ
- Hookless instruction counting — 20% runtime reduction
- instCount as plain number (not BigInt) — 19% faster
- Batch size reduced 100K → 20K for lower interrupt latency

### Added
- Interrupt dispatch policy in Rust (src/interrupts.rs) — 64-IRQ budget, SVC frame mirror
- xPSR restore from stacked frame (required for cmp/beq across batch boundaries)
- Rust-side DMA pump with plan-based execution
- `finish_interrupt()` — single Rust crossing for ISR return

### Fixed
- xPSR restore dropped during frame-restore unification
- SysTick debt drain — multi-period elapsed delivered only one tick

## [0.1.0-alpha.2] — 2026-08-11

### Added
- Real GPIO electrical model (pull-up/down, floating, open-drain, slew)
- Real ADC conversion state machine (timing, channels, DMA, AWD)
- DAC→ADC loopback + ADC external triggers
- Full FSMC (7 NOR banks, NAND, PC-Card)
- Sleep state timing (SLEEPDEEP → frozen peripherals)
- Exceptions: SVC, PendSV, faults (BusFault, UsageFault, HardFault)
- reset_ext_devices() — prevents stale devices across emulator instances
- Multi-chip support: STM32F105xx SVD

### Fixed
- CAN MCR write mask (ABOM bit was silently dropped)
- DMA1 address corrected (0x40006000 → 0x40020000)
- Hardcoded peripheral table aligned with real STM32F103 addresses

## [0.1.0-alpha.1] — 2026-08-06

### Added
- Initial release: STM32F103C8 (Blue Pill) full-system emulation
- Unicorn ARM Cortex-M3 CPU (TCG JIT)
- Rust peripherals (GPIO, USART, TIM, SPI, I2C, DMA, RTC, CRC, CAN, NVIC, EXTI, ADC, DAC, etc.)
- WASM output for browser and Node.js
- Arduino firmware compatibility (24-peripheral test sketch, 37 checks)
- Intel HEX, ELF, and raw binary firmware loading
- SVD-based peripheral register map generation
- CLI with config YAML support
- Browser demo with interactive board SVG
