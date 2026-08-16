use std::time::Duration;

pub(super) const STREAM_PERSIST_INTERVAL: Duration = Duration::from_secs(2);
pub(super) const RECONNECT_DELAY_POLL_INTERVAL: Duration = Duration::from_millis(100);
pub(super) const TCP_RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
