use drm::control::{connector, Device as ControlDevice, PlaneType};
use drm::ClientCapability::{UniversalPlanes, Atomic};
use drm::Device as BasicDevice;
use drm::CLOEXEC;
use std::fs::File;
use std::collections::BinaryHeap;
use std::os::fd::OwnedFd;
use std::os::unix::io::AsFd;
use std::os::unix::io::BorrowedFd;

use crate::drm_kms::types;

struct Card(File);

impl AsFd for Card {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl BasicDevice for Card {}
impl ControlDevice for Card {}

impl Card {
    fn open(path: &str) -> std::io::Result<Self> {
        let file = File::open(path)?;
        Ok(Card(file))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProbeError {
    #[error("Failed to open DRM device: {0}")]
    OpenDevice(#[source] std::io::Error),
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

fn get_dri_cards() -> Result<Vec<String>, ProbeError> {
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
        if info.state() == connector::State::Connected {
            log::info!("Connected connector: {}", info);
            log::debug!("Connector details: {:?}", info);
            connectors.push(ConnectorHeapItem(info));
        }
    }

    if connectors.is_empty() {
        log::warn!("No connected connectors found");
        return Err(ProbeError::NoConnectedConnectors);
    }

    log::debug!("Total connected connectors: {}", connectors.len());

    Ok(connectors.into_sorted_vec().into_iter().map(|item| item.0).collect())
}

fn get_matching_plane_from_connector(card: &Card, connector: &connector::Info) -> Result<Vec<drm::control::plane::Info>, ProbeError> {
    let curr_enc = connector.current_encoder().ok_or(ProbeError::GetConnectorInfo)?;
    let curr_enc_info = card.get_encoder(curr_enc).map_err(|_| ProbeError::GetCrtcInfo)?;
    let mut planes = Vec::new();

    for plane in card.plane_handles().map_err(|_| ProbeError::GetPlaneHandles)? {
        let info = card.get_plane(plane).map_err(|_| ProbeError::GetPlaneInfo)?;
        log::debug!("Checking plane {:?} for encoder {:?}", info, curr_enc);
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

fn get_primary_plane(card: &Card, planes: &[drm::control::plane::Info]) -> Result<drm::control::plane::Info, ProbeError> {
    for plane in planes {
        let properties = card.get_properties(plane.handle()).map_err(|_| ProbeError::GetPlaneProperties)?;
        for (id, value) in properties.iter() {
            let prop = card.get_property(*id).map_err(|_| ProbeError::Unknown)?;
            let name = prop.name().to_str().unwrap_or("Invalid UTF-8");
            if name == "type" && *value == PlaneType::Primary as u64 {
                log::debug!("Primary plane found: {:?}", plane);
                return Ok(plane.clone());
            }
        }
    }
    log::warn!("No primary plane found among matching planes");
    Err(ProbeError::NoPrimaryPlaneForConnector)
}

pub fn probe() -> Result<types::ProbeResult, ProbeError> {

    get_dri_cards()?;

    let card = Card::open("/dev/dri/card1").map_err(ProbeError::OpenDevice)?;

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
        let primary_plane = get_primary_plane(&card, &planes)?;
        let fb = primary_plane.framebuffer().ok_or(ProbeError::Unknown)?;
        log::debug!("{} has {:?}", primary_plane, fb);
        let fb_info = card.get_planar_framebuffer(fb).map_err(|_| ProbeError::GetFramebufferInfo)?;
        log::debug!("{} has {:?}", primary_plane, fb_info);
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
        return Ok(types::ProbeResult {
            fb_info,
            plane_fds: plane_fds,
        });
    }

    Err(ProbeError::NoConnectedConnectors)
}
