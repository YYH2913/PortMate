//! Manual, per-session elevation. Saved/imported settings are never consent.
use super::*;
use portmate_core::ShellConnection;
use std::ffi::OsString;

const MANUAL_CONSENT_REQUIRED: &str = "提权本地终端必须在桌面工作区中手动打开。";
const CONFIGURATION_CHANGED: &str = "提权终端配置已更改，请检查后重新连接。";

pub(super) fn authorize_launch(
    profile: &SessionProfile,
    authorized: Option<&ShellConnection>,
) -> Result<(), String> {
    match (&profile.connection, authorized) {
        (ConnectionConfig::Shell(shell), Some(expected)) if shell.elevated && shell == expected => {
            Ok(())
        }
        (_, Some(_)) => Err(CONFIGURATION_CHANGED.into()),
        (ConnectionConfig::Shell(shell), None) if shell.elevated => {
            Err(MANUAL_CONSENT_REQUIRED.into())
        }
        _ => Ok(()),
    }
}

pub(super) struct ElevationLaunch {
    pub(super) program: PathBuf,
    pub(super) args: Vec<OsString>,
    pub(super) cwd: Option<PathBuf>,
    pub(super) resources: Option<ElevationResources>,
}

// Field order matters: release all pinned handles before removing the directory.
// The Shell reaper owns this lease until its child has actually been reaped.
pub(super) struct ElevationResources {
    _file: fs::File,
    _directory_pin: BoundDirectory,
    _directory: tempfile::TempDir,
}

pub(super) fn prepare_launch(
    launch: &ShellLaunchPaths,
    args: &[String],
    data_directory: Option<&Path>,
) -> Result<ElevationLaunch, String> {
    let program = resolve_elevated_program(&launch.program)?;
    #[cfg(windows)]
    {
        let cwd = launch
            .cwd
            .clone()
            .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
            .or_else(|| std::env::current_dir().ok())
            .ok_or("Windows elevation requires a local working directory")?;
        let cwd = local_windows_working_directory(&cwd)?;
        let (helper, resources) = materialize_helper(data_directory)?;
        Ok(ElevationLaunch {
            program: helper,
            args: windows_arguments(&program, args),
            cwd: Some(cwd),
            resources: Some(resources),
        })
    }
    #[cfg(unix)]
    {
        let _ = data_directory;
        let sudo = ["/usr/bin/sudo", "/bin/sudo"]
            .into_iter()
            .map(PathBuf::from)
            .find(|path| is_executable_file(path))
            .ok_or("未找到 sudo；请先安装 sudo 再打开提权本地终端。")?;
        Ok(ElevationLaunch {
            program: sudo,
            args: sudo_arguments(&program, args),
            cwd: None,
            resources: None,
        })
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = (program, args, data_directory);
        Err("Local terminal elevation is not supported on this platform".into())
    }
}

fn resolve_elevated_program(program: &Path) -> Result<PathBuf, String> {
    if program.is_absolute() {
        return validate_elevated_program(program);
    }
    // Never resolve a relative executable against an untrusted working directory.
    if program.components().count() != 1
        || !matches!(
            program.components().next(),
            Some(std::path::Component::Normal(_))
        )
    {
        return Err(
            "Elevated shell requires an absolute executable path or a program on PATH".into(),
        );
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&paths).filter(|path| path.is_absolute()) {
            let candidate = directory.join(program);
            #[cfg(windows)]
            let candidates = if candidate.extension().is_none() {
                vec![
                    candidate.with_extension("exe"),
                    candidate.with_extension("com"),
                ]
            } else {
                vec![candidate]
            };
            #[cfg(not(windows))]
            let candidates = [candidate];
            for candidate in candidates {
                if let Ok(program) = validate_elevated_program(&candidate) {
                    return Ok(program);
                }
            }
        }
    }
    Err(format!(
        "Elevated shell executable was not found on PATH: {}",
        program.display()
    ))
}

fn validate_elevated_program(program: &Path) -> Result<PathBuf, String> {
    #[cfg(windows)]
    require_local_windows_path(program)?;
    if !is_executable_file(program) {
        return Err(format!(
            "Elevated shell executable is unavailable: {}",
            program.display()
        ));
    }
    #[cfg(windows)]
    let program = fs::canonicalize(program).map_err(|error| error.to_string())?;
    #[cfg(not(windows))]
    let program = program.to_path_buf();
    #[cfg(windows)]
    {
        require_local_windows_path(&program)?;
        let extension = program
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if !extension.eq_ignore_ascii_case("exe") && !extension.eq_ignore_ascii_case("com") {
            // gsudo otherwise falls back to cmd /c, changing argument semantics.
            return Err(
                "Windows 提权终端必须使用本地 .exe 或 .com 程序，不能直接使用脚本。".into(),
            );
        }
    }
    Ok(program)
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(any(windows, test))]
fn windows_arguments(program: &Path, args: &[String]) -> Vec<OsString> {
    [
        "--direct",
        "--inline",
        "--close",
        "--wait",
        "--integrity",
        "High",
        "--",
    ]
    .into_iter()
    .map(OsString::from)
    .chain(std::iter::once(program.as_os_str().to_owned()))
    .chain(args.iter().map(OsString::from))
    .collect()
}

