use std::sync::atomic::{AtomicU32, AtomicU64, AtomicBool, AtomicI32, AtomicU8, Ordering};
use std::cell::RefCell;
use std::rc::Rc;
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

// Per-stream DMA interrupt info: IRQ number (-1 = none) and flags (bit 0=TCIE, 1=HTIE, 2=TEIE)
static DMA_STREAM_IRQ: [AtomicI32; 8] = [
    AtomicI32::new(-1), AtomicI32::new(-1), AtomicI32::new(-1), AtomicI32::new(-1),
    AtomicI32::new(-1), AtomicI32::new(-1), AtomicI32::new(-1), AtomicI32::new(-1),
];
static DMA_STREAM_FLAGS: [AtomicU8; 8] = [
    AtomicU8::new(0), AtomicU8::new(0), AtomicU8::new(0), AtomicU8::new(0),
    AtomicU8::new(0), AtomicU8::new(0), AtomicU8::new(0), AtomicU8::new(0),
];

pub fn set_dma_intr_info(stream_idx: usize, irq: i32, flags: u8) {
    if stream_idx < 8 {
        DMA_STREAM_IRQ[stream_idx].store(irq, Ordering::Release);
        DMA_STREAM_FLAGS[stream_idx].store(flags, Ordering::Release);
    }
}

pub struct WasmSystem {
    pub p: Rc<Peripherals>,
    pending_dma: RefCell<Vec<DmaTransfer>>,
    absorb_buf: RefCell<Vec<u8>>,
    /// Interrupt-dispatch policy state (batch budget, SVC mirror) — shared by
    /// cli.mjs and emulator.js so both use one implementation.
    pub intr: RefCell<crate::interrupts::IntrDispatch>,
}

impl WasmSystem {
    pub fn new() -> Self {
        let gpio = GpioPorts::default();
        let ext = get_ext_devices().lock().unwrap();
        let p = Rc::new(Peripherals::new_wasm(gpio, &*ext));
        drop(ext);
        Self::register_software_spis(&p);
        Self::register_touchscreen_gpios(&p);
        WasmSystem { p, pending_dma: RefCell::new(Vec::new()), absorb_buf: RefCell::new(Vec::new()), intr: RefCell::new(crate::interrupts::IntrDispatch::default()) }
    }

    pub fn new_svd(svd_xml: &str) -> Self {
        let gpio = GpioPorts::default();
        let ext = get_ext_devices().lock().unwrap();
        let p = Rc::new(Peripherals::from_svd(svd_xml, gpio, &*ext));
        drop(ext);
        Self::register_software_spis(&p);
        Self::register_touchscreen_gpios(&p);
        WasmSystem { p, pending_dma: RefCell::new(Vec::new()), absorb_buf: RefCell::new(Vec::new()), intr: RefCell::new(crate::interrupts::IntrDispatch::default()) }
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

    /// Fetch [offset, offset+len) of the absorbed bytes and clear the buffer.
    pub fn dma_absorb_take(&self, offset: usize, len: usize) -> Vec<u8> {
        let mut buf = self.absorb_buf.borrow_mut();
        let out = if offset < buf.len() {
            buf[offset..buf.len().min(offset + len)].to_vec()
        } else {
            Vec::new()
        };
        buf.clear();
        out
    }

    pub fn mark_dma_completed(&self, stream_idx: usize, _success: bool) {
        if stream_idx < 8 {
            DMA_COMPLETION_BITS.fetch_or(1 << stream_idx, Ordering::Release);
        }
        // Fire NVIC interrupt after transfer completes
        if stream_idx < 8 {
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
        if stream_idx < 8 {
            let mask = 1 << stream_idx;
            DMA_COMPLETION_BITS.fetch_and(!mask, Ordering::Acquire) & mask != 0
        } else {
            false
        }
    }

    pub fn dma_take_completions(&self) -> u32 {
        DMA_COMPLETION_BITS.swap(0, Ordering::Acquire)
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
