use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

use bitcode::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::shared::mouse_ring::RingBuffer;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MouseSample {
    pub x: f64,
    pub y: f64,
    pub anchored: bool,
    #[serde(with = "serde_millis")]
    pub timestamp: Instant,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct MouseSampleRecord {
    pub t_ns: u64,
    pub x: f64,
    pub y: f64,
    pub anchored: bool,
}

#[derive(Debug, Clone, Encode, Decode, Default)]
pub struct MouseTrackRecordingInfo {
    pub started_unix_ms: u64,
    pub output_path: Option<String>,
    pub card_path: Option<String>,
    pub connector: Option<String>,
    pub fps: Option<u32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub encoder_backend: Option<String>,
    pub video_codec: Option<String>,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct MouseTrackFile {
    pub version: u32,
    pub recording: MouseTrackRecordingInfo,
    pub samples: Vec<MouseSampleRecord>,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct MouseTrackHeader {
    pub version: u32,
    pub recording: MouseTrackRecordingInfo,
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct MouseTrackChunk {
    pub samples: Vec<MouseSampleRecord>,
}

#[derive(Default)]
pub(crate) struct MouseState {
    pub anchored: bool,
    pub x: f64,
    pub y: f64,
    pub max_x: Option<f64>,
    pub max_y: Option<f64>,
    pub history: VecDeque<(Instant, f64, f64)>,
    pub anchor_epoch: u64,
}

pub trait MouseTracker {
    fn start<T: AsRef<std::path::Path>>(
        file_path: T,
        recording: MouseTrackRecordingInfo,
        ring: Option<Arc<RingBuffer>>,
    ) -> Result<Self, String>
    where
        Self: Sized;
    fn set_paused(&self, paused: bool);
    fn set_bounds(&self, width: f64, height: f64);
    fn sample(&self) -> MouseSample;
}
