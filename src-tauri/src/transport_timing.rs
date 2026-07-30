use std::time::Duration;

pub(super) const STREAM_PERSIST_INTERVAL: Duration = Duration::from_secs(2);
pub(super) const RECONNECT_DELAY_POLL_INTERVAL: Duration = Duration::from_millis(100);
