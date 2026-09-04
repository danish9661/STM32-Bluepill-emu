// TypeScript declarations for pkg/emulator.js
// The core STM32F1 WASM emulator API.

/** Parse Intel HEX text into bytes + lowest load address. */
export function parseIntelHex(text: string): { data: Uint8Array; base: number };

/** Parse GNU ld `.map` output into symbol entries. */
export function parseSymbolMap(text: string): Array<{ name: string; addr: number }>;

/** Parse ELF32 (ARM little-endian) into loadable regions + symbols. */
export function parseElf(buffer: ArrayBuffer | Uint8Array): {
  regions: Array<{ start: number; data: Uint8Array }>;
  symbols: Array<{ name: string; addr: number }>;
};

// ── createEmulator options ──────────────────────────────────────────────────

export interface ExtDevices {
  spi_flash?: Array<{
    peripheral: string | number;
    jedec_id: number | string;
    data?: Uint8Array;
    cs?: string | null;
  }>;
  i2c_eeprom?: Array<{
    peripheral: string | number;
    address: number | string;
    data?: Uint8Array;
  }>;
  i2c_oled?: Array<{
    peripheral: string | number;
    address?: number | string;
    width?: number | string;
    height?: number | string;
  }>;
  lcd?: Array<{
    peripheral: string | number;
    cs?: string | null;
  }>;
  touchscreen?: Array<{
    peripheral: string | number;
    touch_detected_pin?: string | null;
    cs?: string | null;
  }>;
  software_spi?: Array<{
    name: string;
    cs?: string | null;
    clk: string;
    miso: string;
    mosi: string;
  }>;
  fsmc_bank?: Array<{
    name: string;
    data: Uint8Array;
  }>;
}

export interface JsPeripheral {
  base: number;
  size: number;
  read: (addr: number, size: number) => number;
  write: (addr: number, value: number, size: number) => void;
}

export interface CreateEmulatorOptions {
  /** Firmware to load. Raw binary, ELF (auto-detected), or Intel HEX string. */
  firmware?: Uint8Array | ArrayBuffer | string;
  /** Flash region size in bytes (default 0x10000 = 64KB). */
  flash_size?: number;
  /** SRAM size in bytes (default 0x5000 = 20KB). */
  ram_size?: number;
  /** Vector table base address (default 0x08000000). */
  vector_table?: number;
  /** SVD XML string (optional; overrides the builtin F103C8 map). */
  svd?: string | null;
  /** Chip identifier or `{name, svd}` object (default 'stm32f103c8'). */
  chip?: string | { name: string; svd: string };
  /** rp2040js-style custom peripherals. */
  js_peripherals?: JsPeripheral[];
  /** USART base address used by uartRx() (default 0x40013800 = USART1). */
  uart_addr?: number;
  /** External device attachments (SPI flash, I2C EEPROM, OLED, LCD, etc.). */
  ext_devices?: ExtDevices;
  /** Print init info to console. */
  verbose?: boolean;
  /** Fixed batch size (overrides the adaptive 20K/50K policy). */
  batch_size?: number;
}

// ── Register set returned by getRegisters() ─────────────────────────────────

export interface Registers {
  R0: number; R1: number; R2: number; R3: number;
  R4: number; R5: number; R6: number; R7: number;
  R8: number; R9: number; R10: number; R11: number; R12: number;
  SP: number; LR: number; PC: number; xPSR: number;
}

// ── BluepillEmulator (returned by createEmulator) ───────────────────────────

export interface RunResult {
  totalSteps: number;
  instCount: number;
  stopped: boolean;
}

export interface StepResult {
  pc: number;
  instCount: number;
  stopped: boolean;
}

export interface BluepillEmulator {
  /** Raw 32-bit word read (used by drivers for RAM flags). */
  read32: (addr: number) => number;
  /** Raw 32-bit word write. */
  write32: (addr: number, val: number) => void;

  // ── Execution ─────────────────────────────────────────────────────────────

  /** Run up to N instructions (0 = forever). Loops adaptive 20K/50K batches. */
  run(maxInstructions?: number): RunResult;
  /** Run one batch (default 20K instructions), process DMA/interrupts/pin events. */
  step(maxBatch?: number): StepResult;
  /** Request stop of a running run() loop. */
  stop(): void;
  /** Tear down the emulator (no-op; state resets on init). */
  close(): void;

  // ── CPU state ─────────────────────────────────────────────────────────────

  /** Read all ARM registers. */
  getRegisters(): Registers;
  /** Read program counter. */
  getPc(): number;
  /** Read stack pointer. */
  getSp(): number;
  /** Write program counter. */
  setPc(pc: number): void;

  // ── Symbol resolution ─────────────────────────────────────────────────────

