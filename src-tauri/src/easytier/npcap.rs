//! Npcap is a separately licensed user installation, never a bundled payload.
use super::*;

#[cfg(any(windows, test))]
mod setup;
#[cfg(windows)]
mod windows;

pub(super) const DOWNLOAD_URL: &str = "https://npcap.com/dist/npcap-1.90.exe";
pub(super) const LICENSE_URL: &str = "https://github.com/nmap/npcap/blob/v1.90/LICENSE";
pub(super) const DOWNLOAD_VERSION: &str = "1.90";

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NpcapStatus {
    required: bool,
    pub(super) installed: bool,
    phase: &'static str,
    downloaded: u64,
    total: Option<u64>,
    installer_version: &'static str,
    download_url: &'static str,
    license_url: &'static str,
    error: Option<EasyTierError>,
}

#[derive(Default)]
pub(super) struct NpcapRuntime {
    #[cfg(windows)]
    state: Arc<Mutex<setup::SetupState>>,
}

impl NpcapRuntime {
    pub(super) fn status(&self) -> NpcapStatus {
        #[cfg(windows)]
        let (installed, state) = (
            windows::detected(),
            self.state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        #[cfg(not(windows))]
        let installed = true;
        NpcapStatus {
            required: cfg!(windows),
            installed,
            #[cfg(windows)]
            phase: if state.phase.is_empty() {
                if installed {
                    "detected"
                } else {
                    "missing"
                }
            } else {
                state.phase
            },
            #[cfg(not(windows))]
            phase: "not-required",
            #[cfg(windows)]
            downloaded: state.downloaded,
            #[cfg(windows)]
            total: state.total,
            #[cfg(windows)]
            error: state.error.clone(),
            installer_version: DOWNLOAD_VERSION,
            download_url: DOWNLOAD_URL,
            license_url: LICENSE_URL,
            #[cfg(not(windows))]
            downloaded: 0,
            #[cfg(not(windows))]
            total: None,
            #[cfg(not(windows))]
            error: None,
        }
    }

    pub(super) fn ensure_idle(&self) -> Result<(), EasyTierError> {
        #[cfg(windows)]
        if self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .phase
            == "installing"
        {
            return Err(failure(
                "npcap-installing",
                "Complete or close the interactive Npcap installer first",
            ));
        }
        Ok(())
    }

    pub(super) fn shutdown(&self) {
        #[cfg(windows)]
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(cancel) = &state.cancel {
                cancel.store(true, Ordering::SeqCst);
            }
            state.installer.take();
        }
    }

    pub(super) fn cancel_download(&self) {
        #[cfg(windows)]
        if let Some(cancel) = &self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cancel
        {
            cancel.store(true, Ordering::SeqCst);
        }
    }

    pub(super) fn cancel_owner(&self, owner: &str) {
        #[cfg(windows)]
        {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.owner.as_deref() == Some(owner) {
                if let Some(cancel) = &state.cancel {
                    cancel.store(true, Ordering::SeqCst);
                }
                state.installer.take();
                if state.phase == "downloaded" {
                    state.phase = "";
                }
            }
        }
        #[cfg(not(windows))]
        let _ = owner;
    }
}

#[tauri::command]
pub(crate) async fn recheck_npcap(
    window: WebviewWindow,
    runtime: State<'_, EasyTierRuntime>,
) -> Result<NpcapStatus, EasyTierError> {
    authorize(&window)?;
    let _gate = runtime
        .gate
        .try_lock()
        .map_err(|_| failure("busy", "EasyTier operation is already running"))?;
    ensure_idle(&runtime)?;
    #[cfg(windows)]
    windows::recheck(&runtime.npcap.state).await?;
    Ok(runtime.npcap.status())
}

