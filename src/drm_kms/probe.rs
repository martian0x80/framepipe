use drm::control::{connector, plane, Device as ControlDevice, PlaneType};
use drm::ClientCapability::{UniversalPlanes, Atomic};
use drm::Device as BasicDevice;
use drm::CLOEXEC;
use std::collections::BinaryHeap;
use std::os::fd::OwnedFd;

use crate::drm_kms::drm::{DrmInitError, init_drm_device};
use crate::drm_kms::types::{self, Card};

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("Failed to open DRM device: {0}")]
    OpenDevice(#[source] DrmInitError),
    #[error("Failed to set client capability")]
    SetClientCapability,
    #[error("Failed to get resource handles")]
    GetResourceHandles,
    #[error("Failed to get connector info")]
    GetConnectorInfo,
    #[error("Failed to get plane handles")]
    GetPlaneHandles,
    #[error("Failed to get plane info")]
    GetPlaneInfo,
    #[error("Failed to get CRTC info")]
    GetCrtcInfo,
    #[error("Failed to find any DRM devices")]
    NoDrmDevices,
    #[error("Failed to find any connected connectors")]
    NoConnectedConnectors,
    #[error("Failed to find any matching planes for connector")]
    NoMatchingPlanesForConnector,
    #[error("Failed to get properties for plane")]
    GetPlaneProperties,
    #[error("Failed to find primary plane for connector")]
    NoPrimaryPlaneForConnector,
    #[error("Failed to get framebuffer info")]
    GetFramebufferInfo,
    #[error("Failed to convert buffer handle to PRIME fd")]
    BufferToPrimeFd,
    #[error("Unknown probe error")]
    Unknown,
}

struct ConnectorHeapItem(connector::Info);

impl PartialEq for ConnectorHeapItem {
    fn eq(&self, other: &Self) -> bool {
        self.0.modes().len() == other.0.modes().len()
    }
}

impl PartialOrd for ConnectorHeapItem {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.0.modes().len().cmp(&other.0.modes().len()))
    }
}

impl Eq for ConnectorHeapItem {}

impl Ord for ConnectorHeapItem {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.modes().len().cmp(&other.0.modes().len())
    }
}

pub fn get_dri_cards() -> Result<Vec<String>, ProbeError> {
    let mut cards = Vec::new();
    for i in 0..16 {
        let path = format!("/dev/dri/card{}", i);
        if std::fs::metadata(&path).is_ok() {
            cards.push(path);
        }
    }
    if cards.is_empty() {
        log::warn!("No DRM devices found in /dev/dri/");
        return Err(ProbeError::NoDrmDevices);
    } else {
        log::debug!("Found DRM devices: {:?}", cards);
    }
    Ok(cards)
}

fn get_connected_connectors(card: &Card) -> Result<Vec<connector::Info>, ProbeError> {
    let res = card.resource_handles().map_err(|_| ProbeError::GetResourceHandles)?;
    // let mut connected_connectors = Vec::new();
    let mut connectors = BinaryHeap::new();

    for connector in res.connectors() {
        let info = card.get_connector(*connector, false).map_err(|_| ProbeError::GetConnectorInfo)?;
        // connected connectors that are actually being displayed
        if info.state() == connector::State::Connected && info.current_encoder().is_some() {
            log::debug!("Connected connector: {}", info);
            log::trace!("Connector details: {:?}", info);
            connectors.push(ConnectorHeapItem(info));
        }
    }

    if connectors.is_empty() {
        log::warn!("No connected connectors found");
        return Err(ProbeError::NoConnectedConnectors);
    }

    log::debug!("Total connected connectors: {}", connectors.len());

    let mut out: Vec<connector::Info> = connectors.into_sorted_vec().into_iter().map(|item| item.0).collect();
    out.reverse(); // highest mode count first
    Ok(out)
}

fn get_matching_plane_from_connector(card: &Card, connector: &connector::Info) -> Result<Vec<drm::control::plane::Info>, ProbeError> {
    let curr_enc = connector.current_encoder().ok_or(ProbeError::GetConnectorInfo)?;
    let curr_enc_info = card.get_encoder(curr_enc).map_err(|_| ProbeError::GetCrtcInfo)?;
    let mut planes = Vec::new();

    for plane in card.plane_handles().map_err(|_| ProbeError::GetPlaneHandles)? {
        let info = card.get_plane(plane).map_err(|_| ProbeError::GetPlaneInfo)?;
        log::trace!("Checking plane {:?} for encoder {:?}", info, curr_enc);
        if info.crtc() == curr_enc_info.crtc() {
            log::debug!("Matching plane found: {:?}", info);
            planes.push(info);
        }
    }

    if planes.is_empty() {
        log::warn!("No matching planes found for connector {}", connector);
        return Err(ProbeError::NoMatchingPlanesForConnector);
    } else {
        log::debug!("Total matching planes for connector {}: {}", connector, planes.len())
    }

    Ok(planes)
}

