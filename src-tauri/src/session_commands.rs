use super::session_close::close_session_inner;
use super::session_open::{
    open_reserved_session_inner, open_session_inner, register_session_open_cancellation,
    SessionOpenCredentials,
};
use super::*;

pub(super) fn mark_session_connected_with_events(
    store: &mut SessionStore,
    profile: &SessionProfile,
    messages: impl IntoIterator<Item = String>,
) -> Result<(SessionSummary, Vec<String>), String> {
    let fallback = store.set_runtime_status(&profile.id, SessionStatus::Connected)?;
    let mut event_ids = Vec::new();
    for message in messages {
        if let Some(event_id) = store.record_system_event_tracked(&profile.id, message) {
            event_ids.push(event_id);
        }
    }
    if let Some(event_id) = store.record_system_event_tracked(
        &profile.id,
        format!(
            "PortMate: connected to {} ({:?})",
            describe_endpoint(profile),
            profile.kind
        ),
    ) {
        event_ids.push(event_id);
    }
    let summary = store
        .summaries()
        .into_iter()
        .find(|summary| summary.profile.id == profile.id)
        .unwrap_or(fallback);
    Ok((summary, event_ids))
}

pub(super) fn profile_requires_runtime(
    store: &Arc<Mutex<SessionStore>>,
    session_id: &str,
) -> Result<bool, String> {
    let store = store.lock().map_err(|error| error.to_string())?;
    Ok(matches!(
        store.profile(session_id).map(|profile| profile.connection),
        Some(
            ConnectionConfig::Ssh(_)
                | ConnectionConfig::Tmux(_)
                | ConnectionConfig::Tcp(_)
                | ConnectionConfig::Telnet(_)
                | ConnectionConfig::Serial(_)
                | ConnectionConfig::Shell(_)
        )
    ))
}

pub(super) fn record_connection_failure(state: &AppState, session_id: &str, error: &str) {
    if let Ok(mut store) = state.store.lock() {
        let _ = store.set_runtime_status_with_reason(
            session_id,
            SessionStatus::Error,
            Some(error.to_string()),
        );
        store.record_system_event(session_id, format!("PortMate: connection failed: {error}"));
        if let Err(error) =
            persist_applied_store(&store, &state.store_path, "connection failure state")
        {
            eprintln!("PortMate: failed to persist connection failure: {error}");
        }
    }
}

pub(super) fn describe_endpoint(profile: &SessionProfile) -> String {
    match &profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => {
            if ssh.username.is_empty() {
                format!("{}:{}", ssh.endpoint.host, ssh.endpoint.port)
            } else {
                format!(
                    "{}@{}:{}",
                    ssh.username, ssh.endpoint.host, ssh.endpoint.port
                )
            }
        }
        ConnectionConfig::Serial(serial) => serial.port.clone(),
        ConnectionConfig::Shell(shell) => shell.program.clone(),
        ConnectionConfig::Telnet(tcp) | ConnectionConfig::Tcp(tcp) => {
            format!("{}:{}", tcp.host, tcp.port)
        }
    }
}

#[tauri::command]
pub(crate) async fn open_session(
    state: State<'_, AppState>,
    window: WebviewWindow,
    request: OpenSessionRequest,
) -> Result<SessionSummary, String> {
    let state = state.inner().clone();
    let credentials = match request.credential_handle.as_deref() {
        Some(credential_handle) => {
            let credentials = consume_session_credentials_for_owner(
                &state.session_credentials,
                window.label(),
                &request.session_id,
                credential_handle,
                Instant::now(),
            )?;
            SessionOpenCredentials {
                password: credentials.password,
                passphrase: credentials.passphrase,
                credential_binding: Some(credentials.binding),
                ..Default::default()
            }
        }
        None => SessionOpenCredentials::default(),
    };
    open_session_inner(
        state,
        request.session_id,
        credentials,
    )
    .await
}

#[tauri::command]
pub(crate) async fn open_session_with_one_key(
    state: State<'_, AppState>,
    session_id: String,
    one_key_id: String,
) -> Result<SessionSummary, String> {
    let state = state.inner().clone();
    let cancellation = register_session_open_cancellation(&state, &session_id)?;
    let credentials = resolve_one_key_login_credentials(&state, &session_id, &one_key_id)?;
    open_reserved_session_inner(
        state,
        session_id,
        SessionOpenCredentials {
            username: Some(credentials.username),
            password: credentials.password,
            passphrase: credentials.passphrase,
            identity: credentials.identity,
            isolate_saved_ssh_credentials: true,
            credential_binding: None,
        },
        cancellation,
    )
    .await
}

#[tauri::command]
pub(crate) async fn close_session(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<SessionSummary, String> {
    close_session_inner(state.inner(), session_id).await
}
