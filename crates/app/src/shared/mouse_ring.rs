use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Copy)]
pub struct MouseEvent {
    pub t_ns: u64,
    pub x: f64,
    pub y: f64,
    pub max_x: Option<f64>,
    pub max_y: Option<f64>,
}

pub struct RingBuffer {
    buffer: Box<[std::cell::UnsafeCell<Option<MouseEvent>>]>,
    capacity: u64,
    write_idx: AtomicU64,
    read_idx: AtomicU64,
}

unsafe impl Send for RingBuffer {}
unsafe impl Sync for RingBuffer {}

impl RingBuffer {
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.next_power_of_two();
        let mut buf = Vec::with_capacity(cap);
        for _ in 0..cap {
            buf.push(std::cell::UnsafeCell::new(None));
        }
        Self {
            buffer: buf.into_boxed_slice(),
            capacity: cap as u64,
            write_idx: AtomicU64::new(0),
            read_idx: AtomicU64::new(0),
        }
    }

    pub fn push(&self, event: MouseEvent) {
        let idx = self.write_idx.fetch_add(1, Ordering::Release);
        let slot = idx & (self.capacity - 1);
        unsafe {
            *self.buffer[slot as usize].get() = Some(event);
        }
    }

    pub fn latest_before(&self, pts_ns: u64) -> Option<MouseEvent> {
        let write = self.write_idx.load(Ordering::Acquire);
        if write == 0 {
            return None;
        }

        let mut best: Option<MouseEvent> = None;
        let start = write.saturating_sub(self.capacity);

        for i in start..write {
            let slot = i & (self.capacity - 1);
            let event = unsafe { *self.buffer[slot as usize].get() };
            if let Some(e) = event {
                if e.t_ns <= pts_ns {
                    best = Some(e);
                } else {
                    break;
                }
            }
        }
        best
    }

    pub fn clear(&self) {
        let _write = self.write_idx.load(Ordering::Acquire);
        for i in 0..self.capacity {
            let slot = i & (self.capacity - 1);
            unsafe {
                *self.buffer[slot as usize].get() = None;
            }
        }
        self.write_idx.store(0, Ordering::Release);
    }
}
