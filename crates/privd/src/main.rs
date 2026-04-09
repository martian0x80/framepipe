use std::fs::{self, File, OpenOptions};
use std::num::NonZeroU32;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use drm::CLOEXEC;
use drm::ClientCapability::{Atomic, UniversalPlanes};
use drm::Device as BasicDevice;
use drm::control::Device as ControlDevice;
use drm::control::framebuffer;
use thiserror::Error;

use common::ipc::{recv_packet, send_packet};
use common::types::{ExportedFrameInfo, InputDeviceInfo, IpcRequest, IpcResponse};

#[derive(Debug, Error)]
enum PrivdError {
    #[error("missing --ipc-fd argument")]
    MissingIpcFd,
    #[error("ipc receive failed: {0}")]
    IpcRecv(String),
    #[error("ipc send failed: {0}")]
    IpcSend(String),
    #[error("failed to open drm device {path}: {source}")]
    OpenDrm {
        path: String,
        source: std::io::Error,
    },
    #[error("failed to enumerate input nodes: {0}")]
    EnumerateInput(String),
    #[error("session not initialized")]
    SessionNotInitialized,
    #[error("framebuffer export failed for fb {fb_id}: {message}")]
    ExportFramebuffer { fb_id: u32, message: String },
}

#[derive(Debug)]
struct Card(File);

