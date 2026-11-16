use super::*;
use setup::{DownloadedInstaller, SetupState};
use std::os::windows::ffi::OsStringExt;

// Only the genuine system installation is considered. Never search PATH, the
// working directory, a registry-supplied executable, or an app-local Packet.dll.
fn system_directory() -> Result<PathBuf, EasyTierError> {
    let mut buffer = vec![0_u16; 32_768];
    let size = unsafe {
        windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW(
            buffer.as_mut_ptr(),
            buffer.len() as u32,
        )
    } as usize;
    if size == 0 || size >= buffer.len() {
        return Err(failure(
            "npcap-required",
            "Windows system directory is unavailable",
        ));
    }
    Ok(PathBuf::from(std::ffi::OsString::from_wide(
        &buffer[..size],
    )))
}

fn installed_packet() -> Result<PathBuf, EasyTierError> {
    let system = system_directory()?;
    let packet = system.join("Npcap").join("Packet.dll");
    let driver = system.join("drivers").join("npcap.sys");
    if !regular_file(&packet) || !regular_file(&driver) {
        return Err(failure(
            "npcap-required",
            "Install Npcap using its official interactive installer, then recheck",
        ));
    }
    Ok(packet)
}

fn regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.file_attributes() & 0x400 == 0)
}

pub(super) fn detected() -> bool {
    installed_packet().is_ok()
}

pub(super) fn pin_file(path: &Path) -> Result<fs::File, EasyTierError> {
    let file = OpenOptions::new()
        .read(true)
        .share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ)
        .custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|e| failure("npcap-verification", e))?;
    let metadata = file
        .metadata()
        .map_err(|e| failure("npcap-verification", e))?;
    if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
        return Err(failure(
            "npcap-verification",
            "Npcap must be a regular local file, not a reparse point",
        ));
    }
    Ok(file)
}

const VERIFY_SIGNATURE: &str = r#"
$ErrorActionPreference = 'Stop'
$signature = Get-AuthenticodeSignature -LiteralPath $env:PORTMATE_NPCAP_FILE
if ($signature.Status -ne 'Valid' -or -not $signature.SignerCertificate) { throw 'Npcap Authenticode signature is not valid' }
$publisher = $signature.SignerCertificate.GetNameInfo([System.Security.Cryptography.X509Certificates.X509NameType]::SimpleName, $false)
if (@('Nmap Software LLC', 'Insecure.Com LLC') -cnotcontains $publisher) { throw 'Unexpected Npcap publisher' }
"#;

const LAUNCH_INTERACTIVE: &str = r#"
$hash = (Get-FileHash -LiteralPath $env:PORTMATE_NPCAP_FILE -Algorithm SHA256).Hash.ToLowerInvariant()
if ($hash -cne '682b57e1c05ef375931975c9d6ded40888f4bfdf9f4b11571e654f962f1199b6') { throw 'Npcap installer hash changed' }
# Deliberately no ArgumentList: no silent switches, EULA acceptance or driver defaults.
$installer = Start-Process -FilePath $env:PORTMATE_NPCAP_FILE -Verb RunAs -PassThru -Wait -ErrorAction Stop
if ($installer.ExitCode -eq 3010) { Write-Output 'reboot-required' }
elseif ($installer.ExitCode -ne 0) { throw 'Npcap installer was cancelled or did not succeed' }
else { Write-Output 'completed' }
"#;

fn powershell(path: &Path, script: &str) -> Result<tokio::process::Command, EasyTierError> {
    let system = system_directory()?;
    let windows = system
        .parent()
        .ok_or_else(|| failure("npcap-verification", "Invalid Windows directory"))?;
    let powershell = system.join("WindowsPowerShell/v1.0/powershell.exe");
    let encoded = BASE64_STANDARD.encode(
        script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let mut command = tokio::process::Command::new(powershell);
    command
        .env_clear()
        .env("SystemRoot", windows)
        .env("WINDIR", windows)
        .env("PATH", &system)
        .env(
            "PSModulePath",
            system.join("WindowsPowerShell/v1.0/Modules"),
        )
        .env("PORTMATE_NPCAP_FILE", path)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
            &encoded,
        ])
        .current_dir(&system)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    Ok(command)
}

