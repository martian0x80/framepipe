#![cfg(feature = "recording")]

use std::io::{Read, Write};

use bitcode::{Decode, Encode};

#[derive(Debug, Clone, Encode, Decode)]
pub struct RecordingHeader {
    pub version: u32,
    pub metadata: Vec<u8>,
}

pub struct EventWriter<W: Write> {
    writer: W,
}

impl<W: Write> EventWriter<W> {
    pub fn new(writer: W) -> Self {
        Self { writer }
    }

    pub fn write_header(&mut self, header: &RecordingHeader) -> Result<(), std::io::Error> {
        let payload = bitcode::encode(header);
        write_framed(&mut self.writer, &payload)
    }

    pub fn write_chunk<T: Encode>(&mut self, chunk: &T) -> Result<(), std::io::Error> {
        let payload = bitcode::encode(chunk);
        write_framed(&mut self.writer, &payload)
    }
}

pub struct EventReader<R: Read> {
    reader: R,
}

impl<R: Read> EventReader<R> {
    pub fn new(reader: R) -> Self {
        Self { reader }
    }

    pub fn read_frame<T>(&mut self) -> Result<Option<T>, std::io::Error>
    where
        T: for<'de> Decode<'de>,
    {
        let mut len = [0_u8; 4];
        match self.reader.read_exact(&mut len) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
        let size = u32::from_le_bytes(len) as usize;
        let mut payload = vec![0_u8; size];
        self.reader.read_exact(&mut payload)?;
        let decoded = bitcode::decode::<T>(&payload)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        Ok(Some(decoded))
    }
}

fn write_framed<W: Write>(writer: &mut W, payload: &[u8]) -> Result<(), std::io::Error> {
    let len = payload.len() as u32;
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(payload)?;
    writer.flush()
}
