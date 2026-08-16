use super::*;

const SSH_READER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) struct SshRuntime {
    pub(super) runtime_id: String,
    pub(super) profile_snapshot: String,
    pub(super) backend: SshBackendKind,
    pub(super) auth_method: AuthMethod,
    pub(super) handle: Arc<tokio::sync::Mutex<SshBackendSession>>,
    pub(super) sftp: Arc<tokio::sync::Mutex<Option<SftpBackendSession>>>,
    pub(super) jump_handles: Vec<Arc<tokio::sync::Mutex<client::Handle<PortMateSshHandler>>>>,
    pub(super) writer: Arc<tokio::sync::Mutex<SshBackendChannelWriter>>,
    pub(super) tap: broadcast::Sender<Vec<u8>>,
    pub(super) remote_forwards: Arc<Mutex<HashMap<String, TunnelForwardTarget>>>,
    pub(super) remote_forward_acceptor_started: Arc<AtomicBool>,
    pub(super) agent_forwarder_finished: Option<tokio::sync::oneshot::Receiver<()>>,
    pub(super) transport_bridge_finished: Option<tokio::sync::oneshot::Receiver<()>>,
    pub(super) closed: Arc<AtomicBool>,
    pub(super) terminal_channel_open: Arc<AtomicBool>,
    pub(super) reader_finished: tokio::sync::oneshot::Receiver<()>,
}

pub(super) struct EstablishedSshRuntime {
    pub(super) runtime_id: String,
    pub(super) runtime: SshRuntime,
    pub(super) tap: broadcast::Sender<Vec<u8>>,
    pub(super) read_half: SshBackendChannelReader,
    pub(super) auth_method: AuthMethod,
    pub(super) closed: Arc<AtomicBool>,
    pub(super) terminal_channel_open: Arc<AtomicBool>,
    pub(super) reader_finished: tokio::sync::oneshot::Sender<()>,
}

pub(super) async fn disconnect_registered_ssh_runtime(
    runtime: SshRuntime,
    reason: &str,
    jump_reason: &str,
) {
    let SshRuntime {
        runtime_id,
        backend,
        handle,
        sftp,
        jump_handles,
        writer,
        closed,
        reader_finished,
        agent_forwarder_finished,
        transport_bridge_finished,
        ..
    } = runtime;
    closed.store(true, Ordering::SeqCst);
    drop(sftp);

    if backend == SshBackendKind::Libssh {
        let reader_stopped = tokio::time::timeout(SSH_READER_SHUTDOWN_TIMEOUT, reader_finished)
            .await
            .is_ok();
        let agent_forwarder_stopped = match agent_forwarder_finished {
            Some(finished) => tokio::time::timeout(SSH_READER_SHUTDOWN_TIMEOUT, finished)
                .await
                .is_ok(),
            None => true,
        };
        let transport_bridge_stopped = match transport_bridge_finished {
            Some(finished) => tokio::time::timeout(SSH_READER_SHUTDOWN_TIMEOUT, finished)
                .await
                .is_ok(),
            None => true,
        };
        let writer_released = Arc::try_unwrap(writer).map(drop).is_ok();
        let handle_is_exclusive = Arc::strong_count(&handle) == 1;
        if reader_stopped
            && agent_forwarder_stopped
            && transport_bridge_stopped
            && writer_released
            && handle_is_exclusive
        {
            if let Some(warning) =
                request_shared_backend_disconnect_with_timeout(&handle, reason).await
            {
                eprintln!("PortMate: SSH runtime {runtime_id} disconnect warning: {warning}");
            }
        } else {
            eprintln!(
                "PortMate: skipped eager libssh disconnect for reader {runtime_id} while channel users are still shutting down"
            );
        }
        if !reader_stopped {
            eprintln!(
                "PortMate: SSH reader {runtime_id} did not finish within {}ms before libssh disconnect",
                SSH_READER_SHUTDOWN_TIMEOUT.as_millis()
            );
        }
        if !agent_forwarder_stopped {
            eprintln!(
                "PortMate: SSH agent forwarder {runtime_id} did not finish within {}ms before libssh disconnect",
                SSH_READER_SHUTDOWN_TIMEOUT.as_millis()
            );
        }
        if !transport_bridge_stopped {
            eprintln!(
                "PortMate: SSH transport bridge {runtime_id} did not finish within {}ms before libssh disconnect",
                SSH_READER_SHUTDOWN_TIMEOUT.as_millis()
            );
        }
    } else {
        drop(writer);
        if let Some(warning) = request_shared_backend_disconnect_with_timeout(&handle, reason).await {
            eprintln!("PortMate: SSH runtime {runtime_id} disconnect warning: {warning}");
        }
        if tokio::time::timeout(SSH_READER_SHUTDOWN_TIMEOUT, reader_finished)
            .await
            .is_err()
        {
            eprintln!(
                "PortMate: SSH reader {runtime_id} did not finish within {}ms after disconnect",
                SSH_READER_SHUTDOWN_TIMEOUT.as_millis()
            );
        }
    }
    drop(handle);

    for jump_handle in jump_handles {
        if let Some(warning) =
            request_shared_ssh_disconnect_with_timeout(&jump_handle, jump_reason).await
        {
            eprintln!("PortMate: SSH jump runtime {runtime_id} disconnect warning: {warning}");
        }
    }
}

pub(super) async fn disconnect_ssh_runtime(runtime: SshRuntime, reason: &str) {
    let SshRuntime {
        handle,
        jump_handles,
        closed,
        agent_forwarder_finished,
        transport_bridge_finished,
        ..
    } = runtime;
    closed.store(true, Ordering::SeqCst);
    for finished in [agent_forwarder_finished, transport_bridge_finished]
        .into_iter()
        .flatten()
    {
        let _ = tokio::time::timeout(SSH_READER_SHUTDOWN_TIMEOUT, finished).await;
    }
    if let Some(warning) = request_shared_backend_disconnect_with_timeout(&handle, reason).await {
        eprintln!("PortMate: SSH reconnect disconnect warning: {warning}");
    }
    for jump_handle in jump_handles {
        if let Some(warning) = request_shared_ssh_disconnect_with_timeout(&jump_handle, reason).await {
            eprintln!("PortMate: SSH reconnect jump disconnect warning: {warning}");
        }
    }
}
