#[cfg(test)]
pub(super) async fn write_runtime_bytes(
    state: &AppState,
    session_id: &str,
    bytes: &[u8],
) -> Result<(), String> {
    write_runtime_bytes_for_runtime(state, session_id, bytes, None).await
}
