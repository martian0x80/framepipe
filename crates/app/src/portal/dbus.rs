use std::{
    io::Cursor,
    os::fd::{AsRawFd, OwnedFd},
    sync::atomic::{AtomicBool, Ordering},
};

use ashpd::desktop::{
    PersistMode,
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
}

const DRM_FORMAT_MOD_INVALID: i64 = 0x00ff_ffff_ffff_ffff;

pub async fn screencast() -> eyre::Result<PortalPipeWireRemote> {
    let proxy = Screencast::new().await.map_err(Error::Proxy)?;
    let session = proxy
        .create_session(Default::default())
        .await
        .map_err(Error::CreateSession)?;

    let select_opts = SelectSourcesOptions::default()
        .set_cursor_mode(CursorMode::Hidden)
        .set_sources(SourceType::Monitor | SourceType::Window)
        .set_multiple(false)
        .set_persist_mode(PersistMode::DoNot);
    proxy
        .select_sources(&session, select_opts)
        .await
        .map_err(Error::SelectSources)?;

    let start_resp = proxy
        .start(&session, None, Default::default())
        .await
        .map_err(Error::Start)?;
    let start = start_resp.response().map_err(Error::StartResponse)?;
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

    Ok(PortalPipeWireRemote { node_id, fd })
}

pub async fn screencast_session(
    max_frames: u32,
    egl_i: &egl::Instance<egl::Static>,
    egl_display: egl::Display,
) -> eyre::Result<()> {
    let remote = screencast().await?;
    run_pipewire_stream(remote, max_frames, egl_i, egl_display)
}

fn fourcc(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}

fn drm_fourcc_for_spa_format(fmt: pw::spa::param::video::VideoFormat) -> Option<u32> {
    match fmt {
        pw::spa::param::video::VideoFormat::BGRx => Some(fourcc(b'X', b'R', b'2', b'4')), // XRGB8888
        pw::spa::param::video::VideoFormat::BGR => Some(fourcc(b'X', b'R', b'2', b'4')), // XRGB8888
        pw::spa::param::video::VideoFormat::RGBx => Some(fourcc(b'X', b'B', b'2', b'4')), // XBGR8888
        pw::spa::param::video::VideoFormat::RGB => Some(fourcc(b'X', b'B', b'2', b'4')), // XBGR8888
        pw::spa::param::video::VideoFormat::RGBA => Some(fourcc(b'A', b'B', b'2', b'4')), // ABGR8888
        pw::spa::param::video::VideoFormat::BGRA => Some(fourcc(b'A', b'R', b'2', b'4')), // ARGB8888
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
    let mut external_only = vec![egl::FALSE; n as usize];
    let ok = unsafe {
        query(
            display.as_ptr(),
            drm_format as i32,
            n,
            modifiers.as_mut_ptr(),
            external_only.as_mut_ptr(),
            &mut n,
        )
    };
    if ok == egl::FALSE || n <= 0 {
        return vec![DRM_FORMAT_MOD_INVALID];
    }

    modifiers.truncate(n as usize);
    let mut out = Vec::with_capacity(modifiers.len() + 1);
    for m in modifiers {
        let v = m as i64;
        if v != DRM_FORMAT_MOD_INVALID {
            out.push(v);
        }
    }
    out.insert(0, DRM_FORMAT_MOD_INVALID);
    out
}

fn run_pipewire_stream(
    remote: PortalPipeWireRemote,
    max_frames: u32,
    egl_i: &egl::Instance<egl::Static>,
    egl_display: egl::Display,
) -> eyre::Result<()> {
    pw::init();

    let mainloop = pw::main_loop::MainLoopRc::new(None)?;
    let context = pw::context::ContextRc::new(&mainloop, None)?;
    let core = context.connect_fd_rc(remote.fd, None)?;

    let stream_props = properties! {
        *pw::keys::MEDIA_TYPE => "Video",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Screen",
    };

    let stream = pw::stream::StreamBox::new(&core, "framepipe-portal", stream_props)?;

    let params_sent = AtomicBool::new(false);
    let should_stop = std::rc::Rc::new(AtomicBool::new(false));
    let frame_count = std::rc::Rc::new(std::cell::Cell::new(0_u32));
    let mainloop_for_cb = mainloop.clone();

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
        .process(move |stream, _| unsafe {
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
            for i in 0..n_datas {
                let d = &*((*spa_buf).datas.add(i));
                if d.type_ == spa::sys::SPA_DATA_DmaBuf {
                    let (offset, size, stride) = if d.chunk.is_null() {
                        (0_u32, 0_u32, 0_i32)
                    } else {
                        ((*d.chunk).offset, (*d.chunk).size, (*d.chunk).stride)
                    };
                    log::debug!(
                        "pipewire dmabuf plane {}: fd={} offset={} size={} stride={}",
                        i,
                        d.fd,
                        offset,
                        size,
                        stride
                    );
                }
            }

            let next = frame_count.get().saturating_add(1);
            frame_count.set(next);
            stream.queue_raw_buffer(raw);

            if next >= max_frames {
                mainloop_for_cb.quit();
            }
        })
        .register()?;

    fn serialize_object_to_bytes(object: spa::pod::Object) -> eyre::Result<Vec<u8>> {
        let bytes = pw::spa::pod::serialize::PodSerializer::serialize(
            Cursor::new(Vec::new()),
            &pw::spa::pod::Value::Object(object),
        )?
        .0
        .into_inner();
        Ok(bytes)
    }

    let build_enum_format = |format: pw::spa::param::video::VideoFormat,
                             modifiers: Option<&[i64]>|
     -> spa::pod::Object {
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

        if let Some(mods) = modifiers {
            if !mods.is_empty() {
                let mut modifier_prop = spa::pod::Property::new(
                    spa::param::format::FormatProperties::VideoModifier.as_raw(),
                    spa::pod::Value::Choice(spa::pod::ChoiceValue::Long(spa::utils::Choice(
                        spa::utils::ChoiceFlags::empty(),
                        spa::utils::ChoiceEnum::Enum {
                            default: mods[0],
                            alternatives: mods.to_vec(),
                        },
                    ))),
                );
                modifier_prop.flags = spa::pod::PropertyFlags::MANDATORY
                    | spa::pod::PropertyFlags::from_bits_retain(
                        spa::sys::SPA_POD_PROP_FLAG_DONT_FIXATE,
                    );
                properties.push(modifier_prop);
            }
        }

        spa::pod::Object {
            type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
            id: spa::param::ParamType::EnumFormat.as_raw(),
            properties,
        }
    };

    let formats = [
        pw::spa::param::video::VideoFormat::BGRx,
        pw::spa::param::video::VideoFormat::BGR,
        pw::spa::param::video::VideoFormat::RGBx,
        pw::spa::param::video::VideoFormat::RGB,
        pw::spa::param::video::VideoFormat::RGBA,
        pw::spa::param::video::VideoFormat::BGRA,
    ];

    let mut format_bytes = Vec::new();
    for format in formats {
        let modifiers = drm_fourcc_for_spa_format(format)
            .map(|drm| query_modifiers_for_drm_format(egl_i, egl_display, drm));

        format_bytes.push(serialize_object_to_bytes(build_enum_format(
            format,
            modifiers.as_deref(),
        ))?);
        format_bytes.push(serialize_object_to_bytes(build_enum_format(format, None))?);
    }

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

    if should_stop.load(Ordering::Acquire) {
        return Err(eyre::eyre!(
            "pipewire stream ended with error before completion"
        ));
    }

    Ok(())
}