#[cfg(any(unix, test))]
fn sudo_arguments(program: &Path, args: &[String]) -> Vec<OsString> {
    // -k asks sudo to authenticate afresh (subject to sudoers); -H avoids using
    // the unprivileged user's HOME for root shell configuration/history.
    [
        "-k",
        "-H",
        "-u",
        "root",
        "-p",
        "PortMate sudo password: ",
        "--",
    ]
    .into_iter()
    .map(OsString::from)
    .chain(std::iter::once(program.as_os_str().to_owned()))
    .chain(args.iter().map(OsString::from))
    .collect()
}

#[cfg(windows)]
fn require_local_windows_path(path: &Path) -> Result<(), String> {
    use std::path::{Component, Prefix};
    if path.is_absolute()
        && matches!(path.components().next(), Some(Component::Prefix(prefix))
        if matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)))
    {
        Ok(())
    } else {
        Err("Windows elevated shell requires local drive paths; UNC and device paths are not supported".into())
    }
}

#[cfg(windows)]
fn local_windows_working_directory(path: &Path) -> Result<PathBuf, String> {
    use std::path::{Component, Prefix};
    let path = std::path::absolute(path).map_err(|error| error.to_string())?;
    // Reject direct UNC/device paths before metadata access can contact a server.
    require_local_windows_path(&path)?;
    let path = fs::canonicalize(path).map_err(|error| error.to_string())?;
    require_local_windows_path(&path)?;
    // canonicalize returns \\?\C:\...; gsudo interprets a leading \\ as a
    // network cwd and wraps cmd with pushd/popd. Use the equivalent DOS path.
    let mut components = path.components();
    if let Some(Component::Prefix(prefix)) = components.next() {
        if let Prefix::VerbatimDisk(drive) = prefix.kind() {
            let mut dos = PathBuf::from(format!("{}:\\", char::from(drive)));
            for component in components.filter(|part| !matches!(part, Component::RootDir)) {
                dos.push(component.as_os_str());
            }
            return Ok(dos);
        }
    }
    Ok(path)
}

#[cfg(all(windows, feature = "bundled-shell-elevation"))]
include!(concat!(env!("OUT_DIR"), "/shell_elevation_payload.rs"));

#[cfg(all(windows, not(feature = "bundled-shell-elevation")))]
fn materialize_helper(_directory: Option<&Path>) -> Result<(PathBuf, ElevationResources), String> {
    Err("此构建未包含 Windows 提权组件。".into())
}

#[cfg(all(windows, feature = "bundled-shell-elevation"))]
fn materialize_helper(directory: Option<&Path>) -> Result<(PathBuf, ElevationResources), String> {
    let directory =
        directory.ok_or("Windows elevation requires a local application data directory")?;
    require_local_windows_path(directory)?;
    // Do not follow a replaced parent directory during creation or launch.
    let _parent_pin = BoundDirectory::open(directory)
        .map_err(|error| format!("Cannot pin elevation directory: {error}"))?;
    let temporary = tempfile::Builder::new()
        .prefix("shell-elevation-")
        .tempdir_in(directory)
        .map_err(|error| format!("Cannot create elevation directory: {error}"))?;
    let directory_pin =
        BoundDirectory::open(temporary.path()).map_err(|error| error.to_string())?;
    let helper = temporary.path().join("gsudo.exe");
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&helper)
            .map_err(|error| error.to_string())?;
        file.write_all(HELPER_BYTES)
            .and_then(|()| file.sync_all())
            .map_err(|error| error.to_string())?;
    }
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temporary.path().join("gsudo-MIT.txt"))
        .and_then(|mut file| {
            file.write_all(include_bytes!("../../THIRD_PARTY_LICENSES/gsudo-MIT.txt"))
        })
        .map_err(|error| error.to_string())?;
    let file = pin_verified_helper(&helper, HELPER_SHA256)?;
    Ok((
        helper,
        ElevationResources {
            _file: file,
            _directory_pin: directory_pin,
            _directory: temporary,
        },
    ))
}

#[cfg(windows)]
fn pin_verified_helper(path: &Path, expected_sha256: &str) -> Result<fs::File, String> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_READ,
    };
    let mut file = OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|error| error.to_string())?;
    let metadata = file.metadata().map_err(|error| error.to_string())?;
    if !metadata.is_file()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || metadata.len() > 8 * 1024 * 1024
    {
        return Err("Windows elevation helper is not a regular local executable".into());
    }
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    let actual = portmate_core::encode_hex(&digest.finalize());
    if actual != expected_sha256 {
        return Err("Windows 提权组件完整性校验失败，已拒绝启动。".into());
    }
    Ok(file)
}

#[cfg(test)]
#[path = "../../test/rust/portmate/unit/shell_elevation.rs"]
mod tests;

#[cfg(test)]
include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../test/rust/portmate/support/shell_elevation.rs"
));
