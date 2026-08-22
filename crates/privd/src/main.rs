use std::fs::{self, File, OpenOptions};
use std::mem::size_of;
use std::num::NonZeroU32;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use drm::CLOEXEC;
use drm::ClientCapability::{Atomic, UniversalPlanes};
use drm::Device as BasicDevice;
use drm::control::Device as ControlDevice;
use drm::control::GetPlanarFramebufferError;
use drm::control::framebuffer;
use thiserror::Error;

use common::ipc::{recv_packet, send_packet};
use common::types::{
    ExportedFrameInfo, InputDeviceFailure, InputDeviceInfo, IpcRequest, IpcResponse,
    PRIVD_PROTOCOL_VERSION, PrivdErrorKind,
};

#[derive(Debug, Error)]
enum PrivdError {
    #[error("exactly one of --ipc-fd FD or --connect PATH is required")]
    InvalidStartup,
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
    EnumerateInput(#[source] std::io::Error),
    #[error("session not initialized")]
    SessionNotInitialized,
    #[error("framebuffer export failed for fb {fb_id}: {message}")]
    ExportFramebuffer { fb_id: u32, message: String },
    #[error("framebuffer {fb_id} has no exportable plane fds")]
    FramebufferHandlesRedacted { fb_id: u32 },
    #[error("privileged device operation failed: {0}")]
    DeviceOperation(#[source] std::io::Error),
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
    card: Option<(String, Card)>,
    held_input_fds: Vec<OwnedFd>,
}

type OpenedInput = (Vec<OwnedFd>, Vec<InputDeviceInfo>, Vec<InputDeviceFailure>);

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
    let stream = match parse_startup()? {
        Startup::IpcFd(fd) => {
            log::info!("privd start: ipc-fd={fd}");
            unsafe { UnixStream::from_raw_fd(fd) }
        }
        Startup::Connect(path) => connect_elevated(&path)?,
    };

    let mut state = SessionState {
        card: None,
        held_input_fds: Vec::new(),
    };
    let mut hello_complete = false;
    loop {
        let (req, _fds): (IpcRequest, Vec<OwnedFd>) =
            recv_packet(&stream).map_err(|e| PrivdError::IpcRecv(e.to_string()))?;
        log::trace!("privd request: {:?}", req);
        match req {
            IpcRequest::Hello { protocol_version } => {
                if protocol_version != PRIVD_PROTOCOL_VERSION {
                    send_error(
                        &stream,
                        PrivdErrorKind::Unsupported,
                        format!(
                            "unsupported protocol version {protocol_version}; expected {PRIVD_PROTOCOL_VERSION}"
                        ),
                    )?;
                    continue;
                }
                hello_complete = true;
                send_packet(
                    &stream,
                    &IpcResponse::Hello {
                        protocol_version: PRIVD_PROTOCOL_VERSION,
                    },
                    &[],
                )
                .map_err(|e| PrivdError::IpcSend(e.to_string()))?;
            }
            _ if !hello_complete => {
                send_error(
                    &stream,
                    PrivdErrorKind::Protocol,
                    "protocol handshake required".into(),
                )?;
            }
            IpcRequest::OpenDrm { card_path } => match open_drm(&card_path) {
                Ok(card) => {
                    send_packet(
                        &stream,
                        &IpcResponse::DrmOpened {
                            card_path: card_path.clone(),
                        },
                        &[card.0.as_raw_fd()],
                    )
                    .map_err(|e| PrivdError::IpcSend(e.to_string()))?;
                    state.card = Some((card_path, card));
                }
                Err(e) => send_error(&stream, error_kind(&e), e.to_string())?,
            },
            IpcRequest::OpenInput => {
                let (fds, infos, failures) = match open_input() {
                    Ok(opened) => opened,
                    Err(error) => {
                        send_error(&stream, error_kind(&error), error.to_string())?;
                        continue;
                    }
                };
                let raw: Vec<RawFd> = fds.iter().map(AsRawFd::as_raw_fd).collect();
                send_packet(
                    &stream,
                    &IpcResponse::InputOpened {
                        input_devices: infos,
                        failures,
                    },
                    &raw,
                )
                .map_err(|e| PrivdError::IpcSend(e.to_string()))?;
                state.held_input_fds = fds;
            }
            IpcRequest::ExportFramebuffer { fb_id } => {
                let Some((_, card)) = state.card.as_ref() else {
                    let _ = send_error(
                        &stream,
                        PrivdErrorKind::Protocol,
                        PrivdError::SessionNotInitialized.to_string(),
                    );
                    continue;
                };
                match export_framebuffer(card, fb_id) {
                    Ok((frame, fds)) => {
                        let raw: Vec<RawFd> = fds.iter().map(|fd| fd.as_raw_fd()).collect();
                        let resp = IpcResponse::FrameExported { frame };
                        send_packet(&stream, &resp, &raw)
                            .map_err(|e| PrivdError::IpcSend(e.to_string()))?;
                    }
                    Err(e) => {
                        let _ = send_error(&stream, error_kind(&e), e.to_string());
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

fn send_error(stream: &UnixStream, kind: PrivdErrorKind, msg: String) -> Result<(), PrivdError> {
    let resp = IpcResponse::Error { kind, message: msg };
    send_packet(stream, &resp, &[]).map_err(|e| PrivdError::IpcSend(e.to_string()))
}

fn open_drm(card_path: &str) -> Result<Card, PrivdError> {
    validate_device_path(Path::new(card_path), Path::new("/dev/dri"), "card").map_err(
        |source| PrivdError::OpenDrm {
            path: card_path.into(),
            source,
        },
    )?;
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
        .map_err(PrivdError::DeviceOperation)?;
    card.set_client_capability(UniversalPlanes, true)
        .map_err(PrivdError::DeviceOperation)?;

    Ok(card)
}

fn open_input() -> Result<OpenedInput, PrivdError> {
    let mut held_input_fds = Vec::new();
    let mut input_infos = Vec::new();
    let mut failures = Vec::new();
    for path in enumerate_input_event_nodes()? {
        let opened = validate_device_path(&path, Path::new("/dev/input"), "event").and_then(|_| {
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .or_else(|_| OpenOptions::new().read(true).open(&path))
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
                failures.push(InputDeviceFailure {
                    path: path.to_string_lossy().into_owned(),
                    kind: io_error_kind(&e),
                    message: e.to_string(),
                });
            }
        }
    }
    Ok((held_input_fds, input_infos, failures))
}

fn export_framebuffer(
    card: &Card,
    fb_id: u32,
) -> Result<(ExportedFrameInfo, Vec<OwnedFd>), PrivdError> {
    let fb =
        framebuffer::Handle::from(NonZeroU32::new(fb_id).ok_or(PrivdError::ExportFramebuffer {
            fb_id,
            message: "invalid fb id 0".to_string(),
        })?);
    let fb_info = card
        .get_planar_framebuffer(fb)
        .map_err(|error| match error {
            GetPlanarFramebufferError::Io(error) => PrivdError::DeviceOperation(error),
            other => PrivdError::ExportFramebuffer {
                fb_id,
                message: other.to_string(),
            },
        })?;

    let mut plane_fds = Vec::new();
    let mut strides = Vec::new();
    let mut offsets = Vec::new();
    for (index, buf) in fb_info.buffers().iter().enumerate() {
        let Some(handle) = buf else {
            continue;
        };
        let fd = card
            .buffer_to_prime_fd(*handle, CLOEXEC)
            .map_err(PrivdError::DeviceOperation)?;
        plane_fds.push(fd);
        strides.push(fb_info.pitches()[index] as i32);
        offsets.push(fb_info.offsets()[index] as i32);
    }

    if plane_fds.is_empty() {
        return Err(PrivdError::FramebufferHandlesRedacted { fb_id });
    }

    let frame = ExportedFrameInfo {
        fb_id,
        width: fb_info.size().0 as i32,
        height: fb_info.size().1 as i32,
        fourcc: fb_info.pixel_format() as u32,
        modifier: fb_info.modifier().map(|m| m.into()),
        strides,
        offsets,
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
    let entries = fs::read_dir("/dev/input").map_err(PrivdError::EnumerateInput)?;
    for ent in entries {
        let ent = ent.map_err(PrivdError::EnumerateInput)?;
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

enum Startup {
    IpcFd(RawFd),
    Connect(PathBuf),
}

fn parse_startup() -> Result<Startup, PrivdError> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [flag, value] if flag == "--ipc-fd" => value
            .parse()
            .map(Startup::IpcFd)
            .map_err(|_| PrivdError::InvalidStartup),
        [flag, value] if flag == "--connect" => Ok(Startup::Connect(value.into())),
        _ => Err(PrivdError::InvalidStartup),
    }
}

fn connect_elevated(path: &Path) -> Result<UnixStream, PrivdError> {
    let uid: u32 = std::env::var("PKEXEC_UID")
        .ok()
        .and_then(|value| value.parse().ok())
        .ok_or(PrivdError::InvalidStartup)?;
    validate_connect_path(path, uid).map_err(|source| PrivdError::OpenDrm {
        path: path.display().to_string(),
        source,
    })?;

    let stream = connect_seqpacket(path).map_err(|source| PrivdError::OpenDrm {
        path: path.display().to_string(),
        source,
    })?;
    if unsafe { libc::setgroups(0, std::ptr::null()) } != 0 {
        log::warn!(
            "failed to clear supplementary groups: {}",
            std::io::Error::last_os_error()
        );
    }
    for key in std::env::vars_os().map(|(key, _)| key).collect::<Vec<_>>() {
        unsafe { std::env::remove_var(key) };
    }
    Ok(stream)
}

fn connect_seqpacket(path: &Path) -> std::io::Result<UnixStream> {
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let bytes = path.as_os_str().as_encoded_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= address.sun_path.len() {
        unsafe { libc::close(fd) };
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "socket path too long",
        ));
    }
    address.sun_family = libc::AF_UNIX as _;
    for (dest, source) in address.sun_path.iter_mut().zip(bytes) {
        *dest = *source as _;
    }
    let length = size_of::<libc::sa_family_t>() + bytes.len() + 1;
    if unsafe {
        libc::connect(
            fd,
            &address as *const _ as *const libc::sockaddr,
            length as _,
        )
    } != 0
    {
        let error = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(error);
    }
    Ok(unsafe { UnixStream::from_raw_fd(fd) })
}

fn validate_connect_path(path: &Path, uid: u32) -> std::io::Result<()> {
    let expected_dir = PathBuf::from(format!("/run/user/{uid}/framepipe"));
    if path.parent() != Some(expected_dir.as_path()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "socket is outside the caller runtime directory",
        ));
    }
    let dir_meta = fs::symlink_metadata(&expected_dir)?;
    let socket_meta = fs::symlink_metadata(path)?;
    if dir_meta.file_type().is_symlink()
        || !dir_meta.is_dir()
        || dir_meta.uid() != uid
        || dir_meta.permissions().mode() & 0o777 != 0o700
        || socket_meta.file_type().is_symlink()
        || !socket_meta.file_type().is_socket()
        || socket_meta.uid() != uid
        || socket_meta.permissions().mode() & 0o777 != 0o600
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "unsafe rendezvous socket ownership or type",
        ));
    }
    Ok(())
}

fn validate_device_path(path: &Path, parent: &Path, prefix: &str) -> std::io::Result<()> {
    if path.parent() != Some(parent)
        || !path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(prefix))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "device path is not allowed",
        ));
    }
    let meta = fs::symlink_metadata(path)?;
    let canonical = fs::canonicalize(path)?;
    if meta.file_type().is_symlink()
        || canonical != path
        || canonical.parent() != Some(parent)
        || !canonical
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with(prefix))
        || !meta.file_type().is_char_device()
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "device path is not allowed",
        ));
    }
    Ok(())
}

