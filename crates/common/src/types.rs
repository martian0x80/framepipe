use arc_swap::ArcSwapOption;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const PRIVD_PROTOCOL_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrivdErrorKind {
    PermissionDenied,
    AuthenticationCancelled,
    PolkitUnavailable,
    DeviceUnavailable,
    Unsupported,
    Protocol,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcRequest {
    Hello { protocol_version: u32 },
    OpenDrm { card_path: String },
    OpenInput,
    ExportFramebuffer { fb_id: u32 },
    ExportCursor { crtc_id: u32 },
    Stop,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputDeviceInfo {
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputDeviceFailure {
    pub path: String,
    pub kind: PrivdErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedFrameInfo {
    pub fb_id: u32,
    pub width: i32,
    pub height: i32,
    pub fourcc: u32,
    pub modifier: Option<u64>,
    pub strides: Vec<i32>,
    pub offsets: Vec<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedCursorInfo {
    pub frame: ExportedFrameInfo,
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcResponse {
    Hello {
        protocol_version: u32,
    },
    DrmOpened {
        card_path: String,
    },
    InputOpened {
        input_devices: Vec<InputDeviceInfo>,
        failures: Vec<InputDeviceFailure>,
    },
    FrameExported {
        frame: ExportedFrameInfo,
    },
    CursorExported {
        cursor: Option<ExportedCursorInfo>,
    },
    Error {
        kind: PrivdErrorKind,
        message: String,
    },
}

#[derive(Debug, Clone)]
pub struct PreviewFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
    pub t_ns: u64,
}

impl PreviewFrame {
    pub fn new(width: u32, height: u32, rgba: Arc<[u8]>, t_ns: u64) -> Self {
        Self {
            width,
            height,
            rgba,
            t_ns,
        }
    }

    pub fn empty() -> Self {
        Self {
            width: 0,
            height: 0,
            rgba: Arc::new([]),
            t_ns: 0,
        }
    }
}

#[derive(Debug)]
pub struct PreviewMailbox {
    pub frame: Arc<ArcSwapOption<PreviewFrame>>,
}

impl Default for PreviewMailbox {
    fn default() -> Self {
        Self::new()
    }
}

impl PreviewMailbox {
    pub fn new() -> Self {
        Self {
            frame: Arc::new(ArcSwapOption::new(None)),
        }
    }

    pub fn update_frame(&self, frame: PreviewFrame) {
        self.frame.store(Some(Arc::new(frame)));
    }

    pub fn get_frame(&self) -> Option<Arc<PreviewFrame>> {
        self.frame.load_full()
    }
}

impl Clone for PreviewMailbox {
    fn clone(&self) -> Self {
        Self {
            frame: Arc::clone(&self.frame),
        }
    }
}
