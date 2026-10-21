use super::update_policy::*;
use super::*;
use tauri::ipc::Channel;
use tauri::Manager;
use tauri_plugin_updater::{Update, UpdaterExt};

const CANDIDATE_TTL: Duration = Duration::from_secs(30 * 60);
const MAX_RELEASE_METADATA_BYTES: usize = 3 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct UpdateRuntime {
    candidate: tokio::sync::Mutex<Option<UpdateCandidate>>,
    preparing: AtomicBool,
}

struct UpdateCandidate {
    id: String,
    owner: String,
    checked_at: Instant,
    update: Update,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateError {
    code: &'static str,
    detail: String,
}

fn failure(code: &'static str, detail: impl ToString) -> UpdateError {
    UpdateError {
        code,
        detail: detail.to_string(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateCheckResult {
    current_version: String,
    version: String,
    release_url: String,
    notes: String,
    published_at: Option<String>,
    available: bool,
    candidate_id: Option<String>,
    manual_reason: Option<&'static str>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateProgress {
    phase: &'static str,
    downloaded: u64,
    total: Option<u64>,
}

fn authorize(window: &WebviewWindow) -> Result<(), UpdateError> {
    if window.label() == "main" || window.label().starts_with("workspace-") {
        Ok(())
    } else {
        Err(failure("unavailable", "Updates require a workspace window"))
    }
}

fn package_target(_app: &AppHandle) -> Option<String> {
    let package = tauri::utils::platform::bundle_type()?.to_string();
    #[cfg(target_os = "linux")]
    if package == "appimage" {
        // Extracted AppDir/tar packages retain the AppImage bundle marker.
        // Replacing their raw ELF executable with an AppImage would leave the
        // rest of that portable installation stale. Require the actual image.
        let image = _app.env().appimage?;
        let mut header = [0_u8; 11];
        fs::File::open(image).ok()?.read_exact(&mut header).ok()?;
        if !appimage_header(&header) {
            return None;
        }
    }
    updater_target(std::env::consts::OS, std::env::consts::ARCH, &package)
}

pub(super) fn ensure_not_updating(state: &AppState) -> Result<(), String> {
    if state
        .app_handle
        .as_ref()
        .and_then(|app| app.try_state::<UpdateRuntime>())
        .is_some_and(|runtime| runtime.preparing.load(Ordering::SeqCst))
    {
        Err("PortMate is preparing an update; new work is not accepted".into())
    } else {
        Ok(())
    }
}

async fn release_json(url: &str) -> Result<serde_json::Value, UpdateError> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .user_agent(concat!("PortMate/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| failure("check-failed", e))?;
    let mut response = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| failure("check-failed", e))?
        .error_for_status()
        .map_err(|e| failure("check-failed", e))?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| failure("check-failed", e))?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_RELEASE_METADATA_BYTES {
            return Err(failure("invalid-release", "Release metadata is too large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|e| failure("invalid-release", e))
}

#[tauri::command]
pub(crate) async fn check_for_updates(
    app: AppHandle,
    window: WebviewWindow,
    runtime: State<'_, UpdateRuntime>,
) -> Result<UpdateCheckResult, UpdateError> {
    authorize(&window)?;
    let mut cached = runtime
        .candidate
        .try_lock()
        .map_err(|_| failure("busy", "An update operation is already running"))?;
    *cached = None;
    let release: GithubRelease = serde_json::from_value(release_json(RELEASE_API).await?)
        .map_err(|e| failure("invalid-release", e))?;
    let version = stable_release_version(&release)
        .map_err(|code| failure(code, "Expected a stable semantic version"))?;
    let current = app.package_info().version.clone();
    let mut result = UpdateCheckResult {
        current_version: current.to_string(),
        version: version.to_string(),
        release_url: release_url(&release.tag_name),
        notes: release.body.clone().unwrap_or_default(),
        published_at: release.published_at.clone(),
        available: version > current,
        candidate_id: None,
        manual_reason: None,
    };
    if !result.available {
        return Ok(result);
    }
    let Some(target) = package_target(&app) else {
        result.manual_reason = Some("unsupported-package");
        return Ok(result);
    };
    let Some(manifest) = release
        .assets
        .iter()
        .find(|asset| asset.name == "latest.json")
    else {
        result.manual_reason = Some("unsigned-release");
        return Ok(result);
    };
    let endpoint = url::Url::parse(&manifest.browser_download_url)
        .map_err(|e| failure("invalid-release", e))?;
    if !is_release_asset(&endpoint, &release.tag_name) {
        return Err(failure(
            "invalid-release",
            "Updater manifest is outside the selected release",
        ));
    }
    let metadata = release_json(endpoint.as_str()).await?;
    if !metadata
        .get("platforms")
        .and_then(|value| value.as_object())
        .is_some_and(|platforms| platforms.contains_key(&target))
    {
        return Err(failure(
            "invalid-release",
            "The release has no update package for this installation type",
        ));
    }
    let mut update = app
        .updater_builder()
        .target(&target)
        .endpoints(vec![endpoint])
        .map_err(|e| failure("check-failed", e))?
        .timeout(Duration::from_secs(30))
        .configure_client(|client| {
            client
                .https_only(true)
                .connect_timeout(Duration::from_secs(10))
        })
        // prepare_for_update already stops owned processes and flushes data.
        // Preserve the WebView if ShellExecuteW / UAC fails so the error can
        // be reported. The updater exits only after the installer starts.
        .on_before_exit(|| {})
        .build()
        .map_err(|e| failure("check-failed", e))?
        .restore_update(metadata)
        .map_err(|e| failure("check-failed", e))?
        .ok_or_else(|| {
            failure(
                "invalid-release",
                "Updater manifest does not offer the selected newer release",
            )
        })?;
    if update.version != version.to_string()
        || !is_release_asset(&update.download_url, &release.tag_name)
        || !asset_matches_target(&update.download_url, &target)
        || !release
            .assets
            .iter()
            .any(|asset| asset.browser_download_url == update.download_url.as_str())
        || update.signature.trim().is_empty()
    {
        return Err(failure(
            "invalid-release",
            "Updater package identity does not match the selected release",
        ));
    }
    update.timeout = Some(Duration::from_secs(15 * 60));
    let id = Uuid::new_v4().to_string();
    result.candidate_id = Some(id.clone());
    *cached = Some(UpdateCandidate {
        id,
        owner: window.label().to_string(),
        checked_at: Instant::now(),
        update,
    });
    Ok(result)
}

#[tauri::command]
pub(crate) async fn install_update(
    app: AppHandle,
    window: WebviewWindow,
    state: State<'_, AppState>,
    runtime: State<'_, UpdateRuntime>,
    candidate_id: String,
    on_progress: Channel<UpdateProgress>,
) -> Result<(), UpdateError> {
    authorize(&window)?;
    let mut cached = runtime
        .candidate
        .try_lock()
        .map_err(|_| failure("busy", "An update operation is already running"))?;
    let candidate = cached
        .as_ref()
        .filter(|candidate| {
            candidate.id == candidate_id
                && candidate.owner == window.label()
                && candidate.checked_at.elapsed() < CANDIDATE_TTL
        })
        .ok_or_else(|| failure("expired", "Check for updates again before installing"))?;
    let bytes = download_and_prepare_update(
        &candidate.update,
        state.inner(),
        runtime.inner(),
        &on_progress,
    )
    .await?;
    let update = cached
        .take()
        .expect("candidate held under update operation lock")
        .update;
    let _ = on_progress.send(UpdateProgress {
        phase: "installing",
        downloaded: bytes.len() as u64,
        total: Some(bytes.len() as u64),
    });
    // Installer extraction / file replacement is blocking. In particular the
    // macOS installer can dispatch an authorization prompt to the main thread.
    let installed = tauri::async_runtime::spawn_blocking(move || update.install(bytes)).await;
    if let Err(error) = installed
        .map_err(|e| failure("install-failed", e))
        .and_then(|result| result.map_err(|e| failure("install-failed", e)))
    {
        runtime.preparing.store(false, Ordering::SeqCst);
        return Err(error);
    }
    // Windows' updater exits after launching MSI/NSIS. Other platforms finish
    // replacing the bundle here and must restart explicitly.
    app.request_restart();
    Ok(())
}

async fn download_and_prepare_update(
    update: &Update,
    state: &AppState,
    runtime: &UpdateRuntime,
    on_progress: &Channel<UpdateProgress>,
) -> Result<Vec<u8>, UpdateError> {
    let mut downloaded = 0_u64;
    let mut last_progress = Instant::now() - Duration::from_secs(1);
    let _ = on_progress.send(UpdateProgress {
        phase: "downloading",
        downloaded: 0,
        total: None,
    });
    let bytes = update
        .download(
            |chunk, total| {
                downloaded = downloaded.saturating_add(chunk as u64);
                if last_progress.elapsed() >= Duration::from_millis(100) {
                    let _ = on_progress.send(UpdateProgress {
                        phase: "downloading",
                        downloaded,
                        total,
                    });
                    last_progress = Instant::now();
                }
            },
            || {
                let _ = on_progress.send(UpdateProgress {
                    phase: "verifying",
                    downloaded: 0,
                    total: None,
                });
            },
        )
        .await
        .map_err(|e| failure("download-failed", e))?;
    // A failed download/signature must never close a session or exit the app.
    runtime.preparing.store(true, Ordering::SeqCst);
    let prepared = prepare_for_update(state, on_progress).await;
    if let Err(error) = prepared {
        runtime.preparing.store(false, Ordering::SeqCst);
        return Err(error);
    }
    Ok(bytes)
}

#[cfg(all(test, not(feature = "desktop")))]
#[path = "../../test/rust/portmate/unit/update_commands.rs"]
mod tests;

async fn prepare_for_update(
    state: &AppState,
    progress: &Channel<UpdateProgress>,
) -> Result<(), UpdateError> {
    let _ = progress.send(UpdateProgress {
        phase: "preparing",
        downloaded: 0,
        total: None,
    });
    let state_copy = state.clone();
    tauri::async_runtime::spawn_blocking(move || {
        host_script_commands::cancel_owner(&state_copy.store_path, None);
        stop_mcp_http_runtime_inner(&state_copy).map(|_| ())
    })
    .await
    .map_err(|e| failure("prepare-failed", e))?
    .map_err(|e| failure("prepare-failed", e))?;
    let (profiles, mut sessions, transfers) = {
        let store = state
            .store
            .lock()
            .map_err(|e| failure("prepare-failed", e))?;
        (
            store
                .profiles
                .iter()
                .map(|profile| profile.id.clone())
                .collect::<Vec<_>>(),
            store
                .runtimes
                .iter()
                .filter(|runtime| {
                    matches!(
                        runtime.status,
                        SessionStatus::Connected
                            | SessionStatus::Connecting
                            | SessionStatus::Reconnecting
                    )
                })
                .map(|runtime| runtime.session_id.clone())
                .collect::<HashSet<_>>(),
            store
                .transfers
                .iter()
                .filter(|task| {
                    matches!(
                        task.status,
                        TransferStatus::Queued | TransferStatus::Running
                    )
                })
                .map(|task| task.id.clone())
                .collect::<Vec<_>>(),
        )
    };
    for id in transfers {
        cancel_transfer_inner(state, &id).map_err(|e| failure("prepare-failed", e))?;
    }
    for id in &profiles {
        if session_open::cancel_pending_session_opens(state, id)
            .map_err(|e| failure("prepare-failed", e))?
            > 0
        {
            sessions.insert(id.clone());
        }
    }
    sessions.extend(
        state
            .ssh
            .lock()
            .map_err(|e| failure("prepare-failed", e))?
            .keys()
            .cloned(),
    );
    sessions.extend(
        state
            .shell
            .lock()
            .map_err(|e| failure("prepare-failed", e))?
            .keys()
            .cloned(),
    );
    sessions.extend(
        state
            .tcp
            .lock()
            .map_err(|e| failure("prepare-failed", e))?
            .keys()
            .cloned(),
    );
    sessions.extend(
        state
            .serial
            .lock()
            .map_err(|e| failure("prepare-failed", e))?
            .keys()
            .cloned(),
    );
    // Each close owns the session lifecycle lane and waits for Shell children,
    // SSH readers, COM handles and tunnel listeners with existing deadlines.
    for id in sessions {
        session_close::close_session_inner(state, id)
            .await
            .map_err(|e| failure("prepare-failed", e))?;
    }
    // Host-routed listeners can exist without a terminal session.
    let tunnels = {
        let mut tunnels = state
            .tunnels
            .lock()
            .map_err(|e| failure("prepare-failed", e))?;
        tunnels
            .drain()
            .map(|(_, runtime)| {
                runtime.request_shutdown();
                runtime
            })
            .collect::<Vec<_>>()
    };
    if !await_tunnel_listener_shutdowns(&tunnels).await.is_empty() {
        return Err(failure(
            "prepare-failed",
            "A managed tunnel listener is still releasing",
        ));
    }
    let state_copy = state.clone();
    tauri::async_runtime::spawn_blocking(move || {
        flush_inbound_log_queues(&state_copy.store_path, Duration::from_secs(5))?;
        let store = state_copy.store.lock().map_err(|e| e.to_string())?;
        persist_applied_store(&store, &state_copy.store_path, "application update flush")?;
        flush_json_compatibility_snapshots(Duration::from_secs(5))
    })
    .await
    .map_err(|e| failure("prepare-failed", e))?
    .map_err(|e| failure("prepare-failed", e))
}
