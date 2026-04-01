use std::collections::HashMap;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use common::ipc::{recv_packet, send_packet};
use common::types::{ExportedFrameInfo, IpcRequest, IpcResponse};
use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};

#[derive(Debug, thiserror::Error)]
pub enum PrivdError {
    #[error("failed to spawn privd: {0}")]
    Spawn(#[source] io::Error),
    #[error("ipc error: {0}")]
    Io(#[source] io::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
}

pub struct PrivdSession {
    pub drm_fd: OwnedFd,
    pub input_fds: HashMap<PathBuf, OwnedFd>,
    stream: UnixStream,
    child: Child,
}

impl Drop for PrivdSession {
    fn drop(&mut self) {
        let _ = send_packet(&self.stream, &IpcRequest::Stop, &[]);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub struct ExportedFrame {
    pub info: ExportedFrameInfo,
    pub fds: Vec<OwnedFd>,
}

impl PrivdSession {
    pub fn export_framebuffer(&mut self, fb_id: u32) -> Result<ExportedFrame, PrivdError> {
        log::trace!("requesting framebuffer export from privd: fb={}", fb_id);
        let req = IpcRequest::ExportFramebuffer { fb_id };
        send_packet(&self.stream, &req, &[]).map_err(PrivdError::Io)?;
        let (resp, fds): (IpcResponse, Vec<OwnedFd>) =
            recv_packet(&self.stream).map_err(PrivdError::Io)?;
        match resp {
            IpcResponse::FrameExported { frame } => {
                log::trace!(
                    "received exported framebuffer from privd: fb={} planes={}",
                    frame.fb_id,
                    fds.len()
                );
                Ok(ExportedFrame { info: frame, fds })
            }
            IpcResponse::Error { message } => Err(PrivdError::Protocol(message)),
            other => Err(PrivdError::Protocol(format!(
                "unexpected privd response to ExportFramebuffer: {:?}",
                other
            ))),
        }
    }
}

pub fn acquire_device_fds(card_path: &str, include_input_fds: bool) -> Result<PrivdSession, PrivdError> {
    log::info!(
        "acquire_device_fds: card_path={} include_input_fds={}",
        card_path,
        include_input_fds
    );
    let (client_fd, server_fd) = socketpair(
        AddressFamily::Unix,
        SockType::SeqPacket,
        None,
        SockFlag::empty(),
    )
    .map_err(|e| PrivdError::Spawn(io::Error::other(e.to_string())))?;

    let client = unsafe { UnixStream::from_raw_fd(client_fd.into_raw_fd()) };
    let server = unsafe { UnixStream::from_raw_fd(server_fd.into_raw_fd()) };
    let server_raw_fd = server.as_raw_fd();

    unsafe {
        let flags = libc::fcntl(server_raw_fd, libc::F_GETFD);
        if flags >= 0 {
            let _ = libc::fcntl(server_raw_fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC);
        }
    }

    let privd_bin = resolve_privd_path().map_err(PrivdError::Spawn)?;
    log::debug!("spawning privd binary at {}", privd_bin.to_string_lossy());
    let mut cmd = Command::new(privd_bin);
    cmd.arg("--ipc-fd").arg(server_raw_fd.to_string());
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::inherit());
    let child = cmd.spawn().map_err(PrivdError::Spawn)?;
    drop(server);

    let req = IpcRequest::StartSession {
        card_path: card_path.to_string(),
        include_input_fds,
    };
    log::debug!("sending StartSession request");
    send_packet(&client, &req, &[]).map_err(PrivdError::Io)?;

    let (resp, mut fds): (IpcResponse, Vec<OwnedFd>) = recv_packet(&client).map_err(PrivdError::Io)?;
    log::debug!("received privd response with {} fds", fds.len());
    let (input_devices, returned_card_path) = match resp {
        IpcResponse::SessionReady {
            card_path,
            input_devices,
        } => (input_devices, card_path),
        IpcResponse::Error { message } => return Err(PrivdError::Protocol(message)),
        other => {
            return Err(PrivdError::Protocol(format!(
                "unexpected response to StartSession: {:?}",
                other
            )))
        }
    };

    if returned_card_path != card_path {
        return Err(PrivdError::Protocol(format!(
            "privd returned unexpected card path: requested={} got={}",
            card_path, returned_card_path
        )));
    }
    if fds.is_empty() {
        return Err(PrivdError::Protocol(
            "privd returned no fds; expected at least drm fd".to_string(),
        ));
    }

    let drm_fd = fds.remove(0);
    if fds.len() != input_devices.len() {
        return Err(PrivdError::Protocol(format!(
            "privd fd count mismatch: input map count={} fd count={}",
            input_devices.len(),
            fds.len()
        )));
    }

    let mut input_fds = HashMap::new();
    for (info, fd) in input_devices.into_iter().zip(fds.into_iter()) {
        input_fds.insert(PathBuf::from(info.path), fd);
    }
    log::info!(
        "privd session ready: drm_fd=1 input_fds={}",
        input_fds.len()
    );

    Ok(PrivdSession {
        drm_fd,
        input_fds,
        stream: client,
        child,
    })
}

fn resolve_privd_path() -> io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let Some(dir) = exe.parent() else {
        return Err(io::Error::other("missing current_exe parent"));
    };
    let candidate = dir.join("framepipe-privd");
    if candidate.exists() {
        return Ok(candidate);
    }
    Ok(PathBuf::from("framepipe-privd"))
}
