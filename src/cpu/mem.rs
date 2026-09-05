use std::cell::Cell;

pub trait Memory {
    fn read8(&self, addr: u32) -> u8;
    fn read16(&self, addr: u32) -> u16;
    fn read32(&self, addr: u32) -> u32;
    fn write8(&mut self, addr: u32, v: u8);
    fn write16(&mut self, addr: u32, v: u16);
    fn write32(&mut self, addr: u32, v: u32);
    /// Raw access bypassing MPU checks (firmware install, DMA as trusted bus
    /// master, driver/debugger reads). Defaults honor checks; FlatMemory
    /// overrides to go straight to the backing bytes.
    fn read8_raw(&self, addr: u32) -> u8 {
        self.read8(addr)
    }
    fn write8_raw(&mut self, addr: u32, v: u8) {
        self.write8(addr, v)
    }
    fn read16_raw(&self, addr: u32) -> u16 {
        self.read16(addr)
    }
}

pub struct MemRegion {
    pub base: u32,
    pub data: Vec<u8>,
}

fn is_periph(addr: u32) -> bool {
    (addr >= 0x40000000 && addr < 0x51000000)
        || (addr >= 0x60000000 && addr < 0x62000000)
        || (addr >= 0xA0000000 && addr < 0xA2000000)
        || (addr >= 0xE0000000 && addr < 0xE1000000)
}

/// Flat guest memory for the WASM-native CPU: flash + main SRAM + any number
/// of extra RAM/ROM regions (doom's EXTRAM at 0xC0000000, the WAD image at
/// 0xB8000000, ...). Peripheral addresses route into the Rust model via the
/// process-wide `SYS` instance (which `init`/`init_svd` installs before any
/// `WasmCpu` is created).
///
/// Flash is execute/read-only for the guest: normal `write8` stores to flash
/// are ignored (real flash needs an erase/program sequence). `load()` bypasses
/// the protection
/// and is the only way to get firmware into flash.
pub struct FlatMemory {
    pub flash: Vec<u8>,
    pub ram: Vec<u8>,
    pub extra: Vec<MemRegion>,
    pub flash_base: u32,
    pub ram_base: u32,
    /// Last unmapped access (read or write), for diagnostics. Reads of
    /// unmapped memory return 0; writes are dropped.
    pub bad: Cell<Option<u32>>,
}

impl FlatMemory {
    pub fn new(flash_size: usize, ram_size: usize) -> Self {
        Self {
            flash: vec![0; flash_size],
            ram: vec![0; ram_size],
            extra: Vec::new(),
            flash_base: 0x08000000,
            ram_base: 0x20000000,
            bad: Cell::new(None),
        }
    }

    fn in_flash(&self, addr: u32) -> bool {
        addr >= self.flash_base && (addr - self.flash_base) < self.flash.len() as u32
    }
    fn in_ram(&self, addr: u32) -> bool {
        addr >= self.ram_base && (addr - self.ram_base) < self.ram.len() as u32
    }
    fn extra_idx(&self, addr: u32) -> Option<usize> {
        self.extra
            .iter()
            .position(|r| addr >= r.base && (addr - r.base) < r.data.len() as u32)
    }

    /// Map a zeroed extra region in bulk. `load()` builds regions byte by
    /// byte (quadratic for megabyte images); pre-mapping makes WAD/EXTRAM
    /// setup O(size) with a fast fill. Matches the JS driver, which zeroes
    /// every extra_ram region at map time.
    pub fn map_extra(&mut self, base: u32, size: usize) {
        self.extra.push(MemRegion { base, data: vec![0; size] });
    }

    /// Load `data` at `base`, writing through flash protection. Bytes landing
    /// in flash or RAM go there; bytes landing anywhere else create (or
    /// extend) an extra region, so ELF segments / WAD images just work.
    pub fn load(&mut self, data: &[u8], base: u32) {
        for (i, &b) in data.iter().enumerate() {
            let a = base.wrapping_add(i as u32);
            if self.in_flash(a) {
                self.flash[(a - self.flash_base) as usize] = b;
            } else if self.in_ram(a) {
                self.ram[(a - self.ram_base) as usize] = b;
            } else if is_periph(a) {
                // never load firmware into MMIO
            } else if let Some(idx) = self.extra_idx(a) {
                let r = &mut self.extra[idx];
                r.data[(a - r.base) as usize] = b;
            } else {
                // extend backwards into a touching region, else create one
                let mut done = false;
                for r in self.extra.iter_mut() {
                    if a.wrapping_add(1) == r.base {
                        r.base = a;
                        r.data.insert(0, b);
                        done = true;
                        break;
                    }
                    if a == r.base.wrapping_add(r.data.len() as u32) {
                        r.data.push(b);
                        done = true;
                        break;
                    }
                }
                if !done {
                    self.extra.push(MemRegion { base: a, data: vec![b] });
                }
            }
        }
    }
}

