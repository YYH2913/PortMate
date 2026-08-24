use serde::Serialize;
use crate::AppHandle;
use tauri::Emitter;

pub(super) const TERMINAL_LIVE_EVENT: &str = "portmate-terminal-live";
pub(super) const MAX_TERMINAL_BYTES_EVENT_FRAME_BYTES: usize = 64 * 1024;
const MAX_TERMINAL_LIVE_TEXT_CHARACTERS: usize = 64 * 1024;

/// Canonical live terminal packet. The event metadata and terminal bytes are
/// published together so the renderer never has to correlate two Tauri
/// channels with an arbitrary timeout. `bytes` is bounded and is never part
/// of the persisted `SessionEvent`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct TerminalLiveEvent {
    pub(super) event: portmate_core::SessionEvent,
    pub(super) bytes: Vec<u8>,
    pub(super) original_length: usize,
    pub(super) truncated: bool,
}

pub(super) fn publish_terminal_live_event(
    app_handle: Option<&AppHandle>,
    event: &portmate_core::SessionEvent,
    terminal_bytes: &[u8],
) {
    let Some(app_handle) = app_handle else {
        return;
    };
    if terminal_bytes.is_empty() {
        return;
    }
    let captured_length = terminal_bytes
        .len()
        .min(MAX_TERMINAL_BYTES_EVENT_FRAME_BYTES);
    let mut live_event = event.clone();
    if live_event
        .text
        .as_ref()
        .is_some_and(|text| text.chars().count() > MAX_TERMINAL_LIVE_TEXT_CHARACTERS)
    {
        if let Some(text) = live_event.text.as_ref() {
            live_event.text = Some(text.chars().take(MAX_TERMINAL_LIVE_TEXT_CHARACTERS).collect());
        }
        live_event
            .annotations
            .insert("liveTextTruncated".to_string(), "true".to_string());
    }
    let packet = TerminalLiveEvent {
        event: live_event,
        bytes: terminal_bytes[..captured_length].to_vec(),
        original_length: terminal_bytes.len(),
        truncated: captured_length != terminal_bytes.len(),
    };
    let _ = app_handle.emit(TERMINAL_LIVE_EVENT, packet);
}
