//! Memory-mapped peripheral bus (rp2040js-style registry).
//!
//! Peripherals are registered with a `[start, end)` address range; accesses
//! are routed with a binary search over the (sorted) slot list. New
//! peripherals can be added at runtime from Rust (`Bus::register`) or from
//! JS via `register_js_peripheral` (wasm export) — a `JsPeripheral` wrapping
//! JS read/write callbacks.

use std::cell::{RefCell, Cell};

use wasm_bindgen::prelude::*;

use crate::peripherals::{Peripheral, PeripheralSlot};
use crate::system::System;

/// Sorted registry of peripheral slots with tick bookkeeping.
#[derive(Default)]
pub struct Bus {
    slots: Vec<PeripheralSlot<RefCell<Box<dyn Peripheral>>>>,
    tick_indices: Vec<usize>,
    /// Temporal-locality cache for `get()`: the last matched slot's
    /// `[start, end)` range + index. WASM is single-threaded, so `Cell`
    /// interior mutability is safe. Cleared on every `register()` so it can
    /// never go stale when the bus is rebuilt (e.g. `register_js_peripheral`
    /// after init). A hit only short-circuits when the address still falls in
    /// the cached slot, so behavior is identical to the binary search.
    last: Cell<Option<(u32, u32, usize)>>,
}

impl Bus {
    pub fn new() -> Self {
        Bus { slots: Vec::new(), tick_indices: Vec::new(), last: Cell::new(None) }
    }

    /// Register a peripheral covering `[start, end)`. Re-registering an
    /// overlapping range replaces the old slot (rp2040js semantics: last
    /// registration wins), so custom peripherals can shadow built-ins.
    pub fn register(&mut self, start: u32, end: u32, tick: bool, p: Box<dyn Peripheral>) {
        if start >= end {
            // Empty range is nonsensical — skip it rather than aborting the WASM
            // module (a bad registration should never crash the emulator).
            return;
        }
        let slot = PeripheralSlot { start, end, tick, peripheral: RefCell::new(p) };
        self.slots.retain(|s| s.end <= start || s.start >= end);
        self.slots.push(slot);
        self.slots.sort_by_key(|s| s.start);
        self.rebuild_tick_indices();
        self.last.set(None);
    }

    /// Builder-time overlap check (used by the board constructors).
    pub fn finish_assert_no_overlap(&self) {
        for w in self.slots.windows(2) {
            assert!(w[0].end <= w[1].start,
                "Overlap: 0x{:08x}-0x{:08x} vs 0x{:08x}-0x{:08x}",
                w[0].start, w[0].end, w[1].start, w[1].end);
        }
    }

    fn rebuild_tick_indices(&mut self) {
        self.tick_indices = self.slots.iter().enumerate()
            .filter(|(_, s)| s.tick)
            .map(|(i, _)| i)
            .collect();
    }

    /// Binary search for the slot covering `addr`. A same-slot hit (same
    /// address, or sequential accesses within one peripheral / FSMC bank)
    /// returns the cached slot without re-searching.
    pub fn get(&self, addr: u32) -> Option<&PeripheralSlot<RefCell<Box<dyn Peripheral>>>> {
        if let Some((start, end, idx)) = self.last.get() {
            if addr >= start && addr < end {
                // idx is valid: slots only change via register(), which clears
                // the cache. Coverage is already guaranteed by the range check.
                return self.slots.get(idx);
            }
        }
        let index = self.slots.binary_search_by_key(&addr, |p| p.start)
            .map_or_else(|e| e.checked_sub(1), |v| Some(v));
        // Ranges are [start, end): an address exactly at `end` belongs to the
        // next slot (or to a gap), never to this one.
        let found = index.and_then(|i| self.slots.get(i).filter(|p| addr < p.end));
        if let Some(s) = found {
            if let Some(i) = index {
                self.last.set(Some((s.start, s.end, i)));
            }
        }
        found
    }

    pub fn iter(&self) -> impl Iterator<Item = &PeripheralSlot<RefCell<Box<dyn Peripheral>>>> {
        self.slots.iter()
    }

