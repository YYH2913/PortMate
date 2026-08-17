use super::*;

pub(super) const MAX_ACTIVE_TUNNELS: usize = 256;
pub(super) const MAX_TUNNEL_CONNECTIONS: usize = 256;
pub(super) const TUNNEL_CONNECTION_LIMIT_ERROR_PREFIX: &str = "tunnel connection limit reached:";
pub(super) const TUNNEL_LISTENER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const MCP_HOST_ROUTE_OWNER_PREFIX: &str = "\0mcp-host:";

#[derive(Clone)]
pub(super) struct TunnelListenerWorker {
    shutdown: Arc<tokio::sync::Notify>,
    finished: tokio::sync::watch::Receiver<bool>,
}

pub(super) struct TunnelListenerCompletion {
    finished: tokio::sync::watch::Sender<bool>,
}

impl TunnelListenerWorker {
    pub(super) fn running() -> (Self, TunnelListenerCompletion) {
        let (finished, receiver) = tokio::sync::watch::channel(false);
        (
            Self {
                shutdown: Arc::new(tokio::sync::Notify::new()),
                finished: receiver,
            },
            TunnelListenerCompletion { finished },
        )
    }

    pub(super) fn completed() -> Self {
        let (finished, receiver) = tokio::sync::watch::channel(true);
        drop(finished);
        Self {
            shutdown: Arc::new(tokio::sync::Notify::new()),
            finished: receiver,
        }
    }

    pub(super) fn request_shutdown(&self) {
        self.shutdown.notify_one();
    }

    pub(super) async fn wait_shutdown(&self) {
        self.shutdown.notified().await;
    }

    pub(super) fn is_finished(&self) -> bool {
        *self.finished.borrow()
    }

    pub(super) async fn wait_finished(&self) {
        let mut finished = self.finished.clone();
        while !*finished.borrow() {
            if finished.changed().await.is_err() {
                break;
            }
        }
    }
}

impl Drop for TunnelListenerCompletion {
    fn drop(&mut self) {
        let _ = self.finished.send(true);
    }
}

#[derive(Clone)]
pub(super) struct TunnelRuntime {
    pub(super) session_id: String,
    pub(super) ssh_runtime_id: String,
    pub(super) spec: TunnelSpec,
    pub(super) metrics: Arc<TunnelMetrics>,
    pub(super) closed: Arc<AtomicBool>,
    pub(super) listener_worker: TunnelListenerWorker,
}

#[derive(Clone)]
pub(super) struct TunnelRuntimeOwner {
    pub(super) ssh_runtime_id: String,
    pub(super) closed: Arc<AtomicBool>,
}

impl TunnelRuntime {
    pub(super) fn owner(&self) -> TunnelRuntimeOwner {
        TunnelRuntimeOwner {
            ssh_runtime_id: self.ssh_runtime_id.clone(),
            closed: Arc::clone(&self.closed),
        }
    }

    pub(super) fn request_shutdown(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.listener_worker.request_shutdown();
    }
}

impl TunnelRuntimeOwner {
    pub(super) fn owns(&self, runtime: &TunnelRuntime) -> bool {
        runtime.ssh_runtime_id == self.ssh_runtime_id
            && Arc::ptr_eq(&runtime.closed, &self.closed)
    }
}

#[derive(Clone, Debug)]
pub(super) struct TunnelForwardTarget {
    pub(super) spec: TunnelSpec,
    pub(super) metrics: Arc<TunnelMetrics>,
    pub(super) connection_slots: Arc<tokio::sync::Semaphore>,
}

#[derive(Debug, Default)]
pub(super) struct TunnelMetrics {
    pub(super) active_connections: AtomicU64,
    pub(super) total_connections: AtomicU64,
    pub(super) tcp_to_ssh_bytes: AtomicU64,
    pub(super) ssh_to_tcp_bytes: AtomicU64,
    pub(super) last_activity: Mutex<Option<String>>,
    pub(super) last_error: Mutex<Option<String>>,
}

impl TunnelMetrics {
    pub(super) fn connection_opened(&self) {
        self.total_connections.fetch_add(1, Ordering::SeqCst);
        self.active_connections.fetch_add(1, Ordering::SeqCst);
        self.touch();
    }

