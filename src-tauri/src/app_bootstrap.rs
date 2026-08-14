use super::*;

const NATIVE_SMOKE_DATA_DIR_ENV: &str = "PORTMATE_NATIVE_SMOKE_DATA_DIR";
const NATIVE_SMOKE_EXIT_AFTER_MS_ENV: &str = "PORTMATE_NATIVE_SMOKE_EXIT_AFTER_MS";
const MIN_NATIVE_SMOKE_EXIT_AFTER_MS: u64 = 1_000;
const MAX_NATIVE_SMOKE_EXIT_AFTER_MS: u64 = 60_000;

#[derive(Debug)]
struct NativeSmokeConfig {
    data_root: PathBuf,
    data_dir: PathBuf,
    exit_after: Duration,
}

pub fn run() {
    webkit_runtime::configure_webkit_runtime();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let native_smoke = native_smoke_config().map_err(std::io::Error::other)?;
            let (data_root, data_dir) = match &native_smoke {
                Some(config) => {
                    fs::create_dir_all(&config.data_dir)?;
                    (config.data_root.clone(), config.data_dir.clone())
                }
                None => (app.path().data_dir()?, app.path().app_data_dir()?),
            };
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
                session_credentials: Arc::new(Mutex::new(SessionCredentialRegistry::default())),
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
                mcp_content_transfer_staging: Arc::new(Mutex::new(HashMap::new())),
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
                mcp_http_process: Arc::new(Mutex::new(McpHttpProcessRegistry::default())),
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
            if let Some(config) = native_smoke {
                let app_handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(config.exit_after);
                    app_handle.exit(0);
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            session_terminal::list_sessions,
            session_terminal::read_screen,
            log_commands::tail_log,
            log_commands::search_logs,
            log_commands::list_log_shards,
            log_commands::read_log_shard,
            log_commands::delete_log_shards,
            log_commands::search_log_shards,
            log_commands::archive_log_shards,
            log_commands::export_session_bundle_archive,
            session_terminal::send_text,
            session_terminal::send_bytes,
            session_terminal::send_key,
            session_terminal::run_command,
            session_terminal::resize_session,
            command_history_commands::list_command_history,
            command_history_commands::migrate_command_history,
            command_history_commands::record_command_history,
            command_history_commands::merge_command_history,
            command_history_commands::normalize_command_history,
            command_history_commands::clear_command_history,
            custom_script_commands::list_custom_scripts,
            custom_script_commands::save_custom_script,
            custom_script_commands::delete_custom_script,
            custom_script_commands::run_custom_script,
            profile_commands::save_session_profile,
            session_profile_delete::delete_session_profile,
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
            mcp_commands::preview_mcp_http_config,
            mcp_commands::save_mcp_http_settings,
            mcp_commands::rotate_mcp_http_token,
            mcp_commands::mcp_http_runtime_status,
            mcp_commands::start_mcp_http,
            mcp_commands::stop_mcp_http,
            ssh_host_key_commands::list_host_keys,
            ssh_identity_commands::list_ssh_agent_identities,
            one_key_commands::list_one_keys,
            one_key_commands::save_one_key,
            one_key_commands::delete_one_key,
            one_key_commands::send_one_key,
            secret_commands::save_secret,
            secret_commands::delete_secret,
            secret_commands::has_secret,
            session_credentials::stage_session_credentials,
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
                    shutdown_mcp_http_runtime(state.inner());
                    shutdown_ipc_publication(state.inner());
                    shutdown_tmux_controls(state.inner());
                    shutdown_system_event_sink(state.inner());
                    if let Err(error) =
                        flush_json_compatibility_snapshots(Duration::from_secs(5))
                    {
                        eprintln!(
                            "PortMate: JSON compatibility snapshot did not flush during shutdown: {error}"
                        );
                    }
                }
            }
        });
}

fn native_smoke_config() -> Result<Option<NativeSmokeConfig>, String> {
    parse_native_smoke_config(
        std::env::var_os(NATIVE_SMOKE_DATA_DIR_ENV),
        std::env::var_os(NATIVE_SMOKE_EXIT_AFTER_MS_ENV),
    )
}

fn parse_native_smoke_config(
    data_dir: Option<std::ffi::OsString>,
    exit_after_ms: Option<std::ffi::OsString>,
) -> Result<Option<NativeSmokeConfig>, String> {
    let (data_dir, exit_after_ms) = match (data_dir, exit_after_ms) {
        (None, None) => return Ok(None),
        (Some(data_dir), Some(exit_after_ms)) => (data_dir, exit_after_ms),
        _ => {
            return Err(format!(
                "{NATIVE_SMOKE_DATA_DIR_ENV} and {NATIVE_SMOKE_EXIT_AFTER_MS_ENV} must be set together"
            ));
        }
    };
    let data_dir = PathBuf::from(data_dir);
    if !data_dir.is_absolute() || data_dir.parent().is_none() || data_dir.file_name().is_none() {
        return Err(format!(
            "{NATIVE_SMOKE_DATA_DIR_ENV} must be an absolute non-root path"
        ));
    }
    let exit_after_ms = exit_after_ms
        .to_str()
        .ok_or_else(|| format!("{NATIVE_SMOKE_EXIT_AFTER_MS_ENV} must be valid UTF-8"))?
        .parse::<u64>()
        .map_err(|_| format!("{NATIVE_SMOKE_EXIT_AFTER_MS_ENV} must be an integer"))?;
    if !(MIN_NATIVE_SMOKE_EXIT_AFTER_MS..=MAX_NATIVE_SMOKE_EXIT_AFTER_MS).contains(&exit_after_ms) {
        return Err(format!(
            "{NATIVE_SMOKE_EXIT_AFTER_MS_ENV} must be between {MIN_NATIVE_SMOKE_EXIT_AFTER_MS} and {MAX_NATIVE_SMOKE_EXIT_AFTER_MS}"
        ));
    }
    let data_root = data_dir
        .parent()
        .expect("absolute non-root smoke data path was validated")
        .to_path_buf();
    Ok(Some(NativeSmokeConfig {
        data_root,
        data_dir,
        exit_after: Duration::from_millis(exit_after_ms),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_smoke_configuration_is_explicit_bounded_and_isolated() {
        assert!(parse_native_smoke_config(None, None).unwrap().is_none());
        assert!(parse_native_smoke_config(
            Some(std::env::temp_dir().join("portmate-smoke").into_os_string()),
            None,
        )
        .unwrap_err()
        .contains("must be set together"));

        let data_dir = std::env::temp_dir().join("portmate-smoke");
        let config =
            parse_native_smoke_config(Some(data_dir.clone().into_os_string()), Some("2500".into()))
                .unwrap()
                .expect("complete smoke configuration should be enabled");
        assert_eq!(config.data_dir, data_dir);
        assert_eq!(config.data_root, data_dir.parent().unwrap());
        assert_eq!(config.exit_after, Duration::from_millis(2_500));

        for delay in ["999", "60001", "not-a-number"] {
            assert!(parse_native_smoke_config(
                Some(data_dir.clone().into_os_string()),
                Some(delay.into()),
            )
            .is_err());
        }
    }
}
