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
