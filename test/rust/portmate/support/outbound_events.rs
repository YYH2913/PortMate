/// Enqueues desktop input without making the webview wait for the transport
/// writer. Printable input may coalesce; control keys and paste requests are
/// explicit ordering barriers in the same per-session queue.
#[cfg(test)]
pub(super) fn enqueue_interactive_text(
    io: SessionIo,
    session_id: String,
    text: String,
    coalesce: bool,
) -> Result<(), String> {
    enqueue_interactive_text_with_sensitivity(io, session_id, text, coalesce, false)
}

/// Enqueue an atomic payload and wait until the per-session writer has
/// completed the transport write. The regular keyboard path intentionally
/// remains fire-and-forget; this acknowledgement is used by paced senders
/// that must measure their interval from an actual write rather than from
/// queue admission.
#[cfg(test)]
pub(super) async fn enqueue_interactive_text_and_wait(
    io: SessionIo,
    session_id: String,
    text: String,
    coalesce: bool,
) -> Result<(), String> {
    enqueue_interactive_text_and_wait_with_sensitivity(io, session_id, text, coalesce, false).await
}

#[cfg(test)]
pub(super) async fn enqueue_interactive_text_and_wait_with_timeout(
    io: SessionIo,
    session_id: String,
    text: String,
    coalesce: bool,
    timeout: Duration,
) -> Result<(), String> {
    enqueue_interactive_text_and_wait_with_timeout_and_sensitivity(
        io, session_id, text, coalesce, false, timeout,
    )
    .await
}
