use chrono::{DateTime, Utc};
use portmate_core::{EventDirection, EventStream};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use uuid::Uuid;

pub(super) const TERMINAL_BYTES_EVENT: &str = "portmate-terminal-bytes";
pub(super) const MAX_TERMINAL_BYTES_EVENT_FRAME_BYTES: usize = 64 * 1024;

/// A short-lived wire-byte frame for the terminal byte inspector.
///
/// These payloads deliberately bypass the persisted session event model. Raw bytes can contain
/// binary protocol data or secrets, so the frontend keeps only a bounded in-memory window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TerminalBytesEvent {
    pub(super) id: String,
    pub(super) session_id: String,
    pub(super) ts: DateTime<Utc>,
    pub(super) direction: EventDirection,
    pub(super) stream: EventStream,
    pub(super) bytes: Vec<u8>,
    pub(super) original_length: usize,
    pub(super) truncated: bool,
    pub(super) event_id: Option<String>,
}

pub(super) fn terminal_bytes_event(
    session_id: &str,
    direction: EventDirection,
    stream: EventStream,
    raw_bytes: &[u8],
    event_id: Option<&str>,
    ts: DateTime<Utc>,
) -> Option<TerminalBytesEvent> {
    if raw_bytes.is_empty() {
        return None;
    }
    let captured_length = raw_bytes.len().min(MAX_TERMINAL_BYTES_EVENT_FRAME_BYTES);
    Some(TerminalBytesEvent {
        id: Uuid::new_v4().to_string(),
        session_id: session_id.to_string(),
        ts,
        direction,
        stream,
        bytes: raw_bytes[..captured_length].to_vec(),
        original_length: raw_bytes.len(),
        truncated: captured_length != raw_bytes.len(),
        event_id: event_id.map(str::to_string),
    })
}

pub(super) fn publish_terminal_bytes(
    app_handle: Option<&AppHandle>,
    session_id: &str,
    direction: EventDirection,
    stream: EventStream,
    raw_bytes: &[u8],
    event_id: Option<&str>,
    ts: DateTime<Utc>,
) {
    let Some(app_handle) = app_handle else {
        return;
    };
    let Some(event) = terminal_bytes_event(session_id, direction, stream, raw_bytes, event_id, ts)
    else {
        return;
    };
    let _ = app_handle.emit(TERMINAL_BYTES_EVENT, event);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_real_bytes_and_associated_session_event_id() {
        let timestamp = Utc::now();
        let event = terminal_bytes_event(
            "session-a",
            EventDirection::Inbound,
            EventStream::Stdout,
            &[0x00, 0x80, b'A', 0xff],
            Some("event-a"),
            timestamp,
        )
        .expect("nonempty bytes create a live frame");

        assert_eq!(event.session_id, "session-a");
        assert_eq!(event.bytes, vec![0x00, 0x80, b'A', 0xff]);
        assert_eq!(event.original_length, 4);
        assert!(!event.truncated);
        assert_eq!(event.event_id.as_deref(), Some("event-a"));
        assert_eq!(event.ts, timestamp);
    }

    #[test]
    fn empty_frames_are_not_published() {
        assert!(terminal_bytes_event(
            "session-a",
            EventDirection::Outbound,
            EventStream::Control,
            &[],
            None,
            Utc::now(),
        )
        .is_none());
    }

    #[test]
    fn bounds_an_oversized_live_frame_without_reencoding_it() {
        let bytes = (0..MAX_TERMINAL_BYTES_EVENT_FRAME_BYTES + 17)
            .map(|index| (index % 251) as u8)
            .collect::<Vec<_>>();
        let event = terminal_bytes_event(
            "session-a",
            EventDirection::Inbound,
            EventStream::Stderr,
            &bytes,
            None,
            Utc::now(),
        )
        .expect("nonempty bytes create a live frame");

        assert_eq!(event.original_length, bytes.len());
        assert_eq!(event.bytes.len(), MAX_TERMINAL_BYTES_EVENT_FRAME_BYTES);
        assert_eq!(event.bytes, bytes[..MAX_TERMINAL_BYTES_EVENT_FRAME_BYTES]);
        assert!(event.truncated);
    }
}
