use super::*;

pub(super) async fn remove_ssh_runtime_after_failed_open(
    state: &AppState,
    session_id: &str,
    runtime_id: &str,
) -> Result<(), String> {
    let runtime = remove_runtime_if_owned(&state.ssh, session_id, |runtime| {
        runtime.runtime_id == runtime_id
    })?;
    let Some(runtime) = runtime else {
        return Ok(());
    };
    runtime.closed.store(true, Ordering::SeqCst);
    let handle = runtime.handle.lock().await;
    let _ = handle.disconnect("PortMate connection commit failed").await;
    drop(handle);
    for jump_handle in runtime.jump_handles {
        let handle = jump_handle.lock().await;
        let _ = handle
            .disconnect(
                Disconnect::ByApplication,
                "PortMate connection commit failed",
                "en",
            )
            .await;
    }
    Ok(())
}

pub(super) fn restore_one_time_host_keys(
    state: &AppState,
    profile_id: &str,
    keys: Vec<TrustedHostKey>,
) -> Result<(), String> {
    restore_one_time_host_keys_in(&state.one_time_host_keys, profile_id, keys)
}

pub(super) fn restore_one_time_host_keys_in(
    one_time: &Arc<Mutex<HashMap<String, Vec<TrustedHostKey>>>>,
    profile_id: &str,
    keys: Vec<TrustedHostKey>,
) -> Result<(), String> {
    if keys.is_empty() {
        return Ok(());
    }
    let mut one_time = one_time.lock().map_err(|error| error.to_string())?;
    let retained = one_time.entry(profile_id.to_string()).or_default();
    for key in keys {
        if !retained.iter().any(|existing| existing.id == key.id) {
            retained.push(key);
        }
    }
    Ok(())
}

pub(super) async fn open_ssh_session(
    state: &AppState,
    profile: SessionProfile,
    password: Option<String>,
    passphrase: Option<String>,
) -> Result<SessionSummary, String> {
    if let Some(existing) = {
        let mut connections = state.ssh.lock().map_err(|error| error.to_string())?;
        connections.remove(&profile.id)
    } {
        disconnect_registered_ssh_runtime(
            existing,
            "PortMate reconnect",
            "PortMate reconnect jump",
        )
        .await;
    }

    let established = establish_ssh_runtime(state, &profile, password, passphrase).await?;
    let EstablishedSshRuntime {
        runtime_id,
        runtime,
        tap,
        read_half,
        auth_method,
        closed,
        terminal_channel_open,
        reader_finished,
    } = established;
    {
        let mut connections = state.ssh.lock().map_err(|error| error.to_string())?;
        connections.insert(profile.id.clone(), runtime);
    }
    let (consumed_one_time_host_keys, one_time_cleanup_error) =
        match take_one_time_host_keys(state, &profile.id) {
            Ok(keys) => (keys, None),
            Err(error) => (Vec::new(), Some(error)),
        };

    let finalize_result = match state.store.lock() {
        Ok(mut store) => {
            commit_tracked_store_mutation(&mut store, &state.store_path, |next_store| {
                let _ = next_store.record_auth_success(&profile.id, auth_method);
                let mut messages = vec![format!(
                    "PortMate: SSH authentication succeeded via {auth_method:?}"
                )];
                if let Some(error) = one_time_cleanup_error.as_deref() {
                    messages.push(format!(
                        "PortMate: failed to consume one-time host key trust: {error}"
                    ));
                }
                mark_session_connected_with_events(next_store, &profile, messages)
            })
        }
        Err(error) => Err(error.to_string()),
    };
    let summary = match finalize_result {
        Ok(summary) => summary,
        Err(error) => {
            let mut errors = vec![error];
            if let Err(cleanup_error) =
                remove_ssh_runtime_after_failed_open(state, &profile.id, &runtime_id).await
            {
                errors.push(format!("SSH runtime cleanup failed: {cleanup_error}"));
            }
            if let Err(restore_error) =
                restore_one_time_host_keys(state, &profile.id, consumed_one_time_host_keys)
            {
                errors.push(format!(
                    "one-time host key trust restore failed: {restore_error}"
                ));
            }
            return Err(errors.join("; "));
        }
    };

    tauri::async_runtime::spawn(read_ssh_channel(SshReadTask {
        state: state.clone(),
        profile: profile.clone(),
        runtime_id,
        tap,
        read_half,
        closed,
        terminal_channel_open,
        reader_finished,
    }));
    Ok(summary)
}
