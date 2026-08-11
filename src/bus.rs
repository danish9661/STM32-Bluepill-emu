//! Memory-mapped peripheral bus (rp2040js-style registry).
//!
//! Peripherals are registered with a `[start, end)` address range; accesses
//! are routed with a binary search over the (sorted) slot list. New
//! peripherals can be added at runtime from Rust (`Bus::register`) or from
//! JS via `register_js_peripheral` (wasm export) — a `JsPeripheral` wrapping
//! JS read/write callbacks.

use std::cell::RefCell;

use wasm_bindgen::prelude::*;

use crate::peripherals::{Peripheral, PeripheralSlot};
use crate::system::System;

/// Sorted registry of peripheral slots with tick bookkeeping.
#[derive(Default)]
pub struct Bus {
    slots: Vec<PeripheralSlot<RefCell<Box<dyn Peripheral>>>>,
    tick_indices: Vec<usize>,
}

impl Bus {
    pub fn new() -> Self {
        Bus { slots: Vec::new(), tick_indices: Vec::new() }
    }

    /// Register a peripheral covering `[start, end)`. Re-registering an
    /// overlapping range replaces the old slot (rp2040js semantics: last
    /// registration wins), so custom peripherals can shadow built-ins.
    pub fn register(&mut self, start: u32, end: u32, tick: bool, p: Box<dyn Peripheral>) {
        if start >= end {
            panic!("Bus::register: empty range 0x{:08x}-0x{:08x}", start, end);
        }
        let slot = PeripheralSlot { start, end, tick, peripheral: RefCell::new(p) };
        self.slots.retain(|s| s.end <= start || s.start >= end);
        self.slots.push(slot);
        self.slots.sort_by_key(|s| s.start);
        self.rebuild_tick_indices();
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

    /// Binary search for the slot covering `addr`.
    pub fn get(&self, addr: u32) -> Option<&PeripheralSlot<RefCell<Box<dyn Peripheral>>>> {
        let index = self.slots.binary_search_by_key(&addr, |p| p.start)
            .map_or_else(|e| e.checked_sub(1), |v| Some(v));
        index.map(|i| self.slots.get(i).filter(|p| addr <= p.end)).flatten()
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
