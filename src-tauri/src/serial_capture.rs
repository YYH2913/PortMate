use chrono::{DateTime, Utc};
use portmate_core::EventDirection;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

pub const MAX_SERIAL_CAPTURE_FRAMES: usize = 512;
pub const MAX_SERIAL_CAPTURE_BYTES: usize = 1024 * 1024;
pub const MAX_SERIAL_CAPTURE_FRAME_BYTES: usize = 64 * 1024;
pub const MAX_SERIAL_CAPTURE_HISTORY_FRAMES: usize = 4_096;
pub const MAX_SERIAL_CAPTURE_HISTORY_BYTES: usize = 8 * 1024 * 1024;

pub type SerialCaptureRegistry = Mutex<HashMap<String, Arc<Mutex<SerialCaptureBuffer>>>>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialCaptureFrame {
    pub id: String,
    pub ts: DateTime<Utc>,
    pub direction: EventDirection,
    pub bytes: Vec<u8>,
    pub original_length: usize,
    pub truncated: bool,
}

#[derive(Debug, Default)]
pub struct SerialCaptureBuffer {
    pub frames: VecDeque<SerialCaptureFrame>,
    pub captured_bytes: usize,
}

impl SerialCaptureBuffer {
    pub fn push(&mut self, direction: EventDirection, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let captured_length = bytes.len().min(MAX_SERIAL_CAPTURE_FRAME_BYTES);
        let frame = SerialCaptureFrame {
            id: Uuid::new_v4().to_string(),
            ts: Utc::now(),
            direction,
            bytes: bytes[..captured_length].to_vec(),
            original_length: bytes.len(),
            truncated: captured_length != bytes.len(),
        };
        while self.frames.len() >= MAX_SERIAL_CAPTURE_FRAMES
            || self.captured_bytes.saturating_add(captured_length) > MAX_SERIAL_CAPTURE_BYTES
        {
            let Some(removed) = self.frames.pop_front() else {
                break;
            };
            self.captured_bytes = self.captured_bytes.saturating_sub(removed.bytes.len());
        }
        self.captured_bytes = self.captured_bytes.saturating_add(captured_length);
        self.frames.push_back(frame);
    }

    pub fn clear(&mut self) {
        self.frames.clear();
        self.captured_bytes = 0;
    }

    pub fn snapshot_since(&self, after_id: Option<&str>) -> SerialCaptureSnapshot {
        let start = after_id
            .and_then(|id| self.frames.iter().position(|frame| frame.id == id))
            .map(|index| index + 1);
        let reset = after_id.is_none() || start.is_none();
        let frames = if reset {
            self.frames.iter().cloned().collect()
        } else {
            self.frames
                .iter()
                .skip(start.unwrap_or_default())
                .cloned()
                .collect()
        };
        SerialCaptureSnapshot {
            frames,
            reset,
            total_frames: self.frames.len(),
            captured_bytes: self.captured_bytes,
        }
    }
}

pub fn serial_capture_for_session(
    captures: &SerialCaptureRegistry,
    session_id: &str,
) -> Result<Arc<Mutex<SerialCaptureBuffer>>, String> {
    let mut captures = captures.lock().map_err(|error| error.to_string())?;
    Ok(Arc::clone(
        captures
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(SerialCaptureBuffer::default()))),
    ))
}

pub fn record_serial_capture(
    capture: &Arc<Mutex<SerialCaptureBuffer>>,
    direction: EventDirection,
    bytes: &[u8],
) {
    if let Ok(mut capture) = capture.lock() {
        capture.push(direction, bytes);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialCaptureSnapshot {
    pub frames: Vec<SerialCaptureFrame>,
    pub reset: bool,
    pub total_frames: usize,
    pub captured_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SerialCaptureHistorySnapshot {
    pub frames: Vec<SerialCaptureFrame>,
    pub enabled: bool,
    pub total_frames: usize,
    pub captured_bytes: usize,
    pub dropped_frames: usize,
    pub unavailable_frames: usize,
}
