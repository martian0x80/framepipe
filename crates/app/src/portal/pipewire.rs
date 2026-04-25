use std::{
    cell::RefCell,
    io::Cursor,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    rc::Rc,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
};

use crate::{
    capture::types::CaptureFrame, portal::portal_connection,
    shared::pipewire_frame_ring::PipeWireFrameRing,
};
use ashpd::desktop::{
    PersistMode, Session,
    screencast::{CursorMode, Screencast, SelectSourcesOptions, SourceType},
};
use khronos_egl as egl;
use pipewire as pw;
use pw::{properties::properties, spa};
use spa::pod::Pod;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to create ScreenCast proxy")]
    Proxy(#[source] ashpd::Error),
    #[error("failed to create ScreenCast session")]
    CreateSession(#[source] ashpd::Error),
    #[error("failed to select sources")]
    SelectSources(#[source] ashpd::Error),
    #[error("failed to start ScreenCast session")]
    Start(#[source] ashpd::Error),
    #[error("failed to read ScreenCast start response")]
    StartResponse(#[source] ashpd::Error),
    #[error("portal returned no streams")]
    NoStreams,
    #[error("failed to open PipeWire remote fd")]
    OpenPipeWireRemote(#[source] ashpd::Error),
}

#[derive(Debug)]
pub struct PortalPipeWireRemote {
    pub node_id: u32,
    pub fd: OwnedFd,
    pub session: Session<Screencast>,
}

const DRM_FORMAT_MOD_INVALID: i64 = 0x00ff_ffff_ffff_ffff;

#[derive(Clone)]
pub struct FormatOffer {
    format: pw::spa::param::video::VideoFormat,
    #[expect(unused)]
    fourcc: u32,
    modifiers: Vec<i64>,
}

fn screencast_restore_token() -> &'static Mutex<Option<String>> {
    static TOKEN: OnceLock<Mutex<Option<String>>> = OnceLock::new();
    TOKEN.get_or_init(|| Mutex::new(None))
}

fn load_cached_restore_token() -> Option<String> {
    screencast_restore_token()
        .lock()
        .ok()
        .and_then(|g| g.clone())
}

fn store_cached_restore_token(token: Option<String>) {
    if let Ok(mut guard) = screencast_restore_token().lock() {
        *guard = token;
    }
}

async fn screencast_with_restore_token(
    restore_token: Option<String>,
) -> eyre::Result<PortalPipeWireRemote> {
    log::debug!("starting portal screencast session request");
    // i have spent days on this stupid portal api, this call used to hang indefinitely on subsequent calls
    // and it seems that was because ashpd was caching the zbus connection
    // and lldb just commits sepukku if you try to break anywhere close to this
    let proxy = tokio::time::timeout(
        tokio::time::Duration::from_secs(5),
        Screencast::with_connection(portal_connection().await?),
    )
    .await??;
    log::debug!("portal screencast proxy created");
    let session = proxy
        .create_session(Default::default())
        .await
        .map_err(Error::CreateSession)?;
    log::debug!("portal screencast session created");

    let mut select_opts = SelectSourcesOptions::default()
        .set_cursor_mode(CursorMode::Hidden)
        .set_sources(SourceType::Monitor | SourceType::Window)
        .set_multiple(false)
        .set_persist_mode(PersistMode::Application);
    // todo: verify restore token works and persist it in XDG_CONFIG_HOME or something instead
    if let Some(token) = restore_token.as_deref() {
        select_opts = select_opts.set_restore_token(Some(token));
        log::debug!("attempting screencast restore with cached restore token");
    }
    proxy
        .select_sources(&session, select_opts)
        .await
        .map_err(Error::SelectSources)?;
    log::info!("portal select_sources completed");

    let start_resp = proxy
        .start(&session, None, Default::default())
        .await
        .map_err(Error::Start)?;
    let start = start_resp.response().map_err(Error::StartResponse)?;
    store_cached_restore_token(start.restore_token().map(ToOwned::to_owned));
    if start.restore_token().is_some() {
        log::debug!("received new screencast restore token from portal");
    }
    let stream = start.streams().first().ok_or(Error::NoStreams)?;
    let node_id = stream.pipe_wire_node_id();

    let fd = proxy
        .open_pipe_wire_remote(&session, Default::default())
        .await
        .map_err(Error::OpenPipeWireRemote)?;

    log::info!(
        "portal screencast ready: node_id={} pipewire_fd={}",
        node_id,
        fd.as_raw_fd()
    );

    Ok(PortalPipeWireRemote {
        node_id,
        fd,
        session,
    })
}

pub async fn screencast() -> eyre::Result<PortalPipeWireRemote> {
    let cached = load_cached_restore_token();
    if cached.is_some() {
        match screencast_with_restore_token(cached).await {
            Ok(v) => return Ok(v),
            Err(e) => {
                log::warn!(
                    "restore-token screencast attempt failed, falling back to fresh selection: {e}"
                );
                store_cached_restore_token(None);
            }
        }
    }
    screencast_with_restore_token(None).await
}

pub async fn screencast_session(
    max_frames: u32,
    egl_i: &egl::Instance<egl::Static>,
    egl_display: egl::Display,
) -> eyre::Result<()> {
    let remote = screencast().await?;
    let run_result = run_pipewire_stream(
        &remote,
        max_frames,
        Some(egl_i),
        Some(egl_display),
        None,
        None,
        None,
    );
    if let Err(e) = remote.session.close().await {
        log::warn!("failed to close portal screencast session: {e}");
    } else {
        log::info!("portal screencast session closed");
    }
    run_result
}

fn drm_fourcc_for_spa_format(fmt: pw::spa::param::video::VideoFormat) -> Option<u32> {
    match fmt {
        pw::spa::param::video::VideoFormat::BGRx => Some(drm::buffer::DrmFourcc::Xrgb8888 as u32),
        pw::spa::param::video::VideoFormat::BGR => Some(drm::buffer::DrmFourcc::Xrgb8888 as u32),
        pw::spa::param::video::VideoFormat::RGBx => Some(drm::buffer::DrmFourcc::Xbgr8888 as u32),
        pw::spa::param::video::VideoFormat::RGB => Some(drm::buffer::DrmFourcc::Xbgr8888 as u32),
        pw::spa::param::video::VideoFormat::RGBA => Some(drm::buffer::DrmFourcc::Abgr8888 as u32),
        pw::spa::param::video::VideoFormat::BGRA => Some(drm::buffer::DrmFourcc::Argb8888 as u32),
        pw::spa::param::video::VideoFormat::ARGB => Some(drm::buffer::DrmFourcc::Argb8888 as u32),
        pw::spa::param::video::VideoFormat::ABGR => Some(drm::buffer::DrmFourcc::Abgr8888 as u32),
        _ => None,
    }
}

fn query_modifiers_for_drm_format(
    egl_i: &egl::Instance<egl::Static>,
    display: egl::Display,
    drm_format: u32,
) -> Vec<i64> {
    type EglQueryDmaBufModifiersExt = unsafe extern "C" fn(
        dpy: egl::EGLDisplay,
        format: i32,
        max_modifiers: i32,
        modifiers: *mut u64,
        external_only: *mut egl::Boolean,
        num_modifiers: *mut i32,
    ) -> egl::Boolean;

    let Some(sym) = egl_i.get_proc_address("eglQueryDmaBufModifiersEXT") else {
        return vec![DRM_FORMAT_MOD_INVALID];
    };
    let query: EglQueryDmaBufModifiersExt = unsafe { std::mem::transmute(sym) };

    let mut n: i32 = 0;
    let ok = unsafe {
        query(
            display.as_ptr(),
            drm_format as i32,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut n,
        )
    };
    if ok == egl::FALSE || n <= 0 {
        return vec![DRM_FORMAT_MOD_INVALID];
    }

    let mut modifiers = vec![0_u64; n as usize];
    let ok = unsafe {
        query(
            display.as_ptr(),
            drm_format as i32,
            n,
            modifiers.as_mut_ptr(),
            std::ptr::null_mut(),
            &mut n,
        )
    };
    if ok == egl::FALSE || n <= 0 {
        return vec![DRM_FORMAT_MOD_INVALID];
    }

    modifiers.truncate(n as usize);
    let mut out = Vec::with_capacity(modifiers.len());
    for m in &modifiers {
        let v = *m as i64;
        if v == DRM_FORMAT_MOD_INVALID {
            continue;
        }
        out.push(v);
    }

    if !out.contains(&DRM_FORMAT_MOD_INVALID) {
        out.push(DRM_FORMAT_MOD_INVALID);
    }
    out
}

fn serialize_object_to_bytes(object: spa::pod::Object) -> eyre::Result<Vec<u8>> {
    let bytes = pw::spa::pod::serialize::PodSerializer::serialize(
        Cursor::new(Vec::new()),
        &pw::spa::pod::Value::Object(object),
    )?
    .0
    .into_inner();
    Ok(bytes)
}

fn build_pipewire_format_offers(
    egl_i: &egl::Instance<egl::Static>,
    egl_display: egl::Display,
) -> Vec<FormatOffer> {
    let formats = [
        pw::spa::param::video::VideoFormat::RGBx,
        pw::spa::param::video::VideoFormat::BGRx,
        pw::spa::param::video::VideoFormat::RGBA,
        pw::spa::param::video::VideoFormat::BGRA,
        pw::spa::param::video::VideoFormat::RGB,
        pw::spa::param::video::VideoFormat::BGR,
        pw::spa::param::video::VideoFormat::ARGB,
        pw::spa::param::video::VideoFormat::ABGR,
    ];

    let mut offers = Vec::new();
    for format in formats {
        let Some(fourcc) = drm_fourcc_for_spa_format(format) else {
            continue;
        };
        let modifiers = query_modifiers_for_drm_format(egl_i, egl_display, fourcc);
        offers.push(FormatOffer {
            format,
            fourcc,
            modifiers,
        });
    }
    offers
}

fn build_enum_format_object(
    format: pw::spa::param::video::VideoFormat,
    modifiers: &[i64],
) -> spa::pod::Object {
    let mut properties = vec![
        spa::pod::Property::new(
            spa::param::format::FormatProperties::MediaType.as_raw(),
            spa::pod::Value::Id(spa::utils::Id(
                spa::param::format::MediaType::Video.as_raw(),
            )),
        ),
        spa::pod::Property::new(
            spa::param::format::FormatProperties::MediaSubtype.as_raw(),
            spa::pod::Value::Id(spa::utils::Id(
                spa::param::format::MediaSubtype::Raw.as_raw(),
            )),
        ),
        spa::pod::Property::new(
            spa::param::format::FormatProperties::VideoFormat.as_raw(),
            spa::pod::Value::Choice(spa::pod::ChoiceValue::Id(spa::utils::Choice(
                spa::utils::ChoiceFlags::empty(),
                spa::utils::ChoiceEnum::Enum {
                    default: spa::utils::Id(format.as_raw()),
                    alternatives: vec![spa::utils::Id(format.as_raw())],
                },
            ))),
        ),
        spa::pod::Property::new(
            spa::param::format::FormatProperties::VideoSize.as_raw(),
            spa::pod::Value::Choice(spa::pod::ChoiceValue::Rectangle(spa::utils::Choice(
                spa::utils::ChoiceFlags::empty(),
                spa::utils::ChoiceEnum::Range {
                    default: spa::utils::Rectangle {
                        width: 32,
                        height: 32,
                    },
                    min: spa::utils::Rectangle {
                        width: 1,
                        height: 1,
                    },
                    max: spa::utils::Rectangle {
                        width: 16384,
                        height: 16384,
                    },
                },
            ))),
        ),
        spa::pod::Property::new(
            spa::param::format::FormatProperties::VideoFramerate.as_raw(),
            spa::pod::Value::Choice(spa::pod::ChoiceValue::Fraction(spa::utils::Choice(
                spa::utils::ChoiceFlags::empty(),
                spa::utils::ChoiceEnum::Range {
                    default: spa::utils::Fraction { num: 60, denom: 1 },
                    min: spa::utils::Fraction { num: 0, denom: 1 },
                    max: spa::utils::Fraction { num: 500, denom: 1 },
                },
            ))),
        ),
    ];

    if !modifiers.is_empty() {
        let mut modifier_prop = spa::pod::Property::new(
            spa::param::format::FormatProperties::VideoModifier.as_raw(),
            spa::pod::Value::Choice(spa::pod::ChoiceValue::Long(spa::utils::Choice(
                spa::utils::ChoiceFlags::empty(),
                spa::utils::ChoiceEnum::Enum {
                    default: modifiers[0],
                    alternatives: modifiers.to_vec(),
                },
            ))),
        );
        modifier_prop.flags = spa::pod::PropertyFlags::MANDATORY
            | spa::pod::PropertyFlags::from_bits_retain(spa::sys::SPA_POD_PROP_FLAG_DONT_FIXATE);
        properties.push(modifier_prop);
    }

    spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties,
    }
}

fn serialize_format_offers_to_bytes(offers: &[FormatOffer]) -> eyre::Result<Vec<Vec<u8>>> {
    let mut format_bytes = Vec::new();
    for offer in offers {
        // Preferred: modifier-aware offer.
        format_bytes.push(serialize_object_to_bytes(build_enum_format_object(
            offer.format,
            &offer.modifiers,
        ))?);
        // Fallback: same format without modifiers.
        format_bytes.push(serialize_object_to_bytes(build_enum_format_object(
            offer.format,
            &[],
        ))?);
    }
    Ok(format_bytes)
}

fn run_pipewire_stream(
    remote: &PortalPipeWireRemote,
    max_frames: u32,
    _egl_i: Option<&egl::Instance<egl::Static>>,
    _egl_display: Option<egl::Display>,
    prebuilt_offers: Option<Vec<FormatOffer>>,
    frame_ring: Option<Arc<PipeWireFrameRing>>,
    stop: Option<Arc<AtomicBool>>,
) -> eyre::Result<()> {
    pw::init();

    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let remote_fd = dup_fd_raw(remote.fd.as_raw_fd())?;
    let core = context.connect_fd_rc(remote_fd, None)?;

    let stream_props = properties! {
        *pw::keys::MEDIA_TYPE => "Video",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Screen",
    };

    let stream = pw::stream::StreamBox::new(&core, "framepipe-portal", stream_props)?;

    let params_sent = AtomicBool::new(false);
    let should_stop = std::rc::Rc::new(AtomicBool::new(false));
    let frame_count = std::rc::Rc::new(std::cell::Cell::new(0_u32));
    let frame_id = std::rc::Rc::new(std::cell::Cell::new(1_u32));
    let frame_fmt = std::rc::Rc::new(std::cell::Cell::new(None::<FrameFormat>));
    let mainloop_for_cb = mainloop.clone();

    let offers = prebuilt_offers.unwrap_or_else(|| {
        let egl_i = _egl_i.expect("missing EGL instance for pipewire stream");
        let egl_display = _egl_display.expect("missing EGL display for pipewire stream");
        build_pipewire_format_offers(egl_i, egl_display)
    });
    let offers_state = Rc::new(RefCell::new(offers));

    let _listener = stream
        .add_local_listener_with_user_data(spa::param::video::VideoInfoRaw::new())
        .state_changed({
            let mainloop_for_cb = mainloop_for_cb.clone();
            let should_stop = std::rc::Rc::clone(&should_stop);
            move |_, _, old, new| {
                log::debug!("pipewire stream state: {:?} -> {:?}", old, new);
                if matches!(new, pw::stream::StreamState::Error(_)) {
                    should_stop.store(true, Ordering::Release);
                    mainloop_for_cb.quit();
                }
            }
        })
        .param_changed({
            let frame_fmt = std::rc::Rc::clone(&frame_fmt);
            move |stream, user_data, id, param| {
                let Some(param) = param else {
                    return;
                };
                if id != pw::spa::param::ParamType::Format.as_raw() {
                    return;
                }

                let (media_type, media_subtype) =
                    match pw::spa::param::format_utils::parse_format(param) {
                        Ok(v) => v,
                        Err(e) => {
                            log::warn!("pipewire parse_format failed for param id={}: {}", id, e);
                            return;
                        }
                    };

                if media_type != pw::spa::param::format::MediaType::Video
                    || media_subtype != pw::spa::param::format::MediaSubtype::Raw
                {
                    return;
                }

                if let Err(e) = user_data.parse(param) {
                    log::warn!("pipewire video raw parse failed for param id={}: {}", id, e);
                    return;
                }

                log::info!(
                    "pipewire negotiated format: fmt={:?} size={}x{} fps={}/{} modifier=0x{:016x}",
                    user_data.format(),
                    user_data.size().width,
                    user_data.size().height,
                    user_data.framerate().num,
                    user_data.framerate().denom,
                    user_data.modifier(),
                );
                if let Some(fourcc) = drm_fourcc_for_spa_format(user_data.format()) {
                    frame_fmt.set(Some(FrameFormat {
                        width: user_data.size().width as i32,
                        height: user_data.size().height as i32,
                        fourcc,
                        modifier: Some(user_data.modifier()),
                    }));
                }

                if params_sent.swap(true, Ordering::AcqRel) {
                    return;
                }

                let meta_video_crop = spa::pod::Object {
                    type_: spa::utils::SpaTypes::ObjectParamMeta.as_raw(),
                    id: spa::param::ParamType::Meta.as_raw(),
                    properties: vec![
                        spa::pod::Property::new(
                            spa::sys::SPA_PARAM_META_type,
                            spa::pod::Value::Id(spa::utils::Id(spa::sys::SPA_META_VideoCrop)),
                        ),
                        spa::pod::Property::new(
                            spa::sys::SPA_PARAM_META_size,
                            spa::pod::Value::Int(
                                std::mem::size_of::<spa::sys::spa_meta_region>() as i32
                            ),
                        ),
                    ],
                };
                let meta_video_damage = spa::pod::Object {
                    type_: spa::utils::SpaTypes::ObjectParamMeta.as_raw(),
                    id: spa::param::ParamType::Meta.as_raw(),
                    properties: vec![
                        spa::pod::Property::new(
                            spa::sys::SPA_PARAM_META_type,
                            spa::pod::Value::Id(spa::utils::Id(spa::sys::SPA_META_VideoDamage)),
                        ),
                        spa::pod::Property::new(
                            spa::sys::SPA_PARAM_META_size,
                            spa::pod::Value::Int(
                                std::mem::size_of::<spa::sys::spa_meta_region>() as i32
                            ),
                        ),
                    ],
                };
                // doesn't really work for most compositors, so lets just ignre
                // let meta_cursor = spa::pod::Object {
                //     type_: spa::utils::SpaTypes::ObjectParamMeta.as_raw(),
                //     id: spa::param::ParamType::Meta.as_raw(),
                //     properties: vec![
                //         spa::pod::Property::new(
                //             spa::sys::SPA_PARAM_META_type,
                //             spa::pod::Value::Id(spa::utils::Id(spa::sys::SPA_META_Cursor)),
                //         ),
                //         spa::pod::Property::new(
                //             spa::sys::SPA_PARAM_META_size,
                //             spa::pod::Value::Int(
                //                 std::mem::size_of::<spa::sys::spa_meta_cursor>() as i32,
                //             ),
                //         ),
                //     ],
                // };

                let buffers = spa::pod::Object {
                    type_: spa::utils::SpaTypes::ObjectParamBuffers.as_raw(),
                    id: spa::param::ParamType::Buffers.as_raw(),
                    properties: vec![spa::pod::Property::new(
                        spa::sys::SPA_PARAM_BUFFERS_dataType,
                        spa::pod::Value::Int((1_u32 << spa::sys::SPA_DATA_DmaBuf) as i32),
                    )],
                };

                let mut bytes = Vec::<Vec<u8>>::new();
                for object in [meta_video_crop, meta_video_damage, buffers] {
                    match serialize_object_to_bytes(object) {
                        Ok(v) => bytes.push(v),
                        Err(e) => {
                            log::warn!("pipewire param serialization failed: {e}");
                            return;
                        }
                    }
                }

                let mut pods = bytes
                    .iter()
                    .filter_map(|b| spa::pod::Pod::from_bytes(b))
                    .collect::<Vec<_>>();
                if pods.is_empty() {
                    return;
                }

                if let Err(e) = stream.update_params(&mut pods) {
                    log::warn!("pipewire update_params failed: {e}");
                }
            }
        })
        .process({
            let frame_ring = frame_ring.clone();
            let stop = stop.clone();
            let frame_id = std::rc::Rc::clone(&frame_id);
            let frame_fmt = std::rc::Rc::clone(&frame_fmt);
            move |stream, _| unsafe {
            if stop.as_ref().is_some_and(|s| s.load(Ordering::Acquire)) {
                mainloop_for_cb.quit();
                return;
            }
            let raw = stream.dequeue_raw_buffer();
            if raw.is_null() {
                return;
            }

            let spa_buf = (*raw).buffer;
            if spa_buf.is_null() {
                stream.queue_raw_buffer(raw);
                return;
            }

            let n_datas = (*spa_buf).n_datas as usize;
            let mut plane_fds = Vec::new();
            let mut offsets = Vec::new();
            let mut strides = Vec::new();
            for i in 0..n_datas {
                let d = &*((*spa_buf).datas.add(i));
                if d.type_ == spa::sys::SPA_DATA_DmaBuf {
                    let (offset, size, stride) = if d.chunk.is_null() {
                        (0_u32, 0_u32, 0_i32)
                    } else {
                        ((*d.chunk).offset, (*d.chunk).size, (*d.chunk).stride)
                    };
                    log::trace!(
                        "pipewire dmabuf plane {}: fd={} offset={} size={} stride={}",
                        i,
                        d.fd,
                        offset,
                        size,
                        stride
                    );
                    if d.fd >= 0 {
                        let Ok(raw_fd) = i32::try_from(d.fd) else {
                            continue;
                        };
                        if let Ok(fd) = dup_fd_raw(raw_fd) {
                            plane_fds.push(fd);
                            offsets.push(offset);
                            strides.push(stride.max(0) as u32);
                            log::trace!(
                                "pipewire backend dmabuf plane {} duplicated: fd={} offset={} stride={}",
                                i,
                                d.fd,
                                offset,
                                stride
                            );
                        }
                    }
                }
            }

            let next = frame_count.get().saturating_add(1);
            frame_count.set(next);

            // for fd in &plane_fds {
            //     let mut pfd = libc::pollfd {
            //         fd: fd.as_raw_fd(),
            //         events: libc::POLLOUT,
            //         revents: 0,
            //     };
            //     let waited = libc::poll(&mut pfd as *mut libc::pollfd, 1, 8);
            //     if waited == 0 {
            //         log::trace!("pipewire dmabuf fence wait timeout on fd={}", fd.as_raw_fd());
            //     }
            // }

            stream.queue_raw_buffer(raw);

            if let (Some(ring), Some(fmt)) = (frame_ring.as_ref(), frame_fmt.get())
                && !plane_fds.is_empty() {
                    let id = frame_id.get();
                    frame_id.set(id.saturating_add(1));
                    ring.push_overwrite(CaptureFrame {
                        fb_id: id,
                        width: fmt.width,
                        height: fmt.height,
                        fourcc: fmt.fourcc,
                        modifier: fmt.modifier,
                        plane_fds,
                        offsets,
                        strides,
                    });
                }

            if next >= max_frames {
                mainloop_for_cb.quit();
            }
        }
        })
        .register()?;

    let format_bytes = serialize_format_offers_to_bytes(&offers_state.borrow())?;

    let mut params = format_bytes
        .iter()
        .filter_map(|b| Pod::from_bytes(b))
        .collect::<Vec<_>>();
    if params.is_empty() {
        return Err(eyre::eyre!("no pipewire enum format params built"));
    }

    stream.connect(
        spa::utils::Direction::Input,
        Some(remote.node_id),
        pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
        &mut params[..],
    )?;
    stream.set_active(true)?;

    log::info!(
        "pipewire stream started: node={} max_frames={}",
        remote.node_id,
        max_frames
    );

    mainloop.run();
    let _ = stream.set_active(false);
    let _ = stream.disconnect();
    drop(stream);

    if should_stop.load(Ordering::Acquire) {
        return Err(eyre::eyre!(
            "pipewire stream ended with error before completion"
        ));
    }

    Ok(())
}

#[derive(Clone, Copy)]
struct FrameFormat {
    width: i32,
    height: i32,
    fourcc: u32,
    modifier: Option<u64>,
}

pub struct PipeWireCaptureProducer {
    stop: Arc<AtomicBool>,
    // do we even need both?
    ended: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl PipeWireCaptureProducer {
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }

    pub fn is_ended(&self) -> bool {
        self.ended.load(Ordering::Acquire)
    }

    pub fn has_failed(&self) -> bool {
        self.failed.load(Ordering::Acquire)
    }
}

impl Drop for PipeWireCaptureProducer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

pub fn start_capture_producer(
    ring: Arc<PipeWireFrameRing>,
    format_offers: Vec<FormatOffer>,
) -> Result<PipeWireCaptureProducer, String> {
    let stop = Arc::new(AtomicBool::new(false));
    let ended = Arc::new(AtomicBool::new(false));
    let failed = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);
    let ended_thread = Arc::clone(&ended);
    let failed_thread = Arc::clone(&failed);
    let handle = thread::spawn(move || {
        log::info!(
            "starting pipewire capture producer thread (format_offers={})",
            format_offers.len()
        );
        let rt = match tokio::runtime::Runtime::new() {
            Ok(v) => v,
            Err(e) => {
                log::error!("pipewire producer failed to create tokio runtime: {e}");
                failed_thread.store(true, Ordering::Release);
                ended_thread.store(true, Ordering::Release);
                return;
            }
        };
        let remote = match rt.block_on(screencast()) {
            Ok(v) => v,
            Err(e) => {
                log::error!("pipewire producer failed to start screencast portal: {e}");
                failed_thread.store(true, Ordering::Release);
                ended_thread.store(true, Ordering::Release);
                return;
            }
        };
        if let Err(e) = run_pipewire_stream(
            &remote,
            u32::MAX,
            None,
            None,
            Some(format_offers),
            Some(ring),
            Some(stop_thread),
        ) {
            log::error!("pipewire producer stream error: {e}");
            failed_thread.store(true, Ordering::Release);
        }
        if let Err(e) = rt.block_on(remote.session.close()) {
            log::warn!("failed to close portal screencast session: {e}");
        } else {
            log::info!("portal screencast session closed");
        }
        ended_thread.store(true, Ordering::Release);
    });
    Ok(PipeWireCaptureProducer {
        stop,
        ended,
        failed,
        handle: Some(handle),
    })
}

pub fn build_pipewire_enum_format_bytes(
    egl_i: &egl::Instance<egl::Static>,
    egl_display: egl::Display,
) -> eyre::Result<Vec<Vec<u8>>> {
    let offers = build_pipewire_format_offers(egl_i, egl_display);
    serialize_format_offers_to_bytes(&offers)
}

pub fn build_pipewire_format_offers_for_session(
    egl_i: &egl::Instance<egl::Static>,
    egl_display: egl::Display,
) -> Vec<FormatOffer> {
    build_pipewire_format_offers(egl_i, egl_display)
}

fn dup_fd_raw(raw_fd: i32) -> std::io::Result<OwnedFd> {
    let dup_fd = unsafe { libc::fcntl(raw_fd, libc::F_DUPFD_CLOEXEC, 0) };
    if dup_fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(dup_fd) })
}
