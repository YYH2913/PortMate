use std::time::Duration;

pub(super) const STREAM_PERSIST_INTERVAL: Duration = Duration::from_secs(2);
pub(super) const RECONNECT_DELAY_POLL_INTERVAL: Duration = Duration::from_millis(100);
pub(super) const SERIAL_RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
// Manual close/reconnect gets a longer grace period than process shutdown.
// USB serial drivers can take several scheduler ticks to release duplicated
// COM handles after an aborted read/write, especially on Windows.
pub(super) const SERIAL_RUNTIME_CLOSE_TIMEOUT: Duration = Duration::from_secs(5);
pub(super) const TCP_RUNTIME_WRITE_TIMEOUT: Duration = Duration::from_secs(10);
pub(super) const TCP_RUNTIME_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(1);
