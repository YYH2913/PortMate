#[cfg(all(test, unix))]
pub(super) async fn write_ssh_channel_bytes_with_timeout(
    writer: &Arc<tokio::sync::Mutex<SshBackendChannelWriter>>,
    data: &[u8],
    timeout: Duration,
    label: &str,
) -> Result<(), String> {
    write_ssh_channel_bytes_with_cancellation(writer, data, timeout, label, None).await
}
