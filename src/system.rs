use std::sync::atomic::{AtomicU32, AtomicU64, AtomicBool, AtomicI32, AtomicU8, Ordering};
use std::cell::RefCell;
use std::cell::Cell;
use std::rc::Rc;
use std::collections::HashMap;
use std::sync::Mutex;
use crate::peripherals::{Peripherals, gpio::GpioPorts};
use crate::ext_devices::ExtDevices;

// UART output buffer: USART write_dr pushes chars here, JS reads via get_uart_output()
use std::sync::OnceLock;
static UART_OUTPUT: OnceLock<Mutex<String>> = OnceLock::new();
pub fn get_uart_output() -> &'static Mutex<String> {
    UART_OUTPUT.get_or_init(|| Mutex::new(String::new()))
}

// Global ExtDevices: populated by JS add_* calls before init
static EXT_DEVICES: OnceLock<Mutex<ExtDevices>> = OnceLock::new();
pub fn get_ext_devices() -> &'static Mutex<ExtDevices> {
    EXT_DEVICES.get_or_init(|| Mutex::new(ExtDevices::default()))
}

pub static INSTRUCTION_COUNT: AtomicU64 = AtomicU64::new(0);
pub fn instruction_count() -> u64 { INSTRUCTION_COUNT.load(Ordering::Relaxed) }

// Interrupt masks set by JS from Unicorn CPU state on each instruction.
pub static INTR_MASK_PRIMASK: AtomicU32 = AtomicU32::new(0);
pub static INTR_MASK_BASEPRI: AtomicU32 = AtomicU32::new(0);

static WATCHDOG_RESET: AtomicBool = AtomicBool::new(false);

// Software SPI configs queued before init, registered after GPIO exists
static SOFTWARE_SPI_CONFIGS: OnceLock<Mutex<Vec<(String, Option<String>, String, String, String)>>> = OnceLock::new();
pub fn get_software_spi_configs() -> &'static Mutex<Vec<(String, Option<String>, String, String, String)>> {
    SOFTWARE_SPI_CONFIGS.get_or_init(|| Mutex::new(Vec::new()))
}
pub fn is_watchdog_reset_requested() -> bool { WATCHDOG_RESET.swap(false, Ordering::Acquire) }
pub fn request_watchdog_reset() { WATCHDOG_RESET.store(true, Ordering::Release); }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaDir { Read, Write, MemCopy }

#[derive(Debug, Clone)]
pub struct DmaTransfer {
    pub direction: DmaDir,
    pub stream_idx: usize,
    pub dma_name: String,
    pub src: u32,
    pub dst: u32,
    pub size: usize,
    pub peri_addr: u32,
    pub peripheral: bool,
}

impl DmaTransfer {
    pub fn to_u32_vec(&self) -> Vec<u32> {
        vec![
            self.direction as u32,
            self.stream_idx as u32,
            self.src,
            self.dst,
            self.size as u32,
            self.peri_addr,
            self.peripheral as u32,
        ]
    }
}

static DMA_COMPLETION_BITS: AtomicU32 = AtomicU32::new(0);

// Per-stream DMA interrupt info: IRQ number (-1 = none) and flags (bit 0=TCIE, 1=HTIE, 2=TEIE).
// Streams are GLOBAL across both DMAs (DMA1 ch0-6 -> 0-6, DMA2 ch0-4 -> 7-11).
static DMA_STREAM_IRQ: [AtomicI32; 12] = [
    AtomicI32::new(-1), AtomicI32::new(-1), AtomicI32::new(-1), AtomicI32::new(-1),
    AtomicI32::new(-1), AtomicI32::new(-1), AtomicI32::new(-1), AtomicI32::new(-1),
    AtomicI32::new(-1), AtomicI32::new(-1), AtomicI32::new(-1), AtomicI32::new(-1),
];
static DMA_STREAM_FLAGS: [AtomicU8; 12] = [
    AtomicU8::new(0), AtomicU8::new(0), AtomicU8::new(0), AtomicU8::new(0),
    AtomicU8::new(0), AtomicU8::new(0), AtomicU8::new(0), AtomicU8::new(0),
    AtomicU8::new(0), AtomicU8::new(0), AtomicU8::new(0), AtomicU8::new(0),
];