impl Memory for FlatMemory {
    fn read8(&self, addr: u32) -> u8 {
        // MPU data-read gate: one mirrored-flag load + branch when disabled
        // (debugger, DMA and firmware-install paths use raw variants instead).
        // The peripheral arm is outlined cold so the hot RAM/flash skeleton
        // stays small enough for the JIT to inline (see MPU_ON docs).
        if crate::system::mpu_gate_on() {
            core::hint::cold_path();
            if !crate::sys().mpu_check_data_slow(addr, false) {
                return 0;
            }
        }
        if is_periph(addr) {
            return self.read8_periph_cold(addr);
        }
        self.read8_raw_unchecked(addr)
    }

    fn read8_raw(&self, addr: u32) -> u8 {
        if is_periph(addr) {
            return self.read8_periph_cold(addr);
        }
        self.read8_raw_unchecked(addr)
    }

    fn read16(&self, addr: u32) -> u16 {
        if is_periph(addr) {
            return crate::sys().p.read(crate::sys(), addr, 2) as u16;
        }
        let lo = self.read8(addr) as u16;
        let hi = self.read8(addr + 1) as u16;
        lo | (hi << 8)
    }
    fn read16_raw(&self, addr: u32) -> u16 {
        // Raw fetch decoration (MPU fault records); periph window still
        // routes to the model (a fetch there is diagnosed, not executed).
        if is_periph(addr) {
            return crate::sys().p.read(crate::sys(), addr, 2) as u16;
        }
        let lo = self.read8_raw(addr) as u16;
        let hi = self.read8_raw(addr + 1) as u16;
        lo | (hi << 8)
    }
    fn read32(&self, addr: u32) -> u32 {
        if is_periph(addr) {
            return crate::sys().p.read(crate::sys(), addr, 4);
        }
        let b0 = self.read8(addr) as u32;
        let b1 = self.read8(addr + 1) as u32;
        let b2 = self.read8(addr + 2) as u32;
        let b3 = self.read8(addr + 3) as u32;
        b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
    }
    fn write8(&mut self, addr: u32, v: u8) {
        // MPU data-write gate (denials drop the store).
        if crate::system::mpu_gate_on() {
            core::hint::cold_path();
            if !crate::sys().mpu_check_data_slow(addr, true) {
                return;
            }
        }
        if is_periph(addr) {
            return self.write8_periph_cold(addr, v);
        }
        self.write8_raw_unchecked(addr, v)
    }

    fn write8_raw(&mut self, addr: u32, v: u8) {
        if is_periph(addr) {
            return self.write8_periph_cold(addr, v);
        }
        self.write8_raw_unchecked(addr, v)
    }

    fn write16(&mut self, addr: u32, v: u16) {
        if is_periph(addr) {
            crate::sys().p.write(crate::sys(), addr, 2, v as u32);
            return;
        }
        self.write8(addr, (v & 0xFF) as u8);
        self.write8(addr + 1, (v >> 8) as u8);
    }
    fn write32(&mut self, addr: u32, v: u32) {
        if is_periph(addr) {
            crate::sys().p.write(crate::sys(), addr, 4, v);
            return;
        }
        self.write8(addr, (v & 0xFF) as u8);
        self.write8(addr + 1, ((v >> 8) & 0xFF) as u8);
        self.write8(addr + 2, ((v >> 16) & 0xFF) as u8);
        self.write8(addr + 3, ((v >> 24) & 0xFF) as u8);
    }
}

impl FlatMemory {
    /// Hot RAM/flash/extra body shared by read8/read8_raw. Kept tiny so the
    /// JIT keeps inlining the gated callers; peripheral + MPU arms live in
    /// cold out-of-line fns below.
    #[inline(always)]
    fn read8_raw_unchecked(&self, addr: u32) -> u8 {
        if self.in_flash(addr) {
            self.flash[(addr - self.flash_base) as usize]
        } else if self.in_ram(addr) {
            self.ram[(addr - self.ram_base) as usize]
        } else if let Some(idx) = self.extra_idx(addr) {
            let r = &self.extra[idx];
            r.data[(addr - r.base) as usize]
        } else {
            self.bad.set(Some(addr));
            0
        }
    }

    #[inline(always)]
    fn write8_raw_unchecked(&mut self, addr: u32, v: u8) {
        if self.in_flash(addr) {
            // flash protection: guest stores are ignored (see struct docs)
        } else if self.in_ram(addr) {
            self.ram[(addr - self.ram_base) as usize] = v;
        } else if let Some(idx) = self.extra_idx(addr) {
            let r = &mut self.extra[idx];
            r.data[(addr - r.base) as usize] = v;
        } else {
            self.bad.set(Some(addr));
        }
    }

    #[cold]
    #[inline(never)]
    fn read8_periph_cold(&self, addr: u32) -> u8 {
        // Single width-1 model read (mirrors the JS memReadHook, which
        // takes the low byte). The model aligns internally.
        crate::sys().p.read(crate::sys(), addr, 1) as u8
    }

    #[cold]
    #[inline(never)]
    fn write8_periph_cold(&self, addr: u32, v: u8) {
        // Single width-1 model write. Never split a wider guest store
        // into byte RMWs here: each model write can have side effects
        // (a USART DR write emits a UART char), so one guest store must
        // equal exactly one model call, like the JS memWriteHook.
        crate::sys().p.write(crate::sys(), addr, 1, v as u32);
    }
}
