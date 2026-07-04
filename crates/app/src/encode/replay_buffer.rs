use std::{collections::VecDeque, path::Path};

use gstreamer::{self as gst, prelude::*};
use gstreamer_app as gst_app;

use super::EncodeError;
use crate::drm_kms::types::{OutputContainer, VideoCodec};

#[derive(Clone)]
struct ReplayPacket {
    // one per sample/packet, allocation overhead, reminder to consider fixing this
    data: Vec<u8>,
    pts_ns: Option<u64>,
    dts_ns: Option<u64>,
    duration_ns: Option<u64>,
    delta_unit: bool,
}

#[derive(Clone)]
pub struct ReplaySnapshot {
    caps: gst::Caps,
    packets: Vec<ReplayPacket>,
}

pub struct ReplayBuffer {
    max_ns: u64,
    codec: VideoCodec,
    output_container: OutputContainer,
    caps: Option<gst::Caps>,
    packets: VecDeque<ReplayPacket>,
}

impl ReplayBuffer {
    pub fn new(seconds: u32, codec: VideoCodec, output_container: OutputContainer) -> Self {
        Self {
            max_ns: seconds.max(1) as u64 * 1_000_000_000,
            codec,
            output_container,
            caps: None,
            packets: VecDeque::new(),
        }
    }

    pub fn push_sample(&mut self, sample: &gst::Sample) -> Result<(), EncodeError> {
        if let Some(caps) = sample.caps() {
            self.caps = Some(caps.to_owned());
        }

        let buffer = sample
            .buffer()
            .ok_or_else(|| EncodeError::Replay("appsink sample missing buffer".to_string()))?;
        let map = buffer
            .map_readable()
            .map_err(|_| EncodeError::Replay("failed to map encoded replay buffer".to_string()))?;

        self.packets.push_back(ReplayPacket {
            data: map.as_slice().to_vec(),
            pts_ns: buffer.pts().map(|t| t.nseconds()),
            dts_ns: buffer.dts().map(|t| t.nseconds()),
            duration_ns: buffer.duration().map(|t| t.nseconds()),
            delta_unit: buffer.flags().contains(gst::BufferFlags::DELTA_UNIT),
        });
        self.evict_old();
        Ok(())
    }

    pub fn snapshot(&self) -> Result<ReplaySnapshot, EncodeError> {
        let caps = self.caps.clone().ok_or_else(|| {
            EncodeError::Replay("replay buffer has no negotiated caps".to_string())
        })?;
        let newest = self
            .packets
            .back()
            .and_then(|p| p.pts_ns)
            .ok_or_else(|| EncodeError::Replay("replay buffer is empty".to_string()))?;
        let cutoff = newest.saturating_sub(self.max_ns);

        let start = self
            .packets
            .iter()
            .enumerate()
            .filter(|(_, p)| p.pts_ns.unwrap_or(0) >= cutoff && !p.delta_unit)
            .map(|(idx, _)| idx)
            .next()
            .ok_or_else(|| {
                EncodeError::Replay("no keyframe available in replay window".to_string())
            })?;

        Ok(ReplaySnapshot {
            caps,
            packets: self.packets.iter().skip(start).cloned().collect(),
        })
    }

    pub fn save_snapshot(&self, path: &Path) -> Result<(), EncodeError> {
        let snapshot = self.snapshot()?;
        save_snapshot(&snapshot, &self.codec, self.output_container, path)
    }

    fn evict_old(&mut self) {
        let Some(newest) = self.packets.back().and_then(|p| p.pts_ns) else {
            return;
        };
        let cutoff = newest.saturating_sub(self.max_ns);
        while self.packets.len() > 1
            && self
                .packets
                .front()
                .and_then(|p| p.pts_ns)
                .is_some_and(|pts| pts < cutoff)
        {
            self.packets.pop_front();
        }
    }
}

fn save_snapshot(
    snapshot: &ReplaySnapshot,
    codec: &VideoCodec,
    output_container: OutputContainer,
    path: &Path,
) -> Result<(), EncodeError> {
    let parser = match codec {
        VideoCodec::H264 => "h264parse config-interval=-1",
        VideoCodec::H265 => "h265parse config-interval=-1",
        VideoCodec::Av1 => "av1parse",
    };
    let desc = format!(
        "appsrc name=src format=time is-live=false do-timestamp=false block=true ! {parser} {}! filesink location={}",
        output_container.mux_chain(),
        path.to_string_lossy(),
    );
    let element = gst::parse::launch(&desc)?;
    let pipeline = element
        .downcast::<gst::Pipeline>()
        .map_err(|_| gst::glib::bool_error!("parsed replay saver is not a pipeline"))?;
    let appsrc = pipeline
        .by_name("src")
        .ok_or(EncodeError::MissingAppSrc)?
        .downcast::<gst_app::AppSrc>()
        .map_err(|_| EncodeError::MissingAppSrc)?;

    appsrc.set_caps(Some(&snapshot.caps));
    appsrc.set_format(gst::Format::Time);

    pipeline
        .set_state(gst::State::Playing)
        .map_err(|e| EncodeError::Bus(format!("failed to start replay saver: {e:?}")))?;

    let base_pts = snapshot.packets.first().and_then(|p| p.pts_ns).unwrap_or(0);
    for packet in &snapshot.packets {
        let mut buffer = gst::Buffer::from_mut_slice(packet.data.clone());
        if let Some(buf) = buffer.get_mut() {
            if let Some(pts) = packet.pts_ns {
                buf.set_pts(gst::ClockTime::from_nseconds(pts.saturating_sub(base_pts)));
            }
            if let Some(dts) = packet.dts_ns {
                buf.set_dts(gst::ClockTime::from_nseconds(dts.saturating_sub(base_pts)));
            }
            if let Some(duration) = packet.duration_ns {
                buf.set_duration(gst::ClockTime::from_nseconds(duration));
            }
            if packet.delta_unit {
                buf.set_flags(gst::BufferFlags::DELTA_UNIT);
            }
        }
        appsrc
            .push_buffer(buffer)
            .map_err(|e| EncodeError::Replay(format!("failed to push replay packet: {e}")))?;
    }
    appsrc
        .end_of_stream()
        .map_err(|e| EncodeError::Replay(format!("failed to end replay stream: {e:?}")))?;

    let bus = pipeline
        .bus()
        .ok_or_else(|| EncodeError::Bus("replay saver has no bus".to_string()))?;
    loop {
        match bus.timed_pop(gst::ClockTime::from_seconds(10)) {
            Some(msg) => match msg.view() {
                gst::MessageView::Eos(..) => break,
                gst::MessageView::Error(err) => {
                    return Err(EncodeError::Bus(format!(
                        "{} ({:?})",
                        err.error(),
                        err.debug()
                    )));
                }
                _ => {}
            },
            None => return Err(EncodeError::Bus("timeout saving replay".to_string())),
        }
    }

    pipeline
        .set_state(gst::State::Null)
        .map_err(|e| EncodeError::Bus(format!("failed to stop replay saver: {e:?}")))?;
    Ok(())
}
