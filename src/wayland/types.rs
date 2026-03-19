use std::collections::VecDeque;
use std::time::Instant;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct MouseSample {
    pub x: f64,
    pub y: f64,
    pub anchored: bool,
    #[serde(with = "serde_millis")]
    pub timestamp: Instant,
}

#[derive(Default)]
pub(crate) struct MouseState {
    pub anchored: bool,
    pub x: f64,
    pub y: f64,
    pub max_x: Option<f64>,
    pub max_y: Option<f64>,
    pub history: VecDeque<(Instant, f64, f64)>,
}

pub trait MouseTracker {
    fn start<T: AsRef<std::path::Path>>(file_path: T) -> Result<Self, String> where Self: Sized;
    fn set_paused(&self, paused: bool);
    fn set_bounds(&self, width: f64, height: f64);
    fn sample(&self) -> MouseSample;
    fn write_sample<W: std::io::Write>(w: &mut W, sample: &MouseSample) -> Result<(), String>;
}