pub(super) async fn verify_signature(path: &Path) -> Result<(), EasyTierError> {
    let mut command = powershell(path, VERIFY_SIGNATURE)?;
    command.kill_on_drop(true);
    let (mut child, _tree) =
        host_script_process::spawn(&mut command).map_err(|e| failure("npcap-verification", e))?;
    let out = child
        .stdout
        .take()
        .ok_or_else(|| failure("npcap-verification", "Missing verification output"))?;
    let err = child
        .stderr
        .take()
        .ok_or_else(|| failure("npcap-verification", "Missing verification diagnostics"))?;
    tokio::time::timeout(Duration::from_secs(45), async {
        let (status, _, _) = tokio::try_join!(
            async {
                child
                    .wait()
                    .await
                    .map_err(|e| failure("npcap-verification", e))
            },
            bounded_output(out, 4096),
            bounded_output(err, 16 * 1024),
        )?;
        if !status.success() {
            return Err(failure(
                "npcap-verification",
                "Npcap signature or publisher verification failed; nothing was launched",
            ));
        }
        Ok(())
    })
    .await
    .map_err(|_| {
        failure(
            "npcap-verification",
            "Npcap signature verification timed out",
        )
    })?
}

pub(super) async fn confirm(
    window: &WebviewWindow,
    installation: bool,
) -> Result<(), EasyTierError> {
    use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
    let app = window.app_handle().clone();
    let window = window.clone();
    let message = if installation {
        "Open the official Npcap installer?\n\nInstallation is interactive: review and accept Npcap's own license and Windows UAC yourself. PortMate will not silently accept the EULA or select driver options.\n\n是否打开 Npcap 官方交互式安装器？请自行阅读并确认其许可与 Windows UAC；PortMate 不代替接受许可或选择驱动选项。"
    } else {
        "Download Npcap 1.90 directly from https://npcap.com/dist/npcap-1.90.exe ?\n\nNpcap is separate software by the Nmap Project, not bundled with PortMate. Its free license normally permits up to five installations, with exceptions. Downloading does not accept its EULA. Review https://github.com/nmap/npcap/blob/v1.90/LICENSE .\n\n是否从 Npcap 官方下载？Npcap 是独立许可组件，免费许可通常最多允许五台安装（另有例外），下载不代表接受安装许可。"
    };
    let allowed = tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .message(message)
            .parent(&window)
            .title("Npcap / Nmap Project")
            .kind(MessageDialogKind::Info)
            .buttons(MessageDialogButtons::OkCancel)
            .blocking_show()
    })
    .await
    .map_err(|e| failure("npcap-setup", e))?;
    if allowed {
        Ok(())
    } else {
        Err(failure("cancelled", "Npcap setup cancelled by the user"))
    }
}

pub(super) async fn launch_installer(
    owner: &str,
    shared: Arc<Mutex<SetupState>>,
    app: &AppState,
    runtime: &EasyTierRuntime,
) -> Result<(), EasyTierError> {
    let installer = {
        let mut state = shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !state
            .installer
            .as_ref()
            .is_some_and(|installer| installer.owner == owner)
        {
            return Err(failure(
                "npcap-setup",
                "Download the official installer from this workspace first",
            ));
        }
        state.installer.take().unwrap()
    };
    verify_signature(&installer.path).await?;
    setup::verify_file(&installer._file)?;
    let mut command = powershell(
        &installer.path,
        &format!("{VERIFY_SIGNATURE}\n{LAUNCH_INTERACTIVE}"),
    )?;
    ensure_idle(runtime)?;
    update_commands::ensure_not_updating(app).map_err(|e| failure("busy", e))?;
    if app
        .app_handle
        .as_ref()
        .is_none_or(|app| app.get_webview_window(owner).is_none())
    {
        return Err(failure("cancelled", "Workspace window closed"));
    }
    // This is the explicitly launched, independent installer, not a managed
    // networking child: never terminate it halfway through driver installation.
    command.kill_on_drop(false);
    let mut child = command.spawn().map_err(|e| failure("npcap-setup", e))?;
    let out = child
        .stdout
        .take()
        .ok_or_else(|| failure("npcap-setup", "Missing installer output"))?;
    let err = child
        .stderr
        .take()
        .ok_or_else(|| failure("npcap-setup", "Missing installer diagnostics"))?;
    {
        let mut state = shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.phase = "installing";
        state.error = None;
    }
    tokio::spawn(async move {
        let _installer: DownloadedInstaller = installer;
        let result = tokio::try_join!(
            async { child.wait().await.map_err(|e| failure("npcap-setup", e)) },
            bounded_output(out, 4096),
            bounded_output(err, 16 * 1024),
        );
        let mut state = shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match result {
            Ok((status, output, _)) if status.success() => {
                state.phase = if String::from_utf8_lossy(&output).contains("reboot-required") {
                    "reboot-required"
                } else {
                    "completed"
                };
            }
            _ => {
                state.phase = "failed";
                state.error = Some(failure(
                    "npcap-setup",
                    "Npcap installation was cancelled, failed or was denied; recheck or retry",
                ));
            }
        }
    });
    Ok(())
}