    pub fn slot_at(&self, idx: usize) -> &PeripheralSlot<RefCell<Box<dyn Peripheral>>> {
        &self.slots[idx]
    }

    pub fn tick_indices(&self) -> &[usize] {
        &self.tick_indices
    }

    pub fn len(&self) -> usize {
        self.slots.len()
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dummy;
    impl Peripheral for Dummy {
        fn read(&mut self, _sys: &System, _offset: u32) -> u32 { 0 }
        fn write(&mut self, _sys: &System, _offset: u32, _value: u32) {}
    }

    fn bus_with(ranges: &[(u32, u32)]) -> Bus {
        let mut bus = Bus::new();
        for &(start, end) in ranges {
            bus.register(start, end, false, Box::new(Dummy));
        }
        bus
    }

    #[test]
    fn decodes_range_as_half_open() {
        // Two slots with a gap between them, mirroring GPIOD (ends 0x40011800)
        // and ADC1 (starts 0x40012400) in the hardcoded map.
        let bus = bus_with(&[(0x4001_1400, 0x4001_1800), (0x4001_2400, 0x4001_2800)]);

        assert_eq!(bus.get(0x4001_1400).map(|s| s.start), Some(0x4001_1400));
        assert_eq!(bus.get(0x4001_17FF).map(|s| s.start), Some(0x4001_1400));
        // `end` is exclusive: this address is in the gap, not in the first slot.
        assert!(bus.get(0x4001_1800).is_none());
        assert!(bus.get(0x4001_2000).is_none());
        assert_eq!(bus.get(0x4001_2400).map(|s| s.start), Some(0x4001_2400));
        assert!(bus.get(0x3FFF_FFFF).is_none());
    }

    #[test]
    fn adjacent_slots_do_not_bleed() {
        let bus = bus_with(&[(0x4000_0000, 0x4000_0400), (0x4000_0400, 0x4000_0800)]);
        assert_eq!(bus.get(0x4000_03FF).map(|s| s.start), Some(0x4000_0000));
        assert_eq!(bus.get(0x4000_0400).map(|s| s.start), Some(0x4000_0400));
    }

    #[test]
    fn last_registration_wins_on_overlap() {
        let mut bus = bus_with(&[(0x4000_0000, 0x4000_0400)]);
        bus.register(0x4000_0200, 0x4000_0600, false, Box::new(Dummy));
        assert_eq!(bus.len(), 1);
        assert_eq!(bus.get(0x4000_0300).map(|s| s.start), Some(0x4000_0200));
        assert!(bus.get(0x4000_0100).is_none());
    }
}

/// A peripheral implemented entirely in JS (read/write callbacks, called with
/// the absolute address and access width) — Wokwi-style custom chips.
pub struct JsPeripheral {
    base: u32,
    read: js_sys::Function,
    write: js_sys::Function,
}

impl JsPeripheral {
    pub fn new(base: u32, read: js_sys::Function, write: js_sys::Function) -> Self {
        JsPeripheral { base, read, write }
    }

    fn call_read(&self, offset: u32, size: u8) -> u32 {
        let addr = self.base + offset;
        let v = self.read
            .call2(&JsValue::NULL, &JsValue::from(addr), &JsValue::from(size))
            .unwrap_or_else(|_| JsValue::from(0));
        v.as_f64().unwrap_or(0.0) as u32
    }

    fn call_write(&self, offset: u32, value: u32, size: u8) {
        let addr = self.base + offset;
        let _ = self.write
            .call3(&JsValue::NULL, &JsValue::from(addr), &JsValue::from(value), &JsValue::from(size));
    }
}

impl Peripheral for JsPeripheral {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        self.call_read(offset, 4)
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        self.call_write(offset, value, 4);
    }

    fn read_sized(&mut self, _sys: &System, offset: u32, size: u8) -> u32 {
        self.call_read(offset, size)
    }

    fn write_sized(&mut self, _sys: &System, offset: u32, size: u8, value: u32) {
        self.call_write(offset, value, size);
    }
}
