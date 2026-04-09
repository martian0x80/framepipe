use std::{
    cell::UnsafeCell,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::capture::types::CaptureFrame;

pub struct PipeWireFrameRing {
    buffer: Box<[UnsafeCell<Option<CaptureFrame>>]>,
    capacity: u64,
    write_idx: AtomicU64,
    read_idx: AtomicU64,
}

unsafe impl Send for PipeWireFrameRing {}
unsafe impl Sync for PipeWireFrameRing {}

impl PipeWireFrameRing {
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.next_power_of_two().max(2);
        let mut buf = Vec::with_capacity(cap);
        for _ in 0..cap {
            buf.push(UnsafeCell::new(None));
        }
        Self {
            buffer: buf.into_boxed_slice(),
            capacity: cap as u64,
            write_idx: AtomicU64::new(0),
            read_idx: AtomicU64::new(0),
        }
    }

    pub fn push_overwrite(&self, frame: CaptureFrame) {
        let write = self.write_idx.load(Ordering::Relaxed);
        let read = self.read_idx.load(Ordering::Acquire);
        if write.saturating_sub(read) >= self.capacity {
            self.read_idx.store(read.saturating_add(1), Ordering::Release);
        }
        let slot = write & (self.capacity - 1);
        unsafe {
            *self.buffer[slot as usize].get() = Some(frame);
        }
        self.write_idx.store(write.saturating_add(1), Ordering::Release);
    }

    pub fn pop_latest(&self) -> Option<CaptureFrame> {
        let write = self.write_idx.load(Ordering::Acquire);
        let read = self.read_idx.load(Ordering::Acquire);
        if write == read {
            return None;
        }
        let latest = write.saturating_sub(1);
        self.read_idx.store(write, Ordering::Release);
        let slot = latest & (self.capacity - 1);
        unsafe { (*self.buffer[slot as usize].get()).take() }
    }
}
