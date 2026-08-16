use super::*;

fn run_libssh_channel_operation<T>(
    channel: &libssh_rs::Channel,
    deadline: Instant,
    label: &str,
    operation: impl FnOnce(&libssh_rs::Channel) -> Result<T, String>,
) -> Result<T, String> {
    channel
        .set_session_timeout_until(deadline)
        .map_err(|error| format!("{label} libssh deadline setup failed: {error}"))?;
    let result = operation(channel);
    let restored = channel
        .set_session_timeout(SSH_RUNTIME_OPERATION_TIMEOUT)
        .map_err(|error| format!("{label} libssh runtime timeout restore failed: {error}"));
    match (result, restored) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Err(restore_error)) => Err(format!("{error}; {restore_error}")),
    }
}

pub(super) async fn run_libssh_channel_operation_with_timeout<T, F>(
    channel: Arc<tokio::sync::Mutex<libssh_rs::Channel>>,
    timeout: Duration,
    label: &str,
    operation: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(&libssh_rs::Channel) -> Result<T, String> + Send + 'static,
{
    let started = Instant::now();
    let deadline = started
        .checked_add(timeout)
        .ok_or_else(|| format!("{label} deadline is outside the supported range"))?;
    let channel = tokio::time::timeout(timeout, channel.lock_owned())
        .await
        .map_err(|_| format!("{label} channel lock timed out after {} ms", timeout.as_millis()))?;
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| format!("{label} timed out after {} ms", timeout.as_millis()))?;
    let worker_label = label.to_string();
    let worker = tokio::task::spawn_blocking(move || {
        run_libssh_channel_operation(&channel, deadline, &worker_label, operation)
    });
    tokio::time::timeout(remaining, worker)
        .await
        .map_err(|_| format!("{label} timed out after {} ms", timeout.as_millis()))?
        .map_err(|error| format!("{label} worker failed: {error}"))?
}

pub(super) struct LibsshChannelReader {
    channel: Arc<tokio::sync::Mutex<libssh_rs::Channel>>,
    pending: VecDeque<SshBackendMessage>,
    collect_exit_metadata: bool,
    completed: bool,
}

impl LibsshChannelReader {
    pub(super) fn new(channel: libssh_rs::Channel, collect_exit_metadata: bool) -> Self {
        Self {
            channel: Arc::new(tokio::sync::Mutex::new(channel)),
            pending: VecDeque::new(),
            collect_exit_metadata,
            completed: false,
        }
    }

    pub(super) async fn wait(&mut self) -> Option<SshBackendMessage> {
        self.wait_inner(None).await
    }

    pub(super) async fn wait_until_closed(
        &mut self,
        closed: &AtomicBool,
    ) -> Option<SshBackendMessage> {
        self.wait_inner(Some(closed)).await
    }

    pub(super) fn shared_channel(&self) -> Arc<tokio::sync::Mutex<libssh_rs::Channel>> {
        Arc::clone(&self.channel)
    }

    pub(super) async fn close_with_timeout(&self, timeout: Duration) -> Result<(), String> {
        run_libssh_channel_operation_with_timeout(
            Arc::clone(&self.channel),
            timeout,
            "libssh close",
            |channel| channel.close().map_err(|error| error.to_string()),
        )
        .await
    }

    async fn wait_inner(&mut self, closed: Option<&AtomicBool>) -> Option<SshBackendMessage> {
        loop {
            if closed.is_some_and(|closed| closed.load(Ordering::SeqCst)) {
                return None;
            }
            if let Some(message) = self.pending.pop_front() {
                return Some(message);
            }
            if self.completed {
                return None;
            }

            let channel = Arc::clone(&self.channel);
            let collect_exit_metadata = self.collect_exit_metadata;
            let polled = tokio::task::spawn_blocking(move || {
                poll_libssh_channel(channel, collect_exit_metadata)
            })
            .await
            .map_err(|error| format!("libssh read worker failed: {error}"))
            .and_then(|result| result);
            match polled {
                Ok(LibsshChannelPoll::Data(data)) => return Some(SshBackendMessage::Data(data)),
                Ok(LibsshChannelPoll::ExtendedData(data)) => {
                    return Some(SshBackendMessage::ExtendedData { data, ext: 1 });
                }
                Ok(LibsshChannelPoll::Pending) => continue,
                Ok(LibsshChannelPoll::Finished {
                    exit_status,
                    exit_signal,
                    closed,
                }) => {
                    if let Some(signal) = exit_signal {
                        self.pending.push_back(SshBackendMessage::ExitSignal {
                            signal_name: signal
                                .signal_name
                                .unwrap_or_else(|| "unknown".to_string()),
                            error_message: signal.error_message.unwrap_or_default(),
                        });
                    }
                    if let Some(status) = exit_status.and_then(|status| u32::try_from(status).ok())
                    {
                        self.pending
                            .push_back(SshBackendMessage::ExitStatus(status));
                    }
                    self.pending.push_back(if closed {
                        SshBackendMessage::Close
                    } else {
                        SshBackendMessage::Eof
                    });
                    self.completed = true;
                }
                Err(error) => {
                    self.completed = true;
                    return Some(SshBackendMessage::Error(error));
                }
            }
        }
    }
}

enum LibsshChannelPoll {
    Data(Vec<u8>),
    ExtendedData(Vec<u8>),
    Pending,
    Finished {
        exit_status: Option<i32>,
        exit_signal: Option<libssh_rs::SignalState>,
        closed: bool,
    },
}

fn poll_libssh_channel(
    channel: Arc<tokio::sync::Mutex<libssh_rs::Channel>>,
    collect_exit_metadata: bool,
) -> Result<LibsshChannelPoll, String> {
    const POLL_TIMEOUT: Duration = Duration::from_millis(50);
    const MAX_READ_BYTES: usize = 64 * 1024;

    let channel = channel.blocking_lock();
    for (is_stderr, timeout) in [(false, Some(POLL_TIMEOUT)), (true, Some(Duration::ZERO))] {
        match channel
            .poll_timeout(is_stderr, timeout)
            .map_err(|error| error.to_string())?
        {
            libssh_rs::PollStatus::AvailableBytes(available) if available > 0 => {
                let mut data = vec![0_u8; (available as usize).min(MAX_READ_BYTES)];
                let read = channel
                    .read_timeout(&mut data, is_stderr, Some(POLL_TIMEOUT))
                    .map_err(|error| error.to_string())?;
                if read == 0 {
                    continue;
                }
                data.truncate(read);
                return Ok(if is_stderr {
                    LibsshChannelPoll::ExtendedData(data)
                } else {
                    LibsshChannelPoll::Data(data)
                });
            }
            libssh_rs::PollStatus::AvailableBytes(_) | libssh_rs::PollStatus::EndOfFile => {}
        }
    }

    let closed = channel.is_closed();
    if closed || channel.is_eof() {
        return Ok(LibsshChannelPoll::Finished {
            exit_status: collect_exit_metadata
                .then(|| channel.get_exit_status())
                .flatten(),
            exit_signal: collect_exit_metadata
                .then(|| channel.get_exit_signal())
                .flatten(),
            closed,
        });
    }
    Ok(LibsshChannelPoll::Pending)
}