fn get_best_capture_plane(
    card: &Card,
    planes: &[drm::control::plane::Info],
) -> Result<drm::control::plane::Info, ProbeError> {
    let mut best: Option<(u64, u64, drm::control::plane::Info)> = None; // (area, zpos, plane)

    for plane in planes {
        if plane.framebuffer().is_none() {
            continue;
        }

        let properties = card
            .get_properties(plane.handle())
            .map_err(|_| ProbeError::GetPlaneProperties)?;

        let mut is_cursor = false;
        let mut crtc_w: u64 = 0;
        let mut crtc_h: u64 = 0;
        let mut zpos: u64 = 0;

        for (id, value) in properties.iter() {
            let prop = card.get_property(*id).map_err(|_| ProbeError::Unknown)?;
            let name = prop.name().to_str().unwrap_or("Invalid UTF-8");
            match name {
                "type" => {
                    if *value == PlaneType::Cursor as u64 {
                        is_cursor = true;
                    }
                }
                "CRTC_W" => crtc_w = *value,
                "CRTC_H" => crtc_h = *value,
                "zpos" => zpos = *value,
                _ => {}
            }
        }

        if is_cursor {
            continue;
        }

        let area = crtc_w.saturating_mul(crtc_h);
        match &best {
            Some((best_area, best_zpos, _)) if area < *best_area || (area == *best_area && zpos <= *best_zpos) => {}
            _ => best = Some((area, zpos, plane.clone())),
        }
    }

    if let Some((area, zpos, plane)) = best {
        log::debug!(
            "Selected capture plane {:?} (area={}, zpos={}, fb={:?})",
            plane.handle(),
            area,
            zpos,
            plane.framebuffer()
        );
        Ok(plane)
    } else {
        log::warn!("No suitable non-cursor plane found among matching planes");
        Err(ProbeError::NoPrimaryPlaneForConnector)
    }
}

pub fn probe(card_path: &str) -> Result<types::ProbeResult, ProbeError> {
    let mut session = ProbeSession::new(card_path)?;
    session.capture_frame()
}

pub struct ProbeSession {
    card: Card,
    requested_connector: Option<String>,
    allow_fallback_connector: bool,
    selected_connector: Option<connector::Handle>,
    selected_connector_name: Option<String>,
    plane_handles: Vec<plane::Handle>,
}

impl ProbeSession {
    pub fn new(card_path: &str) -> Result<Self, ProbeError> {
        Self::new_with_connector(card_path, None, false)
    }

    pub fn new_with_connector(
        card_path: &str,
        requested_connector: Option<String>,
        allow_fallback_connector: bool,
    ) -> Result<Self, ProbeError> {
        let card = init_drm_device(card_path).map_err(|e| ProbeError::OpenDevice(e))?;
        card.set_client_capability(Atomic, true).map_err(|_| ProbeError::SetClientCapability)?;
        card.set_client_capability(UniversalPlanes, true).map_err(|_| ProbeError::SetClientCapability)?;
        let mut session = Self {
            card,
            requested_connector,
            allow_fallback_connector,
            selected_connector: None,
            selected_connector_name: None,
            plane_handles: Vec::new(),
        };
        session.refresh_selection()?;
        Ok(session)
    }

    fn refresh_selection(&mut self) -> Result<(), ProbeError> {
        let card = &self.card;
        let connected_connectors = get_connected_connectors(card)?;
        let picked = if let Some(requested) = &self.requested_connector {
            connected_connectors
                .iter()
                .find(|c| c.to_string() == *requested)
                .cloned()
                .or_else(|| {
                    if self.allow_fallback_connector {
                        connected_connectors.first().cloned()
                    } else {
                        None
                    }
                })
                .ok_or(ProbeError::NoConnectedConnectors)?
        } else {
            connected_connectors
                .first()
                .cloned()
                .ok_or(ProbeError::NoConnectedConnectors)?
        };

        let planes = get_matching_plane_from_connector(card, &picked)?;
        self.plane_handles = planes.iter().map(|p| p.handle()).collect();
        self.selected_connector = Some(picked.handle());
        self.selected_connector_name = Some(picked.to_string());
        log::info!(
            "Pinned connector {} with {} candidate planes",
            self.selected_connector_name.as_deref().unwrap_or("unknown"),
            self.plane_handles.len()
        );
        Ok(())
    }

