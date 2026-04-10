use clap::ValueEnum;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CaptureBackendKind {
    #[value(name = "kms")]
    DrmKms,
    #[value(name = "portal")]
    PipewirePortal,
}

impl Default for CaptureBackendKind {
    fn default() -> Self {
        Self::DrmKms
    }
}

impl std::fmt::Display for CaptureBackendKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DrmKms => write!(f, "kms"),
            Self::PipewirePortal => write!(f, "portal"),
        }
    }
}

pub struct CaptureFrame {
    pub fb_id: u32,
    pub width: i32,
    pub height: i32,
    pub fourcc: u32,
    pub modifier: Option<u64>,
    pub use_external_texture: bool,
    pub plane_fds: Vec<std::os::fd::OwnedFd>,
    pub offsets: Vec<u32>,
    pub strides: Vec<u32>,
}