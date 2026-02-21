use std::os::fd::OwnedFd;
use std::os::unix::io::{AsFd, BorrowedFd};
use std::fs::File;
use drm::control::Device as ControlDevice;
use drm::Device as BasicDevice;

#[derive(Debug)]
pub(crate) struct Card(File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl BasicDevice for Card {}
impl ControlDevice for Card {}

impl Card {
    pub fn open(path: &str) -> std::io::Result<Self> {
        let file = File::open(path)?;
        Ok(Card(file))
    }
}

#[derive(Debug)]
pub struct ProbeResult {
    pub fb_info: drm::control::framebuffer::PlanarInfo,
    pub plane_fds: Vec<Option<OwnedFd>>, // index matches fb_info planes
}