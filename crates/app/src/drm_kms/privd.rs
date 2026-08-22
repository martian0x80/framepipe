use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::os::unix::{
    fs::{MetadataExt, PermissionsExt},
    net::UnixStream,
};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use std::{collections::HashMap, fs, io, io::Read, mem::size_of};

use common::ipc::{recv_packet, send_packet};
use common::types::{
    ExportedFrameInfo, IpcRequest, IpcResponse, PRIVD_PROTOCOL_VERSION, PrivdErrorKind,
};
use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};

const INSTALLED_PRIVD_PATH: &str = "/usr/lib/framepipe/framepipe-privd";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum PrivilegeMode {
    #[default]
    Auto,
    Direct,
    Polkit,
}

impl std::fmt::Display for PrivilegeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Auto => "auto",
            Self::Direct => "direct",
            Self::Polkit => "polkit",
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PrivdError {
    #[error("failed to spawn privd: {0}")]
    Spawn(#[source] io::Error),
    #[error("ipc error: {0}")]
    Io(#[source] io::Error),
    #[error("privd {kind:?}: {message}")]
    Remote {
        kind: PrivdErrorKind,
        message: String,
    },
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("polkit authentication was cancelled")]
    AuthenticationCancelled,
    #[error("polkit is unavailable: {0}")]
    PolkitUnavailable(String),
    #[error(
        "direct access failed ({direct}); polkit fallback failed ({polkit}). Grant capabilities to {INSTALLED_PRIVD_PATH} or install/enable polkit"
    )]
    AutoFallback {
        direct: Box<PrivdError>,
        polkit: Box<PrivdError>,
    },
}

impl PrivdError {
    fn is_permission_denied(&self) -> bool {
        matches!(
            self,
            Self::Remote {
                kind: PrivdErrorKind::PermissionDenied,
                ..
            }
        )
    }
}

pub struct PrivdSession {
    pub drm_fd: Option<OwnedFd>,
    pub input_fds: HashMap<PathBuf, OwnedFd>,
    stream: UnixStream,
    child: Child,
    fb_cache: HashMap<u32, CachedFrame>,
    mode: PrivilegeMode,
    card_path: Option<String>,
    polkit: bool,
}