    pub fn capture_frame(&mut self) -> Result<types::ProbeResult, ProbeError> {
        if self.plane_handles.is_empty() {
            self.refresh_selection()?;
        }
        let card = &self.card;

        let mut infos = Vec::with_capacity(self.plane_handles.len());
        for handle in &self.plane_handles {
            if let Ok(info) = card.get_plane(*handle) {
                infos.push(info);
            }
        }
        if infos.is_empty() {
            self.refresh_selection()?;
            return self.capture_frame();
        }

        let capture_plane = match get_best_capture_plane(card, &infos) {
            Ok(p) => p,
            Err(e) => {
                if self.allow_fallback_connector {
                    self.refresh_selection()?;
                    return self.capture_frame();
                }
                return Err(e);
            }
        };
        let fb = capture_plane.framebuffer().ok_or(ProbeError::Unknown)?;
        let fb_info = card
            .get_planar_framebuffer(fb)
            .map_err(|_| ProbeError::GetFramebufferInfo)?;
        let mut plane_fds: Vec<Option<OwnedFd>> = Vec::with_capacity(fb_info.buffers().len());
        for (i, buf) in fb_info.buffers().iter().enumerate() {
            match buf {
                Some(handle) => {
                    let fd = card
                        .buffer_to_prime_fd(*handle, CLOEXEC)
                        .map_err(|_| ProbeError::BufferToPrimeFd)?;
                    plane_fds.push(Some(fd));
                    log::trace!(
                        "fb {:?} plane {} -> handle={:?} offset={} pitch={}",
                        fb,
                        i,
                        handle,
                        fb_info.offsets()[i],
                        fb_info.pitches()[i]
                    );
                }
                None => plane_fds.push(None),
            }
        }

        let fb_id: u32 = fb.into();
        Ok(types::ProbeResult {
            fb_id,
            fb_info,
            plane_fds,
        })
    }
}

pub fn list_connectors(card_path: &str) -> Result<Vec<String>, ProbeError> {
    let card = init_drm_device(card_path).map_err(|e| ProbeError::OpenDevice(e))?;
    let connectors = get_connected_connectors(&card)?;
    Ok(connectors.into_iter().map(|c| c.to_string()).collect())
}

fn _legacy_probe(card_path: &str) -> Result<types::ProbeResult, ProbeError> {
    let card = init_drm_device(card_path).map_err(|e| ProbeError::OpenDevice(e))?;
    card.set_client_capability(Atomic, true).map_err(|_| ProbeError::SetClientCapability)?;
    card.set_client_capability(UniversalPlanes, true).map_err(|_| ProbeError::SetClientCapability)?;
    // card.set_client_capability(CursorPlaneHotspot, true)?;

    let res = card.resource_handles().map_err(|_| ProbeError::GetResourceHandles)?;

    log::debug!("Driver: {:?}", res);
    let connected_connectors = get_connected_connectors(&card)?;

    for connector in connected_connectors {
        let planes = get_matching_plane_from_connector(&card, &connector)?;
        for plane in &planes {
            log::debug!("Matching plane for connector {} -> {:?}", connector, plane);
            log::info!("Found matching plane for connector {} -> {}", connector, plane);
        }
        {
            // Just for debugging - print out all the properties of the first matching plane for this connector
            let best_plane = planes.first().ok_or(ProbeError::NoMatchingPlanesForConnector)?;
            let best_plane_id = best_plane.handle();
            let properties = card.get_properties(best_plane_id).map_err(|_| ProbeError::GetPlaneProperties)?;
            log::debug!("Plane properties ->");
            for (id, value) in properties.iter() {
                let prop = card.get_property(*id).map_err(|_| ProbeError::Unknown)?;
                let name = prop.name().to_str().unwrap_or("Invalid UTF-8");
                match name {
                    "type" => {
                        log::debug!("\t{} = {:?}", name, if *value == PlaneType::Primary as u64 { "Primary" } else if *value == PlaneType::Cursor as u64 { "Cursor" } else if *value == PlaneType::Overlay as u64 { "Overlay" } else { "Unknown" });
                    },
                    _ => {
                        log::debug!("\t{} = {:?}", name, value);
                    }
                }
            }
        }
        let capture_plane = get_best_capture_plane(&card, &planes)?;
        let fb = capture_plane.framebuffer().ok_or(ProbeError::Unknown)?;
        log::debug!("{} has {:?}", capture_plane, fb);
        let fb_info = card.get_planar_framebuffer(fb).map_err(|_| ProbeError::GetFramebufferInfo)?;
        log::debug!("{} has {:?}", capture_plane, fb_info);
        let mut plane_fds: Vec<Option<OwnedFd>> = Vec::with_capacity(fb_info.buffers().len());
        // ref: https://docs.kernel.org/gpu/drm-mm.html#c.drm_gem_prime_handle_to_fd
        // it seems like DRM_CLOEXEC is necessary
        for (i, buf) in fb_info.buffers().iter().enumerate() {
            match buf {
                Some(handle) => {
                    let fd = card.buffer_to_prime_fd(*handle, CLOEXEC).map_err(|_| ProbeError::BufferToPrimeFd)?;
                    log::debug!(
                        "plane {}: handle={:?} offset={} pitch={} -> prime_fd",
                        i,
                        handle,
                        fb_info.offsets()[i],
                        fb_info.pitches()[i]
                    );
                    plane_fds.push(Some(fd));
                }
                None => {
                    log::debug!("plane {}: no buffer handle available", i);
                    plane_fds.push(None);
                }
            }
        }
        let fb_id: u32 = fb.into();
        return Ok(types::ProbeResult {
            fb_id,
            fb_info,
            plane_fds: plane_fds,
        });
    }

    Err(ProbeError::NoConnectedConnectors)
}
