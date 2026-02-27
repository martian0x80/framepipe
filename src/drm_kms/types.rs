use std::os::fd::OwnedFd;
use std::os::unix::io::{AsFd, BorrowedFd};
use std::fs::{File, OpenOptions};
use std::path::PathBuf;
use drm::control::Device as ControlDevice;
use drm::Device as BasicDevice;

#[derive(Debug)]
pub(crate) struct Card(File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl BasicDevice for Card {}
impl ControlDevice for Card {}

impl Card {
    pub fn open(path: &str) -> std::io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)?;
        Ok(Card(file))
    }
}

#[derive(Debug)]
pub struct ProbeResult {
    pub fb_id: u32,
    pub fb_info: drm::control::framebuffer::PlanarInfo,
    pub plane_fds: Vec<Option<OwnedFd>>, // index matches fb_info planes
}

/// Represents a framebuffer exported as DMABuf fds, ready for encoding or preview. (not prime fds from drm api)
pub struct ExportedDmabuf {
    pub width: i32,
    pub height: i32,
    pub fourcc: u32,
    pub modifier: u64,
    pub fds: Vec<OwnedFd>,
    pub strides: Vec<i32>,
    pub offsets: Vec<i32>,
    pub acquire_fence_fd: Option<OwnedFd>,
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum FrameRateMode {
    Cfr,
    Vfr,
}

impl ToString for FrameRateMode {
    fn to_string(&self) -> String {
        match self {
            FrameRateMode::Cfr => "cfr".to_string(),
            FrameRateMode::Vfr => "vfr".to_string(),
        }
    }
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum BitrateMode {
    Cbr,
    Vbr,
}

impl ToString for BitrateMode {
    fn to_string(&self) -> String {
        match self {
            BitrateMode::Cbr => "cbr".to_string(),
            BitrateMode::Vbr => "vbr".to_string(),
        }
    }
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum ColorRange {
    Full,
    Limited,
}

impl ToString for ColorRange {
    fn to_string(&self) -> String {
        match self {
            ColorRange::Full => "full".to_string(),
            ColorRange::Limited => "limited".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum CaptureOutput {
    File(PathBuf),
    Preview,
}

#[derive(Debug, Clone)]
pub struct CaptureOptions {
    pub card_path: String,
    pub connector: Option<String>,
    pub allow_fallback_connector: bool,
    pub fps: u32,
    pub dump_frames: bool,
    pub dump_dir: PathBuf,
    pub dump_every: u32,
    pub output: CaptureOutput,
    pub bitrate_kbps: u32,
    pub frame_rate_mode: FrameRateMode,
    pub bitrate_mode: BitrateMode,
    pub color_range: ColorRange,
}