    pub(super) fn connection_closed(&self) {
        let _ = self
            .active_connections
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                Some(value.saturating_sub(1))
            });
        self.touch();
    }

    pub(super) fn add_tcp_to_ssh_bytes(&self, bytes: usize) {
        self.add_tcp_to_ssh_bytes_u64(bytes as u64);
    }

    pub(super) fn add_tcp_to_ssh_bytes_u64(&self, bytes: u64) {
        self.tcp_to_ssh_bytes.fetch_add(bytes, Ordering::SeqCst);
        self.touch();
    }

    pub(super) fn add_ssh_to_tcp_bytes(&self, bytes: usize) {
        self.add_ssh_to_tcp_bytes_u64(bytes as u64);
    }

    pub(super) fn add_ssh_to_tcp_bytes_u64(&self, bytes: u64) {
        self.ssh_to_tcp_bytes.fetch_add(bytes, Ordering::SeqCst);
        self.touch();
    }

    pub(super) fn record_error(&self, error: &str) {
        if let Ok(mut last_error) = self.last_error.lock() {
            *last_error = Some(error.to_string());
        }
        self.touch();
    }

    pub(super) fn record_error_if_changed(&self, error: &str) -> bool {
        let changed = if let Ok(mut last_error) = self.last_error.lock() {
            if last_error.as_deref() == Some(error) {
                false
            } else {
                *last_error = Some(error.to_string());
                true
            }
        } else {
            false
        };
        if changed {
            self.touch();
        }
        changed
    }

    pub(super) fn clear_error(&self) {
        if let Ok(mut last_error) = self.last_error.lock() {
            *last_error = None;
        }
        self.touch();
    }

    pub(super) fn clear_error_with_prefix(&self, prefix: &str) -> bool {
        let cleared = if let Ok(mut last_error) = self.last_error.lock() {
            if last_error
                .as_deref()
                .is_some_and(|error| error.starts_with(prefix))
            {
                *last_error = None;
                true
            } else {
                false
            }
        } else {
            false
        };
        if cleared {
            self.touch();
        }
        cleared
    }

    pub(super) fn touch(&self) {
        if let Ok(mut last_activity) = self.last_activity.lock() {
            *last_activity = Some(Utc::now().to_rfc3339());
        }
    }

    pub(super) fn snapshot(&self, spec: TunnelSpec) -> TunnelStatus {
        TunnelStatus {
            spec,
            active_connections: self.active_connections.load(Ordering::SeqCst),
            total_connections: self.total_connections.load(Ordering::SeqCst),
            tcp_to_ssh_bytes: self.tcp_to_ssh_bytes.load(Ordering::SeqCst),
            ssh_to_tcp_bytes: self.ssh_to_tcp_bytes.load(Ordering::SeqCst),
            last_activity: self
                .last_activity
                .lock()
                .ok()
                .and_then(|value| value.clone()),
            last_error: self.last_error.lock().ok().and_then(|value| value.clone()),
        }
    }
}

pub(super) fn try_acquire_tunnel_connection(
    connection_slots: &Arc<tokio::sync::Semaphore>,
    metrics: &TunnelMetrics,
) -> Option<tokio::sync::OwnedSemaphorePermit> {
    match Arc::clone(connection_slots).try_acquire_owned() {
        Ok(permit) => {
            metrics.clear_error_with_prefix(TUNNEL_CONNECTION_LIMIT_ERROR_PREFIX);
            Some(permit)
        }
        Err(_) => {
            metrics.record_error_if_changed(&format!(
                "{TUNNEL_CONNECTION_LIMIT_ERROR_PREFIX} app limit ({MAX_TUNNEL_CONNECTIONS})"
            ));
            None
        }
    }
}

pub(super) fn forwarded_tcpip_ports(
    connected_port: u32,
    originator_port: u32,
) -> Option<(u16, u16)> {
    Some((
        u16::try_from(connected_port).ok()?,
        u16::try_from(originator_port).ok()?,
    ))
}

pub(super) async fn create_tunnel_inner(
    state: &AppState,
    request: CreateTunnelRequest,
) -> Result<TunnelSpec, String> {
    create_tunnel_inner_with_validation(state, request, None).await
}

pub(super) async fn create_tunnel_inner_with_validation(
    state: &AppState,
    request: CreateTunnelRequest,
    commit_validation: Option<CommitValidation>,
) -> Result<TunnelSpec, String> {
    let request = normalize_tunnel_request(request)?;
    ensure_tunnel_creation_capacity(state, &request.session_id, request.egress)?;
    let tunnel = TunnelSpec {
        id: Uuid::new_v4().to_string(),
        label: request.label.clone().unwrap_or_else(|| {
            tunnel_label_for_egress(
                request.egress,
                request.mode,
                &request.bind_host,
                request.bind_port,
                &request.target_host,
                request.target_port,
            )
        }),
        egress: request.egress,
        mode: request.mode,
        bind_host: request.bind_host.clone(),
        bind_port: request.bind_port,
        target_host: request.target_host.clone(),
        target_port: request.target_port,
        route_rules: request.route_rules.clone(),
        enabled: true,
    };
    validate_tunnels(std::slice::from_ref(&tunnel))?;
    let lifecycle_lane = tunnel_lifecycle_lane(state, &tunnel.id)?;
    let _lifecycle_guard = lifecycle_lane.lock().await;
    let (tunnel, local_addr, owner) = start_tunnel_runtime_with_validation(
        state,
        &request.session_id,
        tunnel,
        request.label.is_none(),
        None,
        commit_validation,
    )
    .await?;
    commit_started_tunnel(state, &request.session_id, tunnel, local_addr, &owner).await
}

