use crate::{
    capture::types::{CaptureCursor, CaptureFrame},
    drm_kms::{
        egl_context::EglError, privd, privd::PrivdSession, probe::ProbeSession,
        types::CaptureOptions,
    },
};
use std::os::fd::AsRawFd;

use super::{backend::CaptureBackend, types::CaptureBackendKind};

#[derive(Default)]
pub struct DrmKmsBackend {
    pub probe: Option<ProbeSession>,
    pub privd: Option<PrivdSession>,
    capture_cursor: bool,
}

impl CaptureBackend for DrmKmsBackend {
    fn start(&mut self, options: &CaptureOptions) -> Result<(), EglError> {
        let include_input_fds = options.custom_cursor_composition
            || options.keyboard_overlay
            || !options.hotkeys.is_empty();
        self.capture_cursor = options.cursor_composition;
        let privd_session = privd::acquire_device_fds(
            Some(&options.card_path),
            include_input_fds,
            options.privilege_mode,
        )
        .map_err(|e| EglError::Pipeline(format!("failed to acquire device fds from privd: {e}")))?;

        let drm_fd = dup_fd(
            privd_session
                .drm_fd
                .as_ref()
                .ok_or_else(|| EglError::Pipeline("privd returned no DRM fd".into()))?
                .as_raw_fd(),
        )
        .map_err(|e| EglError::Pipeline(format!("failed to dup drm fd from privd: {e}")))?;

        let probe_session = ProbeSession::new_with_card(
            crate::drm_kms::types::Card::from_owned_fd(drm_fd),
            options.connector.clone(),
            options.allow_fallback_connector,
        )
        .map_err(EglError::Probe)?;

        self.privd = Some(privd_session);
        self.probe = Some(probe_session);
        Ok(())
    }

    fn kind(&self) -> CaptureBackendKind {
        CaptureBackendKind::DrmKms
    }

    fn next_frame(
        &mut self,
        _timeout: std::time::Duration,
    ) -> Result<Option<CaptureFrame>, EglError> {
        let probe = self.probe.as_mut().ok_or_else(|| {
            EglError::Pipeline("DRM backend not started (probe missing)".to_string())
        })?;
        let privd = self.privd.as_mut().ok_or_else(|| {
            EglError::Pipeline("DRM backend not started (privd missing)".to_string())
        })?;

        let probed = probe.capture_frame().map_err(EglError::Probe)?;
        let exported = privd
            .export_framebuffer(probed.fb_id)
            .map_err(|e| EglError::Pipeline(format!("privd framebuffer export failed: {e}")))?;
        let cursor = match self
            .capture_cursor
            .then(|| privd.export_cursor(probed.crtc_id))
        {
            None => None,
            Some(Ok(Some(exported))) => Some(CaptureCursor {
                x: exported.info.x,
                y: exported.info.y,
                width: exported.info.frame.width.max(0) as u32,
                height: exported.info.frame.height.max(0) as u32,
                fb_id: exported.info.frame.fb_id,
                buffer_width: exported.info.frame.width,
                buffer_height: exported.info.frame.height,
                fourcc: exported.info.frame.fourcc,
                modifier: exported.info.frame.modifier,
                plane_fds: exported.fds,
                offsets: exported
                    .info
                    .frame
                    .offsets
                    .iter()
                    .map(|v| (*v).max(0) as u32)
                    .collect(),
                strides: exported
                    .info
                    .frame
                    .strides
                    .iter()
                    .map(|v| (*v).max(0) as u32)
                    .collect(),
            }),
            Some(Ok(None)) => None,
            Some(Err(error)) => {
                log::debug!("KMS cursor plane export unavailable: {error}");
                None
            }
        };
        let frame = exported.info;

        Ok(Some(CaptureFrame {
            fb_id: frame.fb_id,
            width: frame.width,
            height: frame.height,
            fourcc: frame.fourcc,
            modifier: frame.modifier,
            plane_fds: exported.fds,
            offsets: frame.offsets.iter().map(|v| (*v).max(0) as u32).collect(),
            strides: frame.strides.iter().map(|v| (*v).max(0) as u32).collect(),
            cursor,
        }))
    }

    fn stop(&mut self) -> Result<(), EglError> {
        self.probe = None;
        drop(self.privd.take());
        self.privd = None;
        Ok(())
    }

    fn take_input_fds(
        &mut self,
    ) -> Option<std::collections::HashMap<std::path::PathBuf, std::os::fd::OwnedFd>> {
        self.privd
            .as_mut()
            .map(|s| std::mem::take(&mut s.input_fds))
    }
}

fn dup_fd(raw_fd: i32) -> std::io::Result<std::os::fd::OwnedFd> {
    use std::os::fd::FromRawFd;
    let dup_fd = unsafe { libc::fcntl(raw_fd, libc::F_DUPFD_CLOEXEC, 0) };
    if dup_fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { std::os::fd::OwnedFd::from_raw_fd(dup_fd) })
}