impl AsFd for Card {
    fn as_fd(&self) -> std::os::fd::BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl BasicDevice for Card {}
impl ControlDevice for Card {}

struct SessionState {
    card_path: String,
    card: Card,
    _held_input_fds: Vec<OwnedFd>,
    input_infos: Vec<InputDeviceInfo>,
}

fn main() {
    // Keep privd alive on Ctrl+C from terminal
    unsafe {
        libc::signal(libc::SIGINT, libc::SIG_IGN);
        libc::signal(libc::SIGTERM, libc::SIG_IGN);
    }

    env_logger::builder()
        .format_timestamp_nanos()
        .filter_level(log::LevelFilter::Debug)
        .init();

    if let Err(e) = run() {
        log::error!("privd failed: {e}");
        std::process::exit(1);
    }
}

fn run() -> eyre::Result<()> {
    let fd = parse_ipc_fd().ok_or(PrivdError::MissingIpcFd)?;
    log::info!("privd start: ipc-fd={}", fd);
    let stream = unsafe { UnixStream::from_raw_fd(fd) };

    let mut state: Option<SessionState> = None;
    loop {
        let (req, _fds): (IpcRequest, Vec<OwnedFd>) =
            recv_packet(&stream).map_err(|e| PrivdError::IpcRecv(e.to_string()))?;
        log::trace!("privd request: {:?}", req);
        match req {
            IpcRequest::StartSession {
                card_path,
                include_input_fds,
            } => match start_session(&card_path, include_input_fds) {
                Ok(s) => {
                    let resp = IpcResponse::SessionReady {
                        card_path: s.card_path.clone(),
                        input_devices: s.input_infos.clone(),
                    };
                    let mut raw_fds = Vec::new();
                    raw_fds.push(s.card.0.as_raw_fd());
                    for fd in &s._held_input_fds {
                        raw_fds.push(fd.as_raw_fd());
                    }
                    send_packet(&stream, &resp, &raw_fds)
                        .map_err(|e| PrivdError::IpcSend(e.to_string()))?;
                    log::info!(
                        "session ready sent: drm=1 inputs={} total_fds={}",
                        s.input_infos.len(),
                        raw_fds.len()
                    );
                    state = Some(s);
                }
                Err(e) => {
                    let _ = send_error(&stream, e.to_string());
                }
            },
            IpcRequest::ExportFramebuffer { fb_id } => {
                let Some(session) = state.as_ref() else {
                    let _ = send_error(&stream, PrivdError::SessionNotInitialized.to_string());
                    continue;
                };
                match export_framebuffer(session, fb_id) {
                    Ok((frame, fds)) => {
                        let raw: Vec<RawFd> = fds.iter().map(|fd| fd.as_raw_fd()).collect();
                        let resp = IpcResponse::FrameExported { frame };
                        send_packet(&stream, &resp, &raw)
                            .map_err(|e| PrivdError::IpcSend(e.to_string()))?;
                    }
                    Err(e) => {
                        let _ = send_error(&stream, e.to_string());
                    }
                }
            }
            IpcRequest::Stop => {
                log::info!("privd stop request received");
                break;
            }
        }
    }
    Ok(())
}

fn send_error(stream: &UnixStream, msg: String) -> Result<(), PrivdError> {
    let resp = IpcResponse::Error { message: msg };
    send_packet(stream, &resp, &[]).map_err(|e| PrivdError::IpcSend(e.to_string()))
}

fn start_session(card_path: &str, include_input_fds: bool) -> Result<SessionState, PrivdError> {
    log::info!(
        "starting session for card {}, include_input_fds={}",
        card_path,
        include_input_fds
    );
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(card_path)
        .map_err(|source| PrivdError::OpenDrm {
            path: card_path.to_string(),
            source,
        })?;
    let card = Card(file);

    card.set_client_capability(Atomic, true)
        .map_err(|e| PrivdError::ExportFramebuffer {
            fb_id: 0,
            message: format!("set Atomic capability failed: {e}"),
        })?;
    card.set_client_capability(UniversalPlanes, true)
        .map_err(|e| PrivdError::ExportFramebuffer {
            fb_id: 0,
            message: format!("set UniversalPlanes capability failed: {e}"),
        })?;

    let mut held_input_fds = Vec::new();
    let mut input_infos = Vec::new();
    if include_input_fds {
        for path in enumerate_input_event_nodes()? {
            let opened = OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .or_else(|rw_err| {
                    log::debug!(
                        "rw open failed for {} ({}), retrying read-only",
                        path.to_string_lossy(),
                        rw_err
                    );
                    OpenOptions::new().read(true).open(&path)
                });
            match opened {
                Ok(file) => {
                    log::debug!("opened input node: {}", path.to_string_lossy());
                    held_input_fds.push(file.into());
                    input_infos.push(InputDeviceInfo {
                        path: path.to_string_lossy().into_owned(),
                    });
                }
                Err(e) => {
                    log::debug!("skipping input node {}: {e}", path.to_string_lossy());
                }
            }
        }
    }
    log::info!(
        "session fd bundle prepared: drm=1 inputs={} total={}",
        input_infos.len(),
        1 + held_input_fds.len()
    );

    Ok(SessionState {
        card_path: card_path.to_string(),
        card,
        _held_input_fds: held_input_fds,
        input_infos,
    })
}

fn export_framebuffer(
    session: &SessionState,
    fb_id: u32,
) -> Result<(ExportedFrameInfo, Vec<OwnedFd>), PrivdError> {
    let fb =
        framebuffer::Handle::from(NonZeroU32::new(fb_id).ok_or(PrivdError::ExportFramebuffer {
            fb_id,
            message: "invalid fb id 0".to_string(),
        })?);
    let fb_info =
        session
            .card
            .get_planar_framebuffer(fb)
            .map_err(|e| PrivdError::ExportFramebuffer {
                fb_id,
                message: format!("get_planar_framebuffer failed: {e}"),
            })?;

    let mut plane_fds = Vec::new();
    for (i, buf) in fb_info.buffers().iter().enumerate() {
        let Some(handle) = buf else {
            continue;
        };
        let fd = session
            .card
            .buffer_to_prime_fd(*handle, CLOEXEC)
            .map_err(|e| PrivdError::ExportFramebuffer {
                fb_id,
                message: format!("buffer_to_prime_fd failed for plane {}: {}", i, e),
            })?;
        plane_fds.push(fd);
    }

    if plane_fds.is_empty() {
        return Err(PrivdError::ExportFramebuffer {
            fb_id,
            message: "no exportable plane fds".to_string(),
        });
    }

    let frame = ExportedFrameInfo {
        fb_id,
        width: fb_info.size().0 as i32,
        height: fb_info.size().1 as i32,
        fourcc: fb_info.pixel_format() as u32,
        modifier: fb_info.modifier().map(|m| m.into()),
        strides: fb_info.pitches().iter().map(|v| *v as i32).collect(),
        offsets: fb_info.offsets().iter().map(|v| *v as i32).collect(),
    };

    log::trace!(
        "exported fb={} size={}x{} fourcc=0x{:08x} planes={}",
        frame.fb_id,
        frame.width,
        frame.height,
        frame.fourcc,
        plane_fds.len()
    );

    Ok((frame, plane_fds))
}

fn enumerate_input_event_nodes() -> Result<Vec<PathBuf>, PrivdError> {
    let mut out = Vec::new();
    let entries =
        fs::read_dir("/dev/input").map_err(|e| PrivdError::EnumerateInput(e.to_string()))?;
    for ent in entries {
        let ent = ent.map_err(|e| PrivdError::EnumerateInput(e.to_string()))?;
        let path = ent.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with("event") {
            out.push(path);
        }
    }
    out.sort();
    log::debug!("enumerated {} /dev/input/event* nodes", out.len());
    Ok(out)
}

fn parse_ipc_fd() -> Option<RawFd> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--ipc-fd" {
            if let Some(v) = args.next() {
                if let Ok(fd) = v.parse::<RawFd>() {
                    return Some(fd);
                }
            }
            return None;
        }
    }
    None
}
