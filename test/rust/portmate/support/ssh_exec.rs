#[cfg(all(test, unix))]
pub(super) async fn close_russh_channel_bounded(channel: &Channel<client::Msg>) {
    let _ = tokio::time::timeout(SSH_SETUP_TIMEOUT_DISCONNECT_TIMEOUT, channel.close()).await;
}