pub fn set_dma_intr_info(stream_idx: usize, irq: i32, flags: u8) {
    if stream_idx < 12 {
        DMA_STREAM_IRQ[stream_idx].store(irq, Ordering::Release);
        DMA_STREAM_FLAGS[stream_idx].store(flags, Ordering::Release);
    }
}

/// Virtual-peripheral transaction events drained by JS via `drain_events()`.
#[derive(Debug, Clone)]
pub enum VmEvent {
    SpiTransfer { channel: u8, tx: Vec<u8>, rx: Vec<u8> },
    I2cStart { channel: u8, addr: u8 },
    I2cWrite { channel: u8, byte: u8 },
    I2cRead { channel: u8 },
    I2cStop { channel: u8 },
    UartTx { usart: u8, byte: u8 },
    ExtiEdge { line: u8 },
    AdcDone { adc: u8, chan: u8 },
    TimUpdate { tim: u8 },
    DacWrite { chan: u8, value: u32 },
    CrcResult { value: u32 },
    RtcAlarm { alarm: u32 },
    WdogReset { which: u8 },
    CanTx { can: u8, id: u32, len: u8, data: [u8; 8] },
    CanRx { can: u8, id: u32, len: u8, data: [u8; 8] },
    TimCapture { tim: u8, ch: u8, value: u32 },
    FsmcAccess { bank: u8, offset: u32, write: bool, size: u8, value: u32 },
    UsbIn { ep: u8, data: Vec<u8> },
}

pub struct WasmSystem {
    pub p: Rc<Peripherals>,
    pending_dma: RefCell<Vec<DmaTransfer>>,
    absorb_buf: RefCell<Vec<u8>>,
    /// Virtual-peripheral transaction event queue (SPI/I2C/USART), drained by JS.
    pub event_queue: RefCell<Vec<VmEvent>>,
    /// Injected MISO bytes per SPI channel (virtual device -> MCU).
    pub spi_miso: RefCell<HashMap<u8, Vec<u8>>>,
    /// Injected RX bytes per I2C channel (virtual device -> MCU).
    pub i2c_rx: RefCell<HashMap<u8, Vec<u8>>>,
    /// Interrupt-dispatch policy state (batch budget, SVC mirror) — shared by
    /// cli.mjs and emulator.js so both use one implementation.
    pub intr: RefCell<crate::interrupts::IntrDispatch>,
    /// Set when I2C1 DR is written with the R-bit set; the native driver
    /// drains it per batch for the hi2c Mode RAM patch (same condition as
    /// the Unicorn memWriteHook). Taken (cleared) on read.
    pub i2c_dr_hook: Cell<bool>,
}

impl WasmSystem {
    pub fn new() -> Self {
        let gpio = GpioPorts::default();
        let ext = get_ext_devices().lock().unwrap();
        let p = Rc::new(Peripherals::new_wasm(gpio, &*ext));
        drop(ext);
        Self::register_software_spis(&p);
        Self::register_touchscreen_gpios(&p);
        WasmSystem { p, pending_dma: RefCell::new(Vec::new()), absorb_buf: RefCell::new(Vec::new()),
            event_queue: RefCell::new(Vec::new()), spi_miso: RefCell::new(HashMap::new()),
            i2c_rx: RefCell::new(HashMap::new()), intr: RefCell::new(crate::interrupts::IntrDispatch::default()),
            i2c_dr_hook: Cell::new(false) }
    }

    pub fn new_svd(svd_xml: &str) -> Self {
        let gpio = GpioPorts::default();
        let ext = get_ext_devices().lock().unwrap();
        let p = Rc::new(Peripherals::from_svd(svd_xml, gpio, &*ext));
        drop(ext);
        Self::register_software_spis(&p);
        Self::register_touchscreen_gpios(&p);
        WasmSystem { p, pending_dma: RefCell::new(Vec::new()), absorb_buf: RefCell::new(Vec::new()),
            event_queue: RefCell::new(Vec::new()), spi_miso: RefCell::new(HashMap::new()),
            i2c_rx: RefCell::new(HashMap::new()), intr: RefCell::new(crate::interrupts::IntrDispatch::default()),
            i2c_dr_hook: Cell::new(false) }
    }

