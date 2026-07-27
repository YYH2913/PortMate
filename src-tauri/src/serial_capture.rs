use chrono::{DateTime, Utc};
use portmate_core::{ConnectionConfig, EventDirection, SessionStore};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use super::{
    prepare_export_directory, read_verified_log_bytes_ref, sanitize_log_path_segment,
    write_atomic_export_with_checksum, AppState, ExportSerialCaptureRequest,
    ExportSerialCaptureResult, SerialCaptureMap,
};

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

pub(super) fn ensure_serial_profile(
    store: &Arc<Mutex<SessionStore>>,
    session_id: &str,
) -> Result<(), String> {
    let store = store.lock().map_err(|error| error.to_string())?;
    match store.profile(session_id).map(|profile| profile.connection) {
        Some(ConnectionConfig::Serial(_)) => Ok(()),
        Some(_) => Err(format!("session is not serial-backed: {session_id}")),
        None => Err(format!("unknown session: {session_id}")),
    }
}

pub(super) fn serial_capture_snapshot_inner(
    state: &AppState,
    session_id: &str,
    after_id: Option<&str>,
) -> Result<SerialCaptureSnapshot, String> {
    ensure_serial_profile(&state.store, session_id)?;
    let capture = serial_capture_for_session(&state.serial_captures, session_id)?;
    let capture = capture.lock().map_err(|error| error.to_string())?;
    Ok(capture.snapshot_since(after_id))
}