impl Drop for PrivdSession {
    fn drop(&mut self) {
        let _ = send_packet(&self.stream, &IpcRequest::Stop, &[]);
        let deadline = Instant::now() + Duration::from_millis(500);
        while self.child.try_wait().ok().flatten().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

pub struct ExportedFrame {
    pub info: ExportedFrameInfo,
    pub fds: Vec<OwnedFd>,
}
struct CachedFrame {
    info: ExportedFrameInfo,
    fds: Vec<OwnedFd>,
}

impl PrivdSession {
    pub fn export_framebuffer(&mut self, fb_id: u32) -> Result<ExportedFrame, PrivdError> {
        match self.export_framebuffer_once(fb_id) {
            Err(error)
                if self.mode == PrivilegeMode::Auto
                    && !self.polkit
                    && error.is_permission_denied() =>
            {
                let card_path = self.card_path.clone();
                let elevated =
                    acquire_device_fds(card_path.as_deref(), false, PrivilegeMode::Polkit)
                        .map_err(|polkit| PrivdError::AutoFallback {
                            direct: Box::new(error),
                            polkit: Box::new(polkit),
                        })?;
                *self = elevated;
                self.mode = PrivilegeMode::Auto;
                self.export_framebuffer_once(fb_id)
            }
            result => result,
        }
    }

    fn export_framebuffer_once(&mut self, fb_id: u32) -> Result<ExportedFrame, PrivdError> {
        if let Some(cached) = self.fb_cache.get(&fb_id) {
            return Ok(ExportedFrame {
                info: cached.info.clone(),
                fds: cached.fds.iter().map(dup_fd).collect::<Result<_, _>>()?,
            });
        }
        let (resp, fds) = request(&self.stream, IpcRequest::ExportFramebuffer { fb_id })?;
        match resp {
            IpcResponse::FrameExported { frame }
                if !fds.is_empty()
                    && frame.strides.len() == fds.len()
                    && frame.offsets.len() == fds.len() =>
            {
                let keep = fds.iter().map(dup_fd).collect::<Result<_, _>>()?;
                self.fb_cache.insert(
                    fb_id,
                    CachedFrame {
                        info: frame.clone(),
                        fds: keep,
                    },
                );
                if self.fb_cache.len() > 8
                    && let Some(victim) = self.fb_cache.keys().copied().find(|id| *id != fb_id)
                {
                    self.fb_cache.remove(&victim);
                }
                Ok(ExportedFrame { info: frame, fds })
            }
            other => Err(PrivdError::Protocol(format!(
                "unexpected framebuffer response: {other:?}"
            ))),
        }
    }

    fn initialize(
        &mut self,
        card_path: Option<&str>,
        include_input: bool,
    ) -> Result<(), PrivdError> {
        let (hello, fds) = request(
            &self.stream,
            IpcRequest::Hello {
                protocol_version: PRIVD_PROTOCOL_VERSION,
            },
        )?;
        if !matches!(
            hello,
            IpcResponse::Hello {
                protocol_version: PRIVD_PROTOCOL_VERSION
            }
        ) || !fds.is_empty()
        {
            return Err(PrivdError::Protocol(format!(
                "unexpected handshake response: {hello:?}"
            )));
        }
        if let Some(card_path) = card_path {
            let (resp, mut fds) = request(
                &self.stream,
                IpcRequest::OpenDrm {
                    card_path: card_path.into(),
                },
            )?;
            match resp {
                IpcResponse::DrmOpened {
                    card_path: returned,
                } if returned == card_path && fds.len() == 1 => self.drm_fd = fds.pop(),
                other => {
                    return Err(PrivdError::Protocol(format!(
                        "invalid DRM response: {other:?}, fds={}",
                        fds.len()
                    )));
                }
            }
        }
        if include_input {
            let (resp, fds) = request(&self.stream, IpcRequest::OpenInput)?;
            match resp {
                IpcResponse::InputOpened {
                    input_devices,
                    failures,
                } if input_devices.len() == fds.len() => {
                    self.input_fds = input_devices
                        .into_iter()
                        .zip(fds)
                        .map(|(info, fd)| (PathBuf::from(info.path), fd))
                        .collect();
                    if self.input_fds.is_empty()
                        && failures
                            .iter()
                            .any(|failure| failure.kind == PrivdErrorKind::PermissionDenied)
                    {
                        return Err(PrivdError::Remote {
                            kind: PrivdErrorKind::PermissionDenied,
                            message: "no requested input devices could be opened".into(),
                        });
                    }
                    for failure in failures {
                        log::debug!(
                            "input device unavailable: {}: {}",
                            failure.path,
                            failure.message
                        );
                    }
                }
                other => {
                    return Err(PrivdError::Protocol(format!(
                        "invalid input response: {other:?}"
                    )));
                }
            }
        }
        Ok(())
    }
}

fn request(
    stream: &UnixStream,
    req: IpcRequest,
) -> Result<(IpcResponse, Vec<OwnedFd>), PrivdError> {
    send_packet(stream, &req, &[]).map_err(PrivdError::Io)?;
    let (resp, fds) = recv_packet(stream).map_err(PrivdError::Io)?;
    match resp {
        IpcResponse::Error { kind, message } => Err(PrivdError::Remote { kind, message }),
        response => Ok((response, fds)),
    }
}

pub fn acquire_device_fds(
    card_path: Option<&str>,
    include_input: bool,
    mode: PrivilegeMode,
) -> Result<PrivdSession, PrivdError> {
    let acquire = |polkit| {
        let (stream, child) = if polkit {
            launch_polkit()?
        } else {
            launch_direct()?
        };
        let mut session = PrivdSession {
            drm_fd: None,
            input_fds: HashMap::new(),
            stream,
            child,
            fb_cache: HashMap::new(),
            mode,
            card_path: card_path.map(str::to_owned),
            polkit,
        };
        session.initialize(card_path, include_input)?;
        Ok(session)
    };
    match mode {
        PrivilegeMode::Direct => acquire(false),
        PrivilegeMode::Polkit => acquire(true),
        PrivilegeMode::Auto => match acquire(false) {
            Err(direct) if direct.is_permission_denied() => {
                acquire(true).map_err(|polkit| PrivdError::AutoFallback {
                    direct: Box::new(direct),
                    polkit: Box::new(polkit),
                })
            }
            result => result,
        },
    }
}

fn launch_direct() -> Result<(UnixStream, Child), PrivdError> {
    let (client_fd, server_fd) = socketpair(
        AddressFamily::Unix,
        SockType::SeqPacket,
        None,
        SockFlag::empty(),
    )
    .map_err(|error| PrivdError::Spawn(io::Error::other(error.to_string())))?;
    let client = unsafe { UnixStream::from_raw_fd(client_fd.into_raw_fd()) };
    let server = unsafe { UnixStream::from_raw_fd(server_fd.into_raw_fd()) };
    let server_fd = server.as_raw_fd();
    unsafe {
        let flags = libc::fcntl(server_fd, libc::F_GETFD);
        if flags >= 0 {
            libc::fcntl(server_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
        }
    }
    let child = command(resolve_privd_path().map_err(PrivdError::Spawn)?)
        .arg("--ipc-fd")
        .arg(server_fd.to_string())
        .spawn()
        .map_err(PrivdError::Spawn)?;
    drop(server);
    Ok((client, child))
}

fn launch_polkit() -> Result<(UnixStream, Child), PrivdError> {
    if !Path::new(INSTALLED_PRIVD_PATH).exists() {
        return Err(PrivdError::PolkitUnavailable(format!(
            "installed helper {INSTALLED_PRIVD_PATH} was not found"
        )));
    }
    let uid = unsafe { libc::getuid() };
    let runtime = PathBuf::from(format!("/run/user/{uid}"));
    if std::env::var_os("XDG_RUNTIME_DIR").as_deref() != Some(runtime.as_os_str()) {
        return Err(PrivdError::PolkitUnavailable(format!(
            "XDG_RUNTIME_DIR must be {}",
            runtime.display()
        )));
    }
    let dir = runtime.join("framepipe");
    match fs::create_dir(&dir) {
        Ok(()) => {
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(PrivdError::Io)?
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(PrivdError::Io(error)),
    }
    let metadata = fs::symlink_metadata(&dir).map_err(PrivdError::Io)?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || metadata.uid() != uid
        || metadata.mode() & 0o777 != 0o700
    {
        return Err(PrivdError::PolkitUnavailable(
            "unsafe runtime directory".into(),
        ));
    }
    let path = random_socket_path(&dir).map_err(PrivdError::Io)?;
    let listener = SeqPacketListener::bind(&path).map_err(PrivdError::Io)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(PrivdError::Io)?;
    let mut child = command("pkexec")
        .arg(INSTALLED_PRIVD_PATH)
        .arg("--connect")
        .arg(&path)
        .spawn()
        .map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                PrivdError::PolkitUnavailable("pkexec was not found".into())
            } else {
                PrivdError::Spawn(error)
            }
        })?;
    match listener.accept_root(&mut child, Duration::from_secs(120)) {
        Ok(stream) => Ok((stream, child)),
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            Err(error)
        }
    }
}

fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit());
    command
}

fn random_socket_path(dir: &Path) -> io::Result<PathBuf> {
    let mut bytes = [0_u8; 16];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(dir.join(format!("privd-{:032x}.sock", u128::from_ne_bytes(bytes))))
}

struct SeqPacketListener {
    fd: OwnedFd,
    path: PathBuf,
}

impl SeqPacketListener {
    fn bind(path: &Path) -> io::Result<Self> {
        let fd =
            unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_SEQPACKET | libc::SOCK_CLOEXEC, 0) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        let fd = unsafe { OwnedFd::from_raw_fd(fd) };
        let bytes = path.as_os_str().as_encoded_bytes();
        let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
        if bytes.len() >= address.sun_path.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "socket path too long",
            ));
        }
        address.sun_family = libc::AF_UNIX as _;
        for (dest, source) in address.sun_path.iter_mut().zip(bytes) {
            *dest = *source as _;
        }
        let length = size_of::<libc::sa_family_t>() + bytes.len() + 1;
        if unsafe {
            libc::bind(
                fd.as_raw_fd(),
                &address as *const _ as *const libc::sockaddr,
                length as _,
            )
        } != 0
            || unsafe { libc::listen(fd.as_raw_fd(), 4) } != 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            fd,
            path: path.into(),
        })
    }

    fn accept_root(&self, child: &mut Child, timeout: Duration) -> Result<UnixStream, PrivdError> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = child.try_wait().map_err(PrivdError::Io)? {
                return Err(if status.code() == Some(126) {
                    PrivdError::AuthenticationCancelled
                } else {
                    PrivdError::PolkitUnavailable(format!("pkexec exited with {status}"))
                });
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(PrivdError::PolkitUnavailable(
                    "timed out waiting for authentication".into(),
                ));
            }
            let mut pollfd = libc::pollfd {
                fd: self.fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe {
                libc::poll(
                    &mut pollfd,
                    1,
                    remaining.min(Duration::from_millis(250)).as_millis() as i32,
                )
            };
            if ready < 0 {
                return Err(PrivdError::Io(io::Error::last_os_error()));
            }
            if ready == 0 {
                continue;
            }
            let fd = unsafe {
                libc::accept4(
                    self.fd.as_raw_fd(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    libc::SOCK_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(PrivdError::Io(io::Error::last_os_error()));
            }
            let stream = unsafe { UnixStream::from_raw_fd(fd) };
            let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
            let mut len = size_of::<libc::ucred>() as libc::socklen_t;
            if unsafe {
                libc::getsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_PEERCRED,
                    &mut cred as *mut _ as *mut _,
                    &mut len,
                )
            } == 0
                && cred.uid == 0
            {
                return Ok(stream);
            }
        }
    }
}

impl Drop for SeqPacketListener {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn dup_fd(fd: &OwnedFd) -> Result<OwnedFd, PrivdError> {
    let dup = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_DUPFD_CLOEXEC, 0) };
    if dup < 0 {
        return Err(PrivdError::Io(io::Error::last_os_error()));
    }
    Ok(unsafe { OwnedFd::from_raw_fd(dup) })
}

fn resolve_privd_path() -> io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    if let Some(dir) = exe.parent() {
        let candidate = dir.join("framepipe-privd");
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    if Path::new(INSTALLED_PRIVD_PATH).exists() {
        return Ok(PathBuf::from(INSTALLED_PRIVD_PATH));
    }
    Ok(PathBuf::from("framepipe-privd"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_permission_errors_trigger_auto_fallback() {
        assert!(
            PrivdError::Remote {
                kind: PrivdErrorKind::PermissionDenied,
                message: String::new()
            }
            .is_permission_denied()
        );
        assert!(
            !PrivdError::Remote {
                kind: PrivdErrorKind::DeviceUnavailable,
                message: String::new()
            }
            .is_permission_denied()
        );
    }
}