pub(super) async fn recheck(shared: &Arc<Mutex<SetupState>>) -> Result<(), EasyTierError> {
    let result = async {
        let packet = installed_packet()?;
        let _pin = pin_file(&packet)?;
        verify_signature(&packet).await
    }
    .await;
    let mut state = shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state.error = result.as_ref().err().cloned();
    state.phase = if result.is_ok() {
        "detected"
    } else {
        "missing"
    };
    result
}

pub(super) struct RunLease {
    _source: fs::File,
    _packet: fs::File,
    _binaries: Vec<fs::File>,
    _pin: BoundDirectory,
    _directory: tempfile::TempDir,
}

pub(super) async fn prepare_run(
    root: &Path,
    bundled: &Path,
) -> Result<(PathBuf, RunLease), EasyTierError> {
    let source = installed_packet()?;
    let source_pin = pin_file(&source)?;
    verify_signature(&source).await?;
    let root = root.to_path_buf();
    let bundled = bundled.to_path_buf();
    tauri::async_runtime::spawn_blocking(move || {
        let directory = tempfile::Builder::new()
            .prefix("npcap-run-")
            .tempdir_in(&root)
            .map_err(|e| failure("npcap-setup", e))?;
        let pin = BoundDirectory::open(directory.path()).map_err(|e| failure("npcap-setup", e))?;
        let mut binaries = Vec::new();
        for file in bundled::FILES {
            let destination = directory.path().join(file.name);
            fs::copy(bundled.join(file.name), &destination)
                .map_err(|e| failure("npcap-setup", e))?;
            let pinned = pin_file(&destination)?;
            let bytes = fs::read(&destination).map_err(|e| failure("component-failed", e))?;
            if bytes.len() != file.bytes.len()
                || setup::hex_hash(&Sha256::digest(&bytes)) != file.sha256
            {
                return Err(failure(
                    "component-failed",
                    "Bundled EasyTier changed during preparation",
                ));
            }
            binaries.push(pinned);
        }
        for (name, bytes) in bundled::notice_files() {
            fs::write(directory.path().join(name), bytes)
                .map_err(|e| failure("component-failed", e))?;
        }
        let packet = directory.path().join("Packet.dll");
        let mut input = source_pin
            .try_clone()
            .map_err(|e| failure("npcap-setup", e))?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&packet)
            .map_err(|e| failure("npcap-setup", e))?;
        std::io::copy(&mut input, &mut output).map_err(|e| failure("npcap-setup", e))?;
        output.sync_all().map_err(|e| failure("npcap-setup", e))?;
        drop(output);
        let packet_pin = pin_file(&packet)?;
        let path = directory.path().to_path_buf();
        Ok((
            path,
            RunLease {
                _source: source_pin,
                _packet: packet_pin,
                _binaries: binaries,
                _pin: pin,
                _directory: directory,
            },
        ))
    })
    .await
    .map_err(|e| failure("npcap-setup", e))?
}

#[cfg(test)]
#[path = "../../../../test/rust/portmate/unit/npcap_windows.rs"]
mod tests;
