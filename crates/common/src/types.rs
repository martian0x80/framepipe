use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IpcRequest {
    StartSession {
        card_path: String,
        include_input_fds: bool,
    },
    ExportFramebuffer {
        fb_id: u32,
    },
    Stop,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputDeviceInfo {
    pub path: String,
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
pub enum IpcResponse {
    SessionReady {
        card_path: String,
        input_devices: Vec<InputDeviceInfo>,
    },
    FrameExported {
        frame: ExportedFrameInfo,
    },
    Error {
        message: String,
    },
}
