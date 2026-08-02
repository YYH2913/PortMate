use super::*;

mod control;

pub(super) use control::*;

pub(super) async fn list_tmux_state_inner(
    state: &AppState,
    session_id: &str,
) -> Result<TmuxState, String> {
    let auxiliary = ssh_auxiliary_lease(state, session_id)?;
    list_tmux_state_with_handle(auxiliary.handle()).await
}

async fn list_tmux_state_with_handle(
    handle: Arc<tokio::sync::Mutex<SshBackendSession>>,
) -> Result<TmuxState, String> {
    let sessions_output = exec_ssh_command_capture(
        Arc::clone(&handle),
        &format!(
            "tmux list-sessions -F '#{{session_name}}{TMUX_FIELD_SEPARATOR}#{{session_windows}}{TMUX_FIELD_SEPARATOR}#{{session_attached}}{TMUX_FIELD_SEPARATOR}#{{session_created}}' 2>/dev/null || true"
        ),
        Duration::from_secs(8),
    )
    .await?;
    let windows_output = exec_ssh_command_capture(
        Arc::clone(&handle),
        &format!(
            "tmux list-windows -a -F '#{{session_name}}{TMUX_FIELD_SEPARATOR}#{{window_index}}{TMUX_FIELD_SEPARATOR}#{{window_id}}{TMUX_FIELD_SEPARATOR}#{{window_name}}{TMUX_FIELD_SEPARATOR}#{{window_panes}}{TMUX_FIELD_SEPARATOR}#{{window_active}}' 2>/dev/null || true"
        ),
        Duration::from_secs(8),
    )
    .await?;
    let panes_output = exec_ssh_command_capture(
        handle,
        &format!(
            "tmux list-panes -a -F '#{{session_name}}{TMUX_FIELD_SEPARATOR}#{{window_index}}{TMUX_FIELD_SEPARATOR}#{{pane_index}}{TMUX_FIELD_SEPARATOR}#{{pane_id}}{TMUX_FIELD_SEPARATOR}#{{pane_active}}{TMUX_FIELD_SEPARATOR}#{{pane_current_command}}{TMUX_FIELD_SEPARATOR}#{{pane_title}}{TMUX_FIELD_SEPARATOR}#{{pane_synchronized}}' 2>/dev/null || true"
        ),
        Duration::from_secs(8),
    )
    .await?;

    let sessions = sessions_output
        .lines()
        .filter_map(parse_tmux_session)
        .collect::<Vec<_>>();
    let mut windows = windows_output
        .lines()
        .filter_map(parse_tmux_window)
        .collect::<Vec<_>>();
    let panes = panes_output
        .lines()
        .filter_map(parse_tmux_pane)
        .collect::<Vec<_>>();
    for window in &mut windows {
        let matching = panes
            .iter()
            .filter(|pane| {
                pane.session == window.session && pane.window_index == window.window_index
            })
            .collect::<Vec<_>>();
        window.synchronized = !matching.is_empty() && matching.iter().all(|pane| pane.synchronized);
    }
    Ok(TmuxState {
        sessions,
        windows,
        panes,
    })
}

pub(super) async fn set_tmux_pane_sync_inner(
    state: &AppState,
    session_id: &str,
    target: &str,
    enabled: bool,
) -> Result<TmuxState, String> {
    let auxiliary = ssh_auxiliary_lease(state, session_id)?;
    let handle = auxiliary.handle();
    let command = tmux_pane_sync_command(target, enabled)?;
    let event_message = format!(
        "PortMate: tmux pane synchronization {} ({})",
        if enabled { "enabled" } else { "disabled" },
        normalize_tmux_target(target)?
    );
    exec_ssh_command_capture(Arc::clone(&handle), &command, Duration::from_secs(8)).await?;
    record_applied_system_event(
        state,
        session_id,
        event_message,
        "tmux pane synchronization",
    );
    list_tmux_state_with_handle(handle).await
}

pub(super) async fn mutate_tmux_inner(
    state: &AppState,
    request: TmuxMutationRequest,
) -> Result<TmuxState, String> {
    let command = tmux_mutation_command(&request)?;
    let event_message = format!(
        "PortMate: tmux {} ({})",
        tmux_mutation_label(request.action),
        tmux_mutation_event_scope(&request)?
    );
    let auxiliary = ssh_auxiliary_lease(state, &request.session_id)?;
    let handle = auxiliary.handle();
    exec_ssh_command_capture(Arc::clone(&handle), &command, Duration::from_secs(8)).await?;
    record_applied_system_event(state, &request.session_id, event_message, "tmux mutation");
    list_tmux_state_with_handle(handle).await
}