#[tauri::command]
pub(crate) fn cancel_npcap_download(
    window: WebviewWindow,
    runtime: State<'_, EasyTierRuntime>,
) -> Result<(), EasyTierError> {
    authorize(&window)?;
    #[cfg(windows)]
    {
        let state = runtime
            .npcap
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.owner.as_deref() == Some(window.label()) {
            if let Some(cancel) = &state.cancel {
                cancel.store(true, Ordering::SeqCst);
            }
        }
    }
    #[cfg(not(windows))]
    let _ = runtime;
    Ok(())
}

/// Windows leases hold a per-run directory and a read-only pin on the locally
/// installed DLL. They are not copied into any public installer or archive.
pub(super) struct RunFiles {
    pub(super) directory: PathBuf,
    #[cfg(windows)]
    _lease: windows::RunLease,
}

pub(super) async fn prepare_run(
    root: &Path,
    directory: PathBuf,
) -> Result<RunFiles, EasyTierError> {
    #[cfg(windows)]
    {
        let (directory, lease) = windows::prepare_run(root, &directory).await?;
        Ok(RunFiles {
            directory,
            _lease: lease,
        })
    }
    #[cfg(not(windows))]
    {
        let _ = root;
        Ok(RunFiles { directory })
    }
}

#[tauri::command]
pub(crate) async fn download_npcap_installer(
    window: WebviewWindow,
    state: State<'_, AppState>,
    runtime: State<'_, EasyTierRuntime>,
) -> Result<NpcapStatus, EasyTierError> {
    authorize(&window)?;
    #[cfg(not(windows))]
    {
        let _ = (state, runtime);
        Err(failure(
            "unavailable",
            "Npcap setup is only needed on Windows",
        ))
    }
    #[cfg(windows)]
    {
        let _gate = runtime
            .gate
            .try_lock()
            .map_err(|_| failure("busy", "EasyTier operation is already running"))?;
        ensure_idle(&runtime)?;
        update_commands::ensure_not_updating(&state).map_err(|e| failure("busy", e))?;
        windows::confirm(&window, false).await?;
        ensure_idle(&runtime)?;
        update_commands::ensure_not_updating(&state).map_err(|e| failure("busy", e))?;
        if window
            .app_handle()
            .get_webview_window(window.label())
            .is_none()
        {
            return Err(failure("cancelled", "Workspace window closed"));
        }
        setup::download(&root(&state)?, window.label(), &runtime.npcap.state).await?;
        Ok(runtime.npcap.status())
    }
}

#[tauri::command]
pub(crate) async fn launch_npcap_installer(
    window: WebviewWindow,
    state: State<'_, AppState>,
    runtime: State<'_, EasyTierRuntime>,
) -> Result<NpcapStatus, EasyTierError> {
    authorize(&window)?;
    #[cfg(not(windows))]
    {
        let _ = (state, runtime);
        Err(failure(
            "unavailable",
            "Npcap setup is only needed on Windows",
        ))
    }
    #[cfg(windows)]
    {
        let _gate = runtime
            .gate
            .try_lock()
            .map_err(|_| failure("busy", "EasyTier operation is already running"))?;
        ensure_idle(&runtime)?;
        update_commands::ensure_not_updating(&state).map_err(|e| failure("busy", e))?;
        windows::confirm(&window, true).await?;
        ensure_idle(&runtime)?;
        update_commands::ensure_not_updating(&state).map_err(|e| failure("busy", e))?;
        if window
            .app_handle()
            .get_webview_window(window.label())
            .is_none()
        {
            return Err(failure("cancelled", "Workspace window closed"));
        }
        if let Err(error) = windows::launch_installer(
            window.label(),
            Arc::clone(&runtime.npcap.state),
            &state,
            &runtime,
        )
        .await
        {
            let mut setup = runtime
                .npcap
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if setup.owner.as_deref() == Some(window.label()) {
                setup.phase = "failed";
                setup.error = Some(error.clone());
            }
            return Err(error);
        }
        Ok(runtime.npcap.status())
    }
}

#[cfg(all(test, not(feature = "desktop")))]
#[path = "../../../test/rust/portmate/unit/npcap_authorization.rs"]
mod authorization_tests;