    fn register_software_spis(p: &Peripherals) {
        use crate::peripherals::sw_spi::{SoftwareSpi, SoftwareSpiConfig};
        let configs = get_software_spi_configs().lock().unwrap();
        let ext_devices = get_ext_devices().lock().unwrap();
        for (name, cs, clk, miso, mosi) in configs.iter() {
            let config = SoftwareSpiConfig {
                name: name.clone(),
                cs: cs.clone(),
                clk: clk.clone(),
                miso: miso.clone(),
                mosi: mosi.clone(),
            };
            SoftwareSpi::register(config, &mut p.gpio.borrow_mut(), &ext_devices);
        }
    }

    fn register_touchscreen_gpios(p: &Peripherals) {
        let ext_devices = get_ext_devices().lock().unwrap();
        for ts in &ext_devices.touchscreens {
            ts.borrow_mut().setup_gpio(&mut p.gpio.borrow_mut());
        }
    }

    pub fn queue_dma_transfer(&self, t: DmaTransfer) {
        let mut pending = self.pending_dma.borrow_mut();
        if pending.iter().any(|x| x.stream_idx == t.stream_idx) {
            return; // channel already has a transfer queued; ignore re-arms
        }
        pending.push(t);
    }

    pub fn pending_dma_count(&self) -> usize {
        self.pending_dma.borrow().len()
    }

    pub fn take_pending_dma_transfer(&self, index: usize) -> Option<DmaTransfer> {
        let mut pending = self.pending_dma.borrow_mut();
        if index < pending.len() {
            Some(pending.remove(index))
        } else {
            None
        }
    }

    pub fn take_pending_dma_transfers(&self) -> Vec<DmaTransfer> {
        let mut pending = self.pending_dma.borrow_mut();
        std::mem::take(&mut *pending)
    }

    /// DMA pump helper: absorb `size` bytes from the peripheral at `addr`
    /// (periph_read chunks <= 4, little-endian packed) into the side buffer.
    /// Returns the byte offset for dma_absorb_take(). Absorbs immediately so
    /// JS only writes the result to RAM once per transfer.
    pub fn dma_absorb_store(&self, addr: u32, size: usize) -> usize {
        let mut buf = self.absorb_buf.borrow_mut();
        let off = buf.len();
        let mut j = 0u32;
        while (j as usize) < size {
            let chunk = std::cmp::min(4, size as u32 - j);
            let val = self.p.read(&*self, addr, chunk as u8);
            for k in 0..chunk {
                buf.push(((val >> (k * 8)) & 0xFF) as u8);
            }
            j += chunk;
        }
        off
    }

    /// Fetch [offset, offset+len) of the absorbed bytes. One pump can absorb
    /// for SEVERAL periph->mem transfers, each taken with its own offset, so
    /// the buffer is only released once the take reaches its end — clearing on
    /// the first take dropped every later transfer's bytes. dma_pump_all()
    /// clears it up front, so an abandoned plan cannot leak into the next pump.
    pub fn dma_absorb_take(&self, offset: usize, len: usize) -> Vec<u8> {
        let mut buf = self.absorb_buf.borrow_mut();
        if offset >= buf.len() {
            return Vec::new();
        }
        let end = buf.len().min(offset.saturating_add(len));
        let out = buf[offset..end].to_vec();
        if end >= buf.len() {
            buf.clear();
        }
        out
    }

    /// Drop any bytes left over from a previous pump (see dma_absorb_take).
    pub fn dma_absorb_reset(&self) {
        self.absorb_buf.borrow_mut().clear();
    }

    /// Build the flat DMA op plan (see dma_pump_all in lib.rs): pops ALL
    /// pending transfers, absorbs periph->mem bytes internally, and returns
    /// [op,a,b,c] quadruples. Moved here from lib.rs so native (non-JS)
    /// drivers can build the same plan.
    pub fn dma_build_plan(&self) -> Vec<u32> {
        self.dma_absorb_reset();
        let mut plan: Vec<u32> = Vec::new();
        let mut done_bits = 0u32;
        for t in self.take_pending_dma_transfers() {
            done_bits |= 1 << t.stream_idx;
            if t.direction == DmaDir::MemCopy || !t.peripheral {
                plan.extend([0, t.src, t.dst, t.size as u32]);
            } else if t.direction == DmaDir::Read {
                // periph -> mem: absorb now, executor writes the bytes to RAM
                let off = self.dma_absorb_store(t.peri_addr, t.size);
                plan.extend([1, t.dst, t.size as u32, off as u32]);
            } else {
                // mem -> periph: executor reads RAM, then pushes via dma_push_periph
                plan.extend([2, t.src, t.size as u32, t.peri_addr]);
            }
        }
        if done_bits != 0 {
            plan.extend([3, done_bits, 0, 0]);
        }
        plan
    }

