use sdl2::audio::AudioCallback;
use std::sync::{Arc, Mutex};

pub const DEFAULT_RING_BUFFER_SAMPLES: usize = 2048;

pub struct RingBuffer {
    data: Vec<f32>,
    head: usize,
    len: usize,
}

impl RingBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            data: vec![0.0; capacity],
            head: 0,
            len: 0,
        }
    }

    pub fn capacity(&self) -> usize {
        self.data.len()
    }

    pub fn clear(&mut self) {
        self.head = 0;
        self.len = 0;
    }

    pub fn push(&mut self, sample: f32) {
        let cap = self.data.len();
        if cap == 0 {
            return;
        }
        if self.len == cap {
            self.head = (self.head + 1) % cap;
            self.len -= 1;
        }
        let index = (self.head + self.len) % cap;
        self.data[index] = sample;
        self.len += 1;
    }

    pub fn push_slice(&mut self, samples: &[f32]) {
        for &sample in samples {
            self.push(sample);
        }
    }

    pub fn pop(&mut self) -> Option<f32> {
        let cap = self.data.len();
        if cap == 0 || self.len == 0 {
            return None;
        }
        let sample = self.data[self.head];
        self.head = (self.head + 1) % cap;
        self.len -= 1;
        Some(sample)
    }

    pub fn read_into(&mut self, out: &mut [f32]) {
        for sample in out.iter_mut() {
            *sample = self.pop().unwrap_or(0.0);
        }
    }
}

pub type SharedRingBuffer = Arc<Mutex<RingBuffer>>;

pub fn new_shared_ring_buffer(capacity: usize) -> SharedRingBuffer {
    Arc::new(Mutex::new(RingBuffer::new(capacity)))
}

pub struct RingBufferAudio {
    pub buffer: SharedRingBuffer,
}

impl AudioCallback for RingBufferAudio {
    type Channel = f32;

    fn callback(&mut self, out: &mut [f32]) {
        if let Ok(mut buffer) = self.buffer.try_lock() {
            buffer.read_into(out);
        } else {
            for sample in out.iter_mut() {
                *sample = 0.0;
            }
        }
    }
}
