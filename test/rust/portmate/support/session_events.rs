#[cfg(test)]
pub(super) fn record_channel_bytes(
    io: &SessionIo,
    session_id: &str,
    source_runtime_id: Option<&str>,
    stream: EventStream,
    raw_bytes: &[u8],
    text: String,
) {
    record_channel_bytes_with_accepted_side_effect(
        io,
        session_id,
        source_runtime_id,
        stream,
        ChannelByteViews::same(raw_bytes),
        text,
        || {},
    );
}

#[cfg(test)]
pub(super) fn finish_inbound_log_queue(
    store_path: &Path,
    session_id: &str,
    timeout: Duration,
) -> bool {
    let queue = INBOUND_LOG_QUEUES.get().and_then(|queues| {
        queues
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&(store_path.to_path_buf(), session_id.to_string()))
    });
    let Some(queue) = queue else { return true };
    drop(queue.sender);
    queue.state.wait_finished(Instant::now() + timeout)
}
