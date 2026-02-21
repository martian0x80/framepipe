use std::os::fd::{RawFd, OwnedFd};

#[derive(Clone, Copy)]
pub struct Plane {
    pub fd: RawFd,
    pub offset: u32,
    pub pitch: u32,
}

#[derive(Debug)]
pub struct ProbeResult {
    pub fb_info: drm::control::framebuffer::PlanarInfo,
    pub plane_fds: Vec<Option<OwnedFd>>, // index matches fb_info planes
}