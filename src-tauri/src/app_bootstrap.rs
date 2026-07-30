use super::*;

pub fn run() {
    webkit_runtime::configure_webkit_runtime();
    tauri::Builder::default()
        .setup(|app| {
            let data_root = app.path().data_dir()?;
            let data_dir = app.path().app_data_dir()?;
            migrate_legacy_app_data_dir(&data_root, &data_dir).map_err(std::io::Error::other)?;
            PORTABLE_VAULT
                .set(PortableVaultContext {
                    snapshot_path: data_dir.join(PORTABLE_VAULT_FILE_NAME),
                    salt_path: data_dir.join(PORTABLE_VAULT_SALT_FILE_NAME),
                    stronghold: Mutex::new(None),
                })
                .map_err(|_| std::io::Error::other("portable vault initialized twice"))?;
            let store_path = data_dir.join(STORE_FILE_NAME);
            let store = load_store(&store_path).map_err(std::io::Error::other)?;
            let retention_store_path = store_path.clone();
            let retention_profiles = store.profiles.clone();
            std::thread::spawn(move || {
                for profile in retention_profiles {
                    if let Err(error) =
                        maybe_prune_expired_log_shards(&retention_store_path, &profile)
                    {
                        eprintln!(
                            "PortMate: startup log retention failed for {}: {error}",
                            profile.id
                        );
                    }
                }
            });
            let state = AppState {
                app_handle: Some(app.handle().clone()),
                store: Arc::new(Mutex::new(store)),
                credential_ops: Arc::new(Mutex::new(())),
                credential_lock_path: data_dir.join("credentials.lock"),
                system_event_sink: Arc::new(Mutex::new(None)),
                session_open_slots: Arc::new(tokio::sync::Semaphore::new(
                    MAX_CONCURRENT_SESSION_OPENS,
                )),
                ssh: Arc::new(Mutex::new(HashMap::new())),
                ssh_auxiliary_slots: Arc::new(tokio::sync::Semaphore::new(
                    MAX_CONCURRENT_SSH_AUXILIARY_OPERATIONS,
                )),
                tmux_controls: Arc::new(Mutex::new(HashMap::new())),
                tmux_control_slots: Arc::new(tokio::sync::Semaphore::new(MAX_ACTIVE_TMUX_CONTROLS)),
                shell: Arc::new(Mutex::new(HashMap::new())),
                tcp: Arc::new(Mutex::new(HashMap::new())),
                serial: Arc::new(Mutex::new(HashMap::new())),
                serial_captures: Arc::new(Mutex::new(HashMap::new())),
                active_commands: Arc::new(Mutex::new(HashMap::new())),
                tunnels: Arc::new(Mutex::new(HashMap::new())),
                tunnel_connection_slots: Arc::new(tokio::sync::Semaphore::new(
                    MAX_TUNNEL_CONNECTIONS,
                )),
                transfer_cancellations: Arc::new(Mutex::new(HashMap::new())),
                transfer_task_slots: Arc::new(tokio::sync::Semaphore::new(
                    MAX_ACTIVE_TRANSFER_TASKS,
                )),
                transfer_lanes: Arc::new(Mutex::new(HashMap::new())),
                sysmon_slots: Arc::new(tokio::sync::Semaphore::new(
                    MAX_CONCURRENT_SYSMON_REFRESHES,
                )),
                trigger_command_slots: Arc::new(tokio::sync::Semaphore::new(
                    MAX_TRIGGER_COMMAND_CONCURRENCY,
                )),
                trigger_send_batch_slots: Arc::new(tokio::sync::Semaphore::new(
                    MAX_TRIGGER_SEND_BATCH_CONCURRENCY,
                )),
                pending_mcp_approvals: Arc::new(Mutex::new(HashMap::new())),
                one_time_host_keys: Arc::new(Mutex::new(HashMap::new())),
                ipc_publication: Arc::new(Mutex::new(IpcPublicationState::default())),
                #[cfg(test)]
                ssh_reconnect_install_error: Arc::new(Mutex::new(None)),
                store_path,
            };
            install_system_event_sink(&state).map_err(std::io::Error::other)?;
            start_ipc_server(
                state.clone(),
                data_dir.join("portmate-ipc.json"),
                Uuid::new_v4().to_string(),
            );
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            session_commands::list_sessions,
            session_commands::read_screen,
            log_commands::tail_log,
            log_commands::search_logs,
            log_commands::list_log_shards,
            log_commands::read_log_shard,
            log_commands::delete_log_shards,
            log_commands::search_log_shards,
            log_commands::archive_log_shards,
            log_commands::export_session_bundle_archive,
            session_commands::send_text,
            session_commands::send_bytes,
            session_commands::send_key,
            session_commands::run_command,
            session_commands::resize_session,
            profile_commands::save_session_profile,
            session_commands::delete_session_profile,
            session_commands::open_session,
            session_commands::open_session_with_one_key,
            session_commands::close_session,
            ssh_health::check_ssh_health,
            ssh_host_key_commands::evaluate_host_key,
            ssh_host_key_commands::apply_host_key_decision,
            ssh_host_key_commands::scan_ssh_host_key,
            ssh_host_key_commands::trust_scanned_host_key,
            ssh_host_key_commands::import_known_hosts,
            ssh_host_key_commands::export_known_hosts,
            ssh_host_key_commands::delete_host_key,
            ssh_host_key_commands::delete_host_keys,
            ssh_host_key_commands::update_host_key,
            transfer_commands::list_transfers,
            transfer_commands::retry_transfer,
            transfer_commands::cancel_transfer,
            mcp_commands::list_mcp_audit,
            mcp_commands::export_mcp_audit,
            mcp_commands::list_mcp_grants,
            mcp_commands::list_mcp_approvals,
            mcp_commands::respond_mcp_approval,
            mcp_commands::save_mcp_grant,
            mcp_commands::revoke_mcp_grant,
            mcp_commands::mcp_http_config,
            mcp_commands::rotate_mcp_http_token,
            ssh_host_key_commands::list_host_keys,
            ssh_identity_commands::list_ssh_agent_identities,
            one_key_commands::list_one_keys,
            one_key_commands::save_one_key,
            one_key_commands::delete_one_key,
            one_key_commands::send_one_key,
            secret_commands::save_secret,
            secret_commands::delete_secret,
            secret_commands::has_secret,
            vault_commands::portable_vault_status,
            vault_commands::unlock_portable_vault,
            vault_commands::rotate_portable_vault_password,
            vault_commands::lock_portable_vault,
            vault_commands::preview_profile_secret_migration,
            vault_commands::migrate_profile_secrets,
            vault_commands::get_profile_secret_migration_recovery,
            vault_commands::export_profile_secret_migration_diagnostics,
            vault_commands::recover_profile_secret_migration,
            ssh_identity_commands::update_client_identity,
            ssh_identity_commands::rotate_client_identity,
            ssh_identity_commands::delete_client_identity,
            serial_commands::list_serial_ports,
            serial_commands::list_serial_capture,
            serial_commands::list_serial_capture_history,
            serial_commands::clear_serial_capture,
            serial_commands::export_serial_capture,
            serial_commands::export_serial_capture_history,
            terminal_export_commands::export_terminal_text,
            tmux_commands::list_tmux_state,
            tmux_commands::attach_tmux,
            tmux_commands::set_tmux_pane_sync,
            tmux_commands::mutate_tmux,
            tmux_commands::start_tmux_control,
            tmux_commands::stop_tmux_control,
            file_commands::list_files,
            file_commands::file_properties,
            file_commands::create_directory,
            file_commands::create_file,
            file_commands::delete_path,
            file_commands::delete_paths,
            file_commands::rename_path,
            file_commands::move_paths,
            file_commands::chmod_path,
            serial_commands::serial_set_lines,
            serial_commands::serial_send_break,
            sysmon_commands::refresh_sysmon,
            sysmon_commands::list_sysmon_history,
            transfer_commands::start_transfer,
            transfer_commands::start_external_drop,
            transfer_commands::start_file_batch,
            tunnel_commands::create_tunnel,
            tunnel_commands::list_tunnels,
            tunnel_commands::stop_tunnel,
            mcp_commands::mcp_manifest
        ])
        .build(tauri::generate_context!())
        .expect("error while building PortMate")
        .run(|app_handle, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                if let Some(state) = app_handle.try_state::<AppState>() {
                    shutdown_ipc_publication(state.inner());
                    shutdown_tmux_controls(state.inner());
                    shutdown_system_event_sink(state.inner());
                }
            }
        });
}