pub(super) fn mcp_host_route_owner_id(client_id: &str) -> Result<String, String> {
    normalize_mcp_client_id(client_id)
        .map(|client_id| format!("{MCP_HOST_ROUTE_OWNER_PREFIX}{client_id}"))
}

pub(super) async fn create_host_route_inner_with_validation(
    state: &AppState,
    client_id: &str,
    request: CreateHostRouteRequest,
    commit_validation: Option<CommitValidation>,
) -> Result<TunnelSpec, String> {
    let owner_id = mcp_host_route_owner_id(client_id)?;
    let request = normalize_host_route_request(request)?;
    let tunnel = TunnelSpec {
        id: Uuid::new_v4().to_string(),
        label: request.label.clone().unwrap_or_else(|| {
            tunnel_label_for_egress(
                TunnelEgress::PortmateHost,
                request.mode,
                &request.bind_host,
                request.bind_port,
                &request.target_host,
                request.target_port,
            )
        }),
        egress: TunnelEgress::PortmateHost,
        mode: request.mode,
        bind_host: request.bind_host,
        bind_port: request.bind_port,
        target_host: request.target_host,
        target_port: request.target_port,
        route_rules: request.route_rules,
        enabled: true,
    };
    validate_tunnels(std::slice::from_ref(&tunnel))?;
    let lifecycle_lane = tunnel_lifecycle_lane(state, &tunnel.id)?;
    let _lifecycle_guard = lifecycle_lane.lock().await;
    let (tunnel, _, _) = start_portmate_host_tunnel_runtime_with_validation(
        state,
        &owner_id,
        tunnel,
        request.label.is_none(),
        commit_validation,
    )
    .await?;
    Ok(tunnel)
}

pub(super) fn ensure_tunnel_creation_capacity(
    state: &AppState,
    session_id: &str,
    egress: TunnelEgress,
) -> Result<(), String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    let profile = store
        .profile(session_id)
        .ok_or_else(|| format!("unknown session: {session_id}"))?;
    if egress == TunnelEgress::PortmateHost {
        drop(store);
        let tunnels = state.tunnels.lock().map_err(|error| error.to_string())?;
        if tunnels
            .values()
            .filter(|runtime| {
                runtime.session_id == session_id
                    && runtime.spec.egress == TunnelEgress::PortmateHost
                    && !runtime.closed.load(Ordering::SeqCst)
            })
            .count()
            >= MAX_TUNNELS_PER_PROFILE
        {
            return Err(format!(
                "enabled PortMate host proxy count has reached {MAX_TUNNELS_PER_PROFILE}"
            ));
        }
        return Ok(());
    }
    let tunnels = match profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => ssh.tunnels,
        _ => return Err("tunnels require an SSH or Tmux session".to_string()),
    };
    if tunnels.iter().filter(|tunnel| tunnel.enabled).count() >= MAX_TUNNELS_PER_PROFILE {
        return Err(format!(
            "enabled tunnel count has reached {MAX_TUNNELS_PER_PROFILE}"
        ));
    }
    Ok(())
}

pub(super) async fn commit_started_tunnel(
    state: &AppState,
    session_id: &str,
    tunnel: TunnelSpec,
    local_addr: Option<std::net::SocketAddr>,
    owner: &TunnelRuntimeOwner,
) -> Result<TunnelSpec, String> {
    if let Err(commit_error) =
        persist_tunnel_to_profile_and_log(state, session_id, &tunnel, local_addr)
    {
        let cleanup = stop_tunnel_runtime_effects(state, &tunnel.id, owner).await;
        return Err(match cleanup {
            Ok((_, warnings)) if warnings.is_empty() => format!(
                "tunnel Store commit failed and the uncommitted runtime was closed: {commit_error}"
            ),
            Ok((_, warnings)) => format!(
                "tunnel Store commit failed: {commit_error}; runtime cleanup warnings: {}",
                warnings.join("; ")
            ),
            Err(cleanup_error) => format!(
                "tunnel Store commit failed: {commit_error}; runtime cleanup failed: {cleanup_error}"
            ),
        });
    }
    Ok(tunnel)
}