    /// Execute a plan from dma_build_plan() against a Rust memory (the
    /// native equivalent of processDma in cli.mjs/emulator.js, which uses
    /// Unicorn mem_read/mem_write for the same ops).
    pub fn dma_exec_plan(&self, mem: &mut dyn crate::cpu::mem::Memory, plan: &[u32]) {
        let mut i = 0;
        while i + 4 <= plan.len() {
            let (op, a, b, c) = (plan[i], plan[i + 1], plan[i + 2], plan[i + 3]);
            match op {
                0 => {
                    for k in 0..c {
                        let v = mem.read8(a.wrapping_add(k));
                        mem.write8(b.wrapping_add(k), v);
                    }
                }
                1 => {
                    for (k, v) in self.dma_absorb_take(c as usize, b as usize).into_iter().enumerate() {
                        mem.write8(a.wrapping_add(k as u32), v);
                    }
                }
                2 => {
                    // mem -> periph: read RAM, push through the normal
                    // peripheral write path in <=4B chunks (mirrors the
                    // dma_push_periph wasm export).
                    let mut k = 0usize;
                    let n = b as usize;
                    while k < n {
                        let chunk = std::cmp::min(4, n - k);
                        let mut val = 0u32;
                        for j in 0..chunk {
                            val |= (mem.read8(a.wrapping_add((k + j) as u32)) as u32) << (j * 8);
                        }
                        self.p.write(self, c, chunk as u8, val);
                        k += chunk;
                    }
                }
                _ => {
                    for stream in 0..12 {
                        if a & (1 << stream) != 0 {
                            self.mark_dma_completed(stream, true);
                        }
                    }
                }
            }
            i += 4;
        }
    }

    pub fn mark_dma_completed(&self, stream_idx: usize, _success: bool) {
        if stream_idx < 12 {
            DMA_COMPLETION_BITS.fetch_or(1 << stream_idx, Ordering::Release);
        }
        // Fire NVIC interrupt after transfer completes
        if stream_idx < 12 {
            let irq = DMA_STREAM_IRQ[stream_idx].swap(-1, Ordering::Acquire);
            if irq >= 0 {
                let flags = DMA_STREAM_FLAGS[stream_idx].swap(0, Ordering::Acquire);
                if flags & 0x7 != 0 {
                    self.p.nvic.borrow_mut().set_intr_pending(irq);
                }
            }
        }
    }

    pub fn dma_check_completion(&self, stream_idx: usize) -> bool {
        if stream_idx < 12 {
            let mask = 1 << stream_idx;
            DMA_COMPLETION_BITS.fetch_and(!mask, Ordering::Acquire) & mask != 0
        } else {
            false
        }
    }

    pub fn dma_take_completions(&self) -> u32 {
        DMA_COMPLETION_BITS.swap(0, Ordering::Acquire)
    }

    /// Take only the completion bits in `mask`, leaving other streams' bits
    /// queued for their own DMA's tick.
    pub fn dma_take_completions_masked(&self, mask: u32) -> u32 {
        let old = DMA_COMPLETION_BITS.load(Ordering::Acquire);
        DMA_COMPLETION_BITS.fetch_and(!mask, Ordering::AcqRel);
        old & mask
    }

    /// Append a virtual-peripheral event to the drain queue.
    pub fn push_event(&self, e: VmEvent) {
        self.event_queue.borrow_mut().push(e);
    }

    /// Take (and clear) all buffered virtual-peripheral events.
    pub fn take_events(&self) -> Vec<VmEvent> {
        std::mem::take(&mut *self.event_queue.borrow_mut())
    }

    /// Queue injected MISO bytes for a SPI channel (virtual device -> MCU).
    pub fn spi_inject_miso(&self, channel: u8, bytes: &[u8]) {
        self.spi_miso.borrow_mut().entry(channel).or_default().extend_from_slice(bytes);
    }

