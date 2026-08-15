use super::*;

#[tauri::command]
pub(crate) async fn list_tmux_state(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<TmuxState, String> {
    list_tmux_state_inner(state.inner(), &session_id).await
}

#[tauri::command]
pub(crate) async fn attach_tmux(
    state: State<'_, AppState>,
    session_id: String,
    target: String,
) -> Result<SessionEvent, String> {
    let command = tmux_attach_command(&target)?;
    send_text_inner_with_context(
        state.inner().session_io(),
        session_id,
        command,
        "desktop-user",
        Some("attach_tmux"),
    )
    .await
}

#[tauri::command]
pub(crate) async fn set_tmux_pane_sync(
    state: State<'_, AppState>,
    session_id: String,
    target: String,
    enabled: bool,
) -> Result<TmuxState, String> {
    set_tmux_pane_sync_inner(state.inner(), &session_id, &target, enabled).await
}

#[tauri::command]
pub(crate) async fn mutate_tmux(
    state: State<'_, AppState>,
    request: TmuxMutationRequest,
) -> Result<TmuxState, String> {
    mutate_tmux_inner(state.inner(), request).await
}

#[tauri::command]
pub(crate) async fn start_tmux_control(
    state: State<'_, AppState>,
    session_id: String,
    target: String,
) -> Result<TmuxControlStatus, String> {
    start_tmux_control_inner(state.inner(), &session_id, &target).await
}

#[tauri::command]
pub(crate) fn stop_tmux_control(
    state: State<'_, AppState>,
    session_id: String,
    target: Option<String>,
    runtime_id: Option<String>,
) -> Result<TmuxControlStatus, String> {
    stop_tmux_control_inner(
        state.inner(),
        &session_id,
        target.as_deref(),
        runtime_id.as_deref(),
    )
}
