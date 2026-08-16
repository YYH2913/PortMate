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
        // libssh requires the session to outlive every channel and SFTP handle. A
        // detached blocking worker can retain one without affecting the runtime's Arcs,
        // so an eager ssh_disconnect here would race its eventual destructor. Dropping
        // our handles lets SessionHolder close the socket after the final user exits.
        drop(writer);
        if !reader_stopped {
            eprintln!(
                "PortMate: SSH reader {runtime_id} did not finish within {}ms before libssh handle release",
                SSH_READER_SHUTDOWN_TIMEOUT.as_millis()
            );
        }
        if !agent_forwarder_stopped {
            eprintln!(
                "PortMate: SSH agent forwarder {runtime_id} did not finish within {}ms before libssh handle release",
                SSH_READER_SHUTDOWN_TIMEOUT.as_millis()
            );
        }
        if !transport_bridge_stopped {
            eprintln!(
                "PortMate: SSH transport bridge {runtime_id} did not finish within {}ms before libssh handle release",
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

pub(super) async fn disconnect_ssh_runtime(
    runtime: SshRuntime,
    read_half: SshBackendChannelReader,
    reader_finished: tokio::sync::oneshot::Sender<()>,
    reason: &str,
) {
    // An uninstalled runtime has no reader task to own these values. libssh requires
    // every channel to be freed before its session is disconnected.
    drop(read_half);
    drop(reader_finished);
    disconnect_registered_ssh_runtime(runtime, reason, reason).await;
}
