use std::path::Path;
use std::{fs::File, io::Read};

use crate::wayland::types::{MouseTrackChunk, MouseTrackFile, MouseTrackHeader};

fn read_framed<R: Read>(reader: &mut R) -> Result<Option<Vec<u8>>, String> {
    let mut len_buf = [0u8; 4];
    match reader.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(format!("failed reading track frame length: {e}")),
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    let mut payload = vec![0u8; len];
    reader
        .read_exact(&mut payload)
        .map_err(|e| format!("failed reading track frame payload: {e}"))?;
    Ok(Some(payload))
}

pub fn load_mouse_track(path: &Path) -> Result<MouseTrackFile, String> {
    let bytes = std::fs::read(path)
        .map_err(|e| format!("failed to read mouse track file {}: {e}", path.display()))?;
    if let Ok(single) = bitcode::decode::<MouseTrackFile>(&bytes) {
        return Ok(single);
    }

    let mut reader = File::open(path)
        .map_err(|e| format!("failed to open mouse track file {}: {e}", path.display()))?;
    let Some(header_bytes) = read_framed(&mut reader)? else {
        return Err("mouse track file is empty".to_string());
    };
    let header: MouseTrackHeader = bitcode::decode(&header_bytes)
        .map_err(|e| format!("failed to decode mouse track header: {e}"))?;
    let mut samples = Vec::new();
    while let Some(payload) = read_framed(&mut reader)? {
        let chunk: MouseTrackChunk = bitcode::decode(&payload)
            .map_err(|e| format!("failed to decode mouse track chunk: {e}"))?;
        samples.extend(chunk.samples);
    }
    Ok(MouseTrackFile {
        version: header.version,
        recording: header.recording,
        samples,
    })
}

pub fn save_mouse_track(path: &Path, track: &MouseTrackFile) -> Result<(), String> {
    let bytes = bitcode::encode(track);
    std::fs::write(path, bytes)
        .map_err(|e| format!("failed to write mouse track file {}: {e}", path.display()))
}