pub(super) fn serial_capture_history_inner(
    store_path: &Path,
    store: &Arc<Mutex<SessionStore>>,
    session_id: &str,
) -> Result<SerialCaptureHistorySnapshot, String> {
    ensure_serial_profile(store, session_id)?;
    let (enabled, candidates) = {
        let store = store.lock().map_err(|error| error.to_string())?;
        let profile = store
            .profile(session_id)
            .ok_or_else(|| format!("unknown session: {session_id}"))?;
        let enabled = profile.logging.enabled && profile.logging.raw;
        let candidates = if enabled {
            store
                .events
                .iter()
                .filter_map(|event| {
                    let reference = event.bytes_ref.as_ref()?;
                    (event.session_id == session_id
                        && matches!(
                            event.direction,
                            EventDirection::Inbound | EventDirection::Outbound
                        ))
                    .then(|| {
                        (
                            event.id.clone(),
                            event.ts,
                            event.direction,
                            reference.clone(),
                        )
                    })
                })
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        (enabled, candidates)
    };
    if !enabled {
        return Ok(SerialCaptureHistorySnapshot {
            frames: Vec::new(),
            enabled: false,
            total_frames: 0,
            captured_bytes: 0,
            dropped_frames: 0,
            unavailable_frames: 0,
        });
    }

    let total_frames = candidates.len();
    let mut frames = Vec::new();
    let mut captured_bytes = 0_usize;
    let mut unavailable_frames = 0_usize;
    for (id, ts, direction, reference) in candidates.into_iter().rev() {
        if frames.len() >= MAX_SERIAL_CAPTURE_HISTORY_FRAMES {
            break;
        }
        let Ok((relative, _, bytes)) = read_verified_log_bytes_ref(store_path, &reference) else {
            unavailable_frames = unavailable_frames.saturating_add(1);
            continue;
        };
        if Path::new(&relative)
            .extension()
            .and_then(|value| value.to_str())
            != Some("raw")
            || bytes.is_empty()
        {
            unavailable_frames = unavailable_frames.saturating_add(1);
            continue;
        }
        let original_length = bytes.len();
        let captured_length = original_length.min(MAX_SERIAL_CAPTURE_FRAME_BYTES);
        if captured_bytes.saturating_add(captured_length) > MAX_SERIAL_CAPTURE_HISTORY_BYTES {
            break;
        }
        captured_bytes = captured_bytes.saturating_add(captured_length);
        frames.push(SerialCaptureFrame {
            id,
            ts,
            direction,
            bytes: bytes[..captured_length].to_vec(),
            original_length,
            truncated: captured_length != original_length,
        });
    }
    frames.reverse();
    Ok(SerialCaptureHistorySnapshot {
        dropped_frames: total_frames
            .saturating_sub(frames.len())
            .saturating_sub(unavailable_frames),
        frames,
        enabled: true,
        total_frames,
        captured_bytes,
        unavailable_frames,
    })
}

fn serial_capture_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn serial_capture_ascii(bytes: &[u8]) -> String {
    let mut preview = String::new();
    for byte in bytes {
        match byte {
            b'\r' => preview.push_str("\\r"),
            b'\n' => preview.push_str("\\n"),
            b'\t' => preview.push_str("\\t"),
            0x20..=0x7e => preview.push(char::from(*byte)),
            _ => preview.push('.'),
        }
    }
    preview
}

pub(super) fn export_serial_capture_inner(
    store_path: &Path,
    captures: &SerialCaptureMap,
    request: ExportSerialCaptureRequest,
) -> Result<ExportSerialCaptureResult, String> {
    let requested = serial_capture_export_ids(&request, MAX_SERIAL_CAPTURE_FRAMES)?;
    let capture = {
        let captures = captures.lock().map_err(|error| error.to_string())?;
        captures
            .get(&request.session_id)
            .map(Arc::clone)
            .ok_or_else(|| "serial capture is empty".to_string())?
    };
    let selected = {
        let capture = capture.lock().map_err(|error| error.to_string())?;
        capture
            .frames
            .iter()
            .filter(|frame| requested.contains(&frame.id))
            .cloned()
            .collect::<Vec<_>>()
    };
    if selected.len() != requested.len() {
        return Err("serial capture changed; refresh before exporting".to_string());
    }

    export_serial_capture_frames(store_path, &request.session_id, "live", &selected)
}

pub(super) fn export_serial_capture_history_inner(
    store_path: &Path,
    store: &Arc<Mutex<SessionStore>>,
    request: ExportSerialCaptureRequest,
) -> Result<ExportSerialCaptureResult, String> {
    let requested = serial_capture_export_ids(&request, MAX_SERIAL_CAPTURE_HISTORY_FRAMES)?;
    let history = serial_capture_history_inner(store_path, store, &request.session_id)?;
    if !history.enabled {
        return Err("Raw logging is not enabled for this serial profile".to_string());
    }
    let selected = history
        .frames
        .into_iter()
        .filter(|frame| requested.contains(&frame.id))
        .collect::<Vec<_>>();
    if selected.len() != requested.len() {
        return Err("serial capture history changed; refresh before exporting".to_string());
    }
    export_serial_capture_frames(store_path, &request.session_id, "raw-log", &selected)
}

fn serial_capture_export_ids(
    request: &ExportSerialCaptureRequest,
    maximum: usize,
) -> Result<HashSet<String>, String> {
    if request.frame_ids.is_empty() {
        return Err("select at least one serial capture frame to export".to_string());
    }
    if request.frame_ids.len() > maximum {
        return Err(format!(
            "serial capture export frame limit exceeded ({maximum})"
        ));
    }
    let requested = request.frame_ids.iter().cloned().collect::<HashSet<_>>();
    if requested.len() != request.frame_ids.len() {
        return Err("serial capture export contains duplicate frame IDs".to_string());
    }
    Ok(requested)
}

fn export_serial_capture_frames(
    store_path: &Path,
    session_id: &str,
    source: &str,
    selected: &[SerialCaptureFrame],
) -> Result<ExportSerialCaptureResult, String> {
    let created_at = Utc::now();
    let mut output = Vec::new();
    serde_json::to_writer(
        &mut output,
        &serde_json::json!({
            "type": "metadata",
            "format": "portmate-serial-capture",
            "version": 1,
            "createdAt": created_at.to_rfc3339(),
            "sessionId": session_id,
            "source": source,
            "rawUnredacted": true,
            "frameCount": selected.len(),
        }),
    )
    .map_err(|error| format!("failed to encode serial capture metadata: {error}"))?;
    output.push(b'\n');
    let mut captured_bytes = 0_usize;
    let mut truncated_frames = 0_usize;
    for frame in selected {
        captured_bytes = captured_bytes.saturating_add(frame.bytes.len());
        truncated_frames += usize::from(frame.truncated);
        serde_json::to_writer(
            &mut output,
            &serde_json::json!({
                "type": "frame",
                "id": frame.id,
                "ts": frame.ts,
                "direction": frame.direction,
                "capturedLength": frame.bytes.len(),
                "originalLength": frame.original_length,
                "truncated": frame.truncated,
                "hex": serial_capture_hex(&frame.bytes),
                "ascii": serial_capture_ascii(&frame.bytes),
            }),
        )
        .map_err(|error| format!("failed to encode serial capture frame: {error}"))?;
        output.push(b'\n');
    }

    let export_dir = prepare_export_directory(store_path, "serial capture")?;
    let timestamp = created_at.format("%Y%m%dT%H%M%SZ");
    let name = format!(
        "{}-{timestamp}-serial-{}.jsonl",
        sanitize_log_path_segment(session_id),
        &Uuid::new_v4().simple().to_string()[..8]
    );
    let final_path = export_dir.join(name);
    let finalized =
        write_atomic_export_with_checksum(&final_path, &output, "serial capture export")?;
    Ok(ExportSerialCaptureResult {
        path: final_path.display().to_string(),
        checksum_path: finalized.checksum_path.display().to_string(),
        sha256: finalized.sha256,
        size: finalized.size,
        frames: selected.len(),
        captured_bytes,
        truncated_frames,
    })
}