fn io_error_kind(error: &std::io::Error) -> PrivdErrorKind {
    match error.raw_os_error() {
        Some(code) if code == libc::EACCES || code == libc::EPERM => {
            PrivdErrorKind::PermissionDenied
        }
        Some(code) if code == libc::ENOENT || code == libc::ENODEV => {
            PrivdErrorKind::DeviceUnavailable
        }
        _ => PrivdErrorKind::Internal,
    }
}

fn error_kind(error: &PrivdError) -> PrivdErrorKind {
    match error {
        PrivdError::OpenDrm { source, .. } => io_error_kind(source),
        PrivdError::EnumerateInput(source) => io_error_kind(source),
        PrivdError::DeviceOperation(source) => io_error_kind(source),
        PrivdError::SessionNotInitialized => PrivdErrorKind::Protocol,
        PrivdError::FramebufferHandlesRedacted { .. } => PrivdErrorKind::PermissionDenied,
        PrivdError::ExportFramebuffer { .. } => PrivdErrorKind::DeviceUnavailable,
        _ => PrivdErrorKind::Internal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_arbitrary_device_paths() {
        let error = validate_device_path(Path::new("/etc/passwd"), Path::new("/dev/dri"), "card")
            .unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn classifies_permission_errors() {
        assert_eq!(
            io_error_kind(&std::io::Error::from_raw_os_error(libc::EPERM)),
            PrivdErrorKind::PermissionDenied
        );
        assert_eq!(
            error_kind(&PrivdError::FramebufferHandlesRedacted { fb_id: 1 }),
            PrivdErrorKind::PermissionDenied
        );
    }
}
