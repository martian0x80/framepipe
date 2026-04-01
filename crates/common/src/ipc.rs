use std::io;
use std::io::{IoSlice, IoSliceMut};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;

use nix::errno::Errno;
use nix::sys::socket::{
    ControlMessage, ControlMessageOwned, MsgFlags, recvmsg, sendmsg,
};
use serde::de::DeserializeOwned;
use serde::Serialize;

const IPC_MAX_PAYLOAD: usize = 64 * 1024;
const IPC_MAX_FDS: usize = 256;

pub fn send_packet<T: Serialize>(stream: &UnixStream, msg: &T, fds: &[RawFd]) -> io::Result<()> {
    let payload = bincode::serialize(msg)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    if payload.len() > IPC_MAX_PAYLOAD {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("ipc payload too large: {}", payload.len()),
        ));
    }

    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(&payload);
    let iov = [IoSlice::new(&frame)];

    let cmsgs = if fds.is_empty() {
        Vec::new()
    } else {
        vec![ControlMessage::ScmRights(fds)]
    };

    sendmsg::<()>(
        stream.as_raw_fd(),
        &iov,
        &cmsgs,
        MsgFlags::MSG_NOSIGNAL,
        None,
    )
    .map_err(map_nix_err)?;
    Ok(())
}

pub fn recv_packet<T: DeserializeOwned>(stream: &UnixStream) -> io::Result<(T, Vec<OwnedFd>)> {
    let mut buf = vec![0u8; IPC_MAX_PAYLOAD + 4];
    let mut cmsgspace = nix::cmsg_space!([RawFd; IPC_MAX_FDS]);
    let (bytes, raw_fds) = {
        let mut iov = [IoSliceMut::new(&mut buf)];
        let recv = recvmsg::<()>(
            stream.as_raw_fd(),
            &mut iov,
            Some(&mut cmsgspace),
            MsgFlags::MSG_CMSG_CLOEXEC,
        )
        .map_err(map_nix_err)?;

        let bytes = recv.bytes;
        let mut raw_fds = Vec::new();
        if let Ok(cmsgs) = recv.cmsgs() {
            for cmsg in cmsgs {
                if let ControlMessageOwned::ScmRights(rights) = cmsg {
                    raw_fds.extend(rights);
                }
            }
        }
        (bytes, raw_fds)
    };

    if bytes < 4 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "short ipc header",
        ));
    }
    let len = u32::from_le_bytes(buf[0..4].try_into().unwrap()) as usize;
    if 4 + len > bytes {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "short ipc payload",
        ));
    }

    let msg: T = bincode::deserialize(&buf[4..4 + len])
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;

    let fds = raw_fds
        .into_iter()
        .map(|fd| unsafe { OwnedFd::from_raw_fd(fd) })
        .collect::<Vec<_>>();

    Ok((msg, fds))
}

fn map_nix_err(err: nix::Error) -> io::Error {
    if err == Errno::EAGAIN || err == Errno::EWOULDBLOCK {
        return io::Error::new(io::ErrorKind::WouldBlock, err.to_string());
    }
    io::Error::new(io::ErrorKind::Other, err.to_string())
}