  /** Set symbol table for resolveSymbol(). Pass null to clear. */
  setSymbols(list: Array<{ name: string; addr: number }> | null): void;
  /** Number of loaded symbols. */
  getSymbolCount(): number;
  /** Resolve address to nearest symbol (e.g. 'main+0x1e'). */
  resolveSymbol(addr: number): string | null;

  // ── UART ──────────────────────────────────────────────────────────────────

  /** Collect USART1 TX output since last call. */
  getUartOutput(): string;
  /** Inject byte into UART RX (default USART1). */
  uartRx(byte: number): boolean;
  /** Inject byte into specific USART by base address. */
  uartRxAddr(addr: number, byte: number): boolean;
  /** Inject multiple bytes into UART RX. */
  uartRxBytes(bytes: Uint8Array | number[]): boolean;
  /** Unread bytes in UART RX buffer. */
  rxPending(): number;
  /** True while a DMA transfer is queued. */
  dmaPending(): boolean;

  // ── GPIO ──────────────────────────────────────────────────────────────────

  /** Read driven output level (port: 0=A, 1=B, 2=C). */
  gpioReadOutput(port: number, pin: number): boolean;
  /** Read input level. */
  gpioReadInput(port: number, pin: number): boolean;
  /** Drive an external input into a pin. */
  gpioSetInput(port: number, pin: number, value: boolean): void;
  /** Set analog wire voltage (12-bit, 0xFFFF clears). */
  gpioSetAnalog(port: number, pin: number, level: number): void;
  /** PWM duty (0-100) of a timer channel. */
  pwmDuty(addr: number, channel?: number): number;

  // ── ADC / Analog ──────────────────────────────────────────────────────────

  /** Set ADC simulation value. */
  setSimAdc(value: number): void;
  /** Set RC sample-and-hold time constant in ADC cycles. */
  adcSetRcTau(cycles: number): void;

  // ── Touchscreen ───────────────────────────────────────────────────────────

  /** Set touch coordinates on a touchscreen device. */
  setTouch(peripheral: string, x: number, y: number, pressure: number): void;

  // ── CAN ───────────────────────────────────────────────────────────────────

  /** Inject CAN message; returns true if accepted. */
  canInjectMessage(addr: number, tir: number, tdtr: number, tdlr: number, tdhr: number): boolean;
  /** Inject a USB SETUP packet (8 bytes) into EP0. Returns false when NAKed. */
  usbInjectSetup(bytes: Uint8Array): boolean;
  /** Inject a USB OUT packet into an endpoint. Returns false when NAKed. */
  usbInjectOut(ep: number, bytes: Uint8Array): boolean;

  // ── Bus observers ─────────────────────────────────────────────────────────

  /** Watch every peripheral register write; returns unsubscribe function. */
  onPeriphWrite(fn: (addr: number, width: number, value: number) => void): () => void;
  /** Watch chip-driven GPIO level changes; returns unsubscribe function. */
  onPinChange(fn: (port: number, pin: number, level: number) => void): () => void;

  // ── Event queue ───────────────────────────────────────────────────────────

  /** Drain buffered pin-change events (flat [port, pin, level, ...]). */
  takePinEvents(): Uint32Array | number[];
  /** Drain virtual-peripheral transaction events as flat i32 array. */
  drainEvents(): number[] | Int32Array;
  /** Queue MISO bytes for a SPI channel. */
  spiInjectMiso(channel: number, bytes: Uint8Array): void;
  /** Queue RX bytes for an I2C channel. */
  i2cInjectRx(channel: number, bytes: Uint8Array): void;

  // ── OLED / LCD framebuffers ───────────────────────────────────────────────

  /** I2C OLED framebuffer (page-major, 1 byte/column). */
  i2cOledFb(peripheral: string, address?: number): Uint8Array | null;
  /** SPI LCD framebuffer (128x64, 1 byte/pixel). */
  lcdFb(peripheral: string): Uint8Array | null;

  // ── FSMC ──────────────────────────────────────────────────────────────────

  /** Write byte directly into FSMC backing image. */
  fsmcWriteByte(name: string, offset: number, value: number): boolean;
  /** Read byte from FSMC backing image (-1 on error). */
  fsmcReadByte(name: string, offset: number): number;

  // ── Low-level peripheral access ───────────────────────────────────────────

  /** Read peripheral register (width: 1, 2, or 4 bytes; default 4). */
  periphRead(addr: number, width?: number): number;
  /** Write peripheral register. */
  periphWrite(addr: number, width: number, value: number): void;
  /** Read 32-bit word from emulated memory. */
  memRead32(addr: number): number;

  // ── Custom peripherals / Interrupt control ────────────────────────────────

  /** Register rp2040js-style custom peripheral. */
  addJsPeripheral(base: number, size: number, read: (addr: number, size: number) => number, write: (addr: number, value: number, size: number) => void): boolean;
}

/** Create a full STM32F103C8 emulator instance. */
export function createEmulator(opts?: CreateEmulatorOptions): Promise<BluepillEmulator>;