    /// Pop the next injected MISO byte for a SPI channel, if any.
    pub fn spi_take_miso(&self, channel: u8) -> Option<u8> {
        let mut m = self.spi_miso.borrow_mut();
        m.get_mut(&channel).and_then(|v| { if v.is_empty() { None } else { Some(v.remove(0)) } })
    }

    /// Queue injected RX bytes for an I2C channel (virtual device -> MCU).
    pub fn i2c_inject_rx(&self, channel: u8, bytes: &[u8]) {
        self.i2c_rx.borrow_mut().entry(channel).or_default().extend_from_slice(bytes);
    }

    /// Pop the next injected RX byte for an I2C channel, if any.
    pub fn i2c_take_rx(&self, channel: u8) -> Option<u8> {
        let mut m = self.i2c_rx.borrow_mut();
        m.get_mut(&channel).and_then(|v| { if v.is_empty() { None } else { Some(v.remove(0)) } })
    }

    pub fn tick(&self) {
        let p = self.p.clone();
        let deep = p.in_deep_sleep();
        let bus = p.bus.borrow();
        for &idx in bus.tick_indices() {
            if deep {
                // STOP/STANDBY: only LSI/LSE-clocked peripherals keep running
                // (IWDG @ 0x40003000, RTC @ 0x40002800); everything else freezes.
                let base = bus.slot_at(idx).start;
                if base != 0x4000_3000 && base != 0x4000_2800 {
                    bus.slot_at(idx).peripheral.borrow_mut().tick_frozen(self);
                    continue;
                }
            }
            bus.slot_at(idx).peripheral.borrow_mut().tick(self);
        }
        drop(bus);
        // SysTick runs off HCLK: frozen in STOP/STANDBY.
        if !deep {
            p.nvic.borrow_mut().maybe_set_systick_intr_pending();
        }
    }

    pub fn addr_desc(&self, addr: u32) -> String {
        self.p.addr_desc(addr)
    }
}

pub type System = WasmSystem;

unsafe impl Sync for WasmSystem {}
unsafe impl Send for WasmSystem {}

#[cfg(test)]
mod tests {
    use crate::test_util::with_sys;

    /// DMA1 channel 1 CMAR — a plain 32-bit register to absorb known bytes from.
    const MAR: u32 = 0x4002_0014;

    #[test]
    fn absorb_buffer_serves_every_transfer_in_a_pump() {
        with_sys(|sys| {
            sys.p.write(sys, MAR, 4, 0xAABB_CCDD);

            // One pump can absorb for several periph->mem transfers; JS takes
            // each one by its own offset, in plan order.
            let first = sys.dma_absorb_store(MAR, 4);
            let second = sys.dma_absorb_store(MAR, 4);
            assert_eq!((first, second), (0, 4), "offsets are sequential");

            assert_eq!(sys.dma_absorb_take(first, 4), vec![0xDD, 0xCC, 0xBB, 0xAA]);
            // Taking the first slice must NOT discard the rest.
            assert_eq!(sys.dma_absorb_take(second, 4), vec![0xDD, 0xCC, 0xBB, 0xAA]);

            // The final take releases the buffer.
            assert!(sys.dma_absorb_take(0, 4).is_empty());
        });
    }

    #[test]
    fn absorb_buffer_is_reset_between_pumps() {
        with_sys(|sys| {
            sys.p.write(sys, MAR, 4, 0x1122_3344);
            sys.dma_absorb_store(MAR, 4);
            // An abandoned plan (JS threw before taking) must not shift the
            // offsets of the next pump.
            sys.dma_absorb_reset();
            assert_eq!(sys.dma_absorb_store(MAR, 4), 0);
            assert_eq!(sys.dma_absorb_take(0, 4), vec![0x44, 0x33, 0x22, 0x11]);
        });
    }

    #[test]
    fn absorb_take_clamps_out_of_range_requests() {
        with_sys(|sys| {
            sys.dma_absorb_store(MAR, 4);
            assert!(sys.dma_absorb_take(8, 4).is_empty(), "offset past the end");
            assert_eq!(sys.dma_absorb_take(2, 99).len(), 2, "length past the end");
        });
    }
}
