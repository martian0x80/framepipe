use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Copy)]
pub struct KeyOverlayEvent {
    pub t_ns: u64,
    pub keycode: u32,
    pub key_name: &'static str,
    pub is_modifier: bool,
    pub pressed: bool,
}

pub struct KeyboardOverlayRingBuffer {
    buffer: Box<[std::cell::UnsafeCell<Option<KeyOverlayEvent>>]>,
    capacity: u64,
    write_idx: AtomicU64,
    #[expect(unused)]
    read_idx: AtomicU64,
}

unsafe impl Send for KeyboardOverlayRingBuffer {}
unsafe impl Sync for KeyboardOverlayRingBuffer {}

impl KeyboardOverlayRingBuffer {
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

    pub fn push(&self, event: KeyOverlayEvent) {
        let idx = self.write_idx.load(Ordering::Relaxed);
        let slot = idx & (self.capacity - 1);
        unsafe {
            *self.buffer[slot as usize].get() = Some(event);
        }
        self.write_idx.store(idx + 1, Ordering::Release);
    }

    pub fn recent(&self, max_count: usize) -> Vec<KeyOverlayEvent> {
        let write = self.write_idx.load(Ordering::Acquire);
        if write == 0 {
            return Vec::new();
        }

        let mut events = Vec::new();
        let start = write.saturating_sub(self.capacity);
        for i in (start..write).rev() {
            let slot = i & (self.capacity - 1);
            let event = unsafe { *self.buffer[slot as usize].get() };
            if let Some(e) = event {
                events.push(e);
                if events.len() >= max_count {
                    break;
                }
            }
        }
        events.reverse();
        events
    }

    pub fn write_index(&self) -> u64 {
        self.write_idx.load(Ordering::Acquire)
    }

    pub fn drain_since(&self, last_idx: u64) -> Vec<KeyOverlayEvent> {
        let write = self.write_idx.load(Ordering::Acquire);
        if write == 0 || last_idx >= write {
            return Vec::new();
        }

        let mut events = Vec::new();
        for i in last_idx..write {
            let slot = i & (self.capacity - 1);
            let event = unsafe { *self.buffer[slot as usize].get() };
            if let Some(e) = event {
                events.push(e);
            }
        }
        events
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
