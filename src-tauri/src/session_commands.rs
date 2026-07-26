use super::*;

#[tauri::command]
pub(crate) fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionSummary>, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    Ok(store.summaries())
}

#[tauri::command]
pub(crate) fn read_screen(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<String, String> {
    let store = state.store.lock().map_err(|error| error.to_string())?;
    store
        .screen(&session_id)
        .ok_or_else(|| format!("no screen data for session: {session_id}"))
}

#[tauri::command]
pub(crate) async fn send_text(
    state: State<'_, AppState>,
    session_id: String,
    text: String,
) -> Result<SessionEvent, String> {
    send_text_inner(state.inner().session_io(), session_id, text).await
}

#[tauri::command]
pub(crate) async fn send_bytes(
    state: State<'_, AppState>,
    session_id: String,
    bytes: Vec<u8>,
) -> Result<SessionEvent, String> {
    send_bytes_inner(state.inner().session_io(), session_id, bytes).await
}

#[tauri::command]
pub(crate) async fn send_key(
    state: State<'_, AppState>,
    session_id: String,
    key: String,
) -> Result<SessionEvent, String> {
    let io = state.inner().session_io();
    let text =
        terminal_key_sequence_for_protocol(&key, is_telnet_session(&io.store, &session_id)?)?;
    send_text_inner_with_context(io, session_id, text, "desktop-user", Some("send_key")).await
}

#[tauri::command]
pub(crate) async fn run_command(
    state: State<'_, AppState>,
    session_id: String,
    command: String,
) -> Result<SessionEvent, String> {
    let io = state.inner().session_io();
    let text = terminate_command_for_protocol(command, is_telnet_session(&io.store, &session_id)?);
    run_command_inner_with_context(io, session_id, text, "desktop-user", Some("run_command")).await
}

#[tauri::command]
pub(crate) async fn resize_session(
    state: State<'_, AppState>,
    session_id: String,
    cols: u16,
    rows: u16,
) -> Result<SessionSummary, String> {
    resize_session_inner(state.inner(), session_id, cols, rows).await
}

#[tauri::command]
pub(crate) async fn delete_session_profile(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<DeleteSessionProfileResponse, String> {
    delete_session_profile_inner(state.inner(), session_id).await
}

#[tauri::command]
pub(crate) async fn open_session(
    state: State<'_, AppState>,
    session_id: String,
    password: Option<String>,
    passphrase: Option<String>,
) -> Result<SessionSummary, String> {
    let state = state.inner().clone();
    open_session_inner(
        state,
        session_id,
        SessionOpenCredentials {
            password,
            passphrase,
            ..Default::default()
        },
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
