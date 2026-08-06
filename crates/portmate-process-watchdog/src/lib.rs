use std::ffi::OsString;

const WATCHDOG_THREAD_NAME: &str = "parent-watchdog";

pub fn install_parent_watchdog_from_environment(variable_name: &str) -> Result<(), String> {
    validate_environment_variable_name(variable_name)?;
    let Some(raw_parent_pid) = std::env::var_os(variable_name) else {
        return Ok(());
    };
    let parent_pid = parse_parent_pid(variable_name, raw_parent_pid)?;
    install_parent_watchdog(parent_pid)
}

pub fn install_parent_watchdog(parent_pid: u32) -> Result<(), String> {
    if parent_pid == 0 {
        return Err("parent process ID must be positive".to_string());
    }

    #[cfg(unix)]
    {
        install_unix_parent_watchdog(parent_pid)
    }

    #[cfg(windows)]
    {
        install_windows_parent_watchdog(parent_pid)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = parent_pid;
        Err("parent process watchdog is unsupported on this platform".to_string())
    }
}

fn validate_environment_variable_name(variable_name: &str) -> Result<(), String> {
    if variable_name.is_empty() || variable_name.contains(['\0', '=']) {
        return Err("parent process ID environment variable name is invalid".to_string());
    }
    Ok(())
}

fn parse_parent_pid(variable_name: &str, raw_parent_pid: OsString) -> Result<u32, String> {
    let raw_parent_pid = raw_parent_pid
        .into_string()
        .map_err(|_| format!("{variable_name} must contain valid UTF-8"))?;
    if raw_parent_pid.is_empty() || !raw_parent_pid.as_bytes().iter().all(u8::is_ascii_digit) {
        return Err(format!(
            "{variable_name} must be a positive decimal process ID"
        ));
    }
    let parent_pid = raw_parent_pid
        .parse::<u64>()
        .map_err(|_| format!("{variable_name} exceeds the platform process ID range"))?;
    if parent_pid == 0 {
        return Err(format!("{variable_name} must be a positive process ID"));
    }
    u32::try_from(parent_pid)
        .map_err(|_| format!("{variable_name} exceeds the platform process ID range"))
}

#[cfg(unix)]
fn install_unix_parent_watchdog(parent_pid: u32) -> Result<(), String> {
    let parent_pid = libc::pid_t::try_from(parent_pid)
        .map_err(|_| "parent process ID exceeds the platform process ID range".to_string())?;
    // The managed child is spawned directly by its owner. A changed PPID means the owner
    // exited without getting a chance to run its normal child cleanup.
    if unsafe { libc::getppid() } != parent_pid {
        return Err("parent process is no longer available".to_string());
    }
    let _watchdog = std::thread::Builder::new()
        .name(WATCHDOG_THREAD_NAME.to_string())
        .spawn(move || loop {
            std::thread::sleep(std::time::Duration::from_secs(1));
            if unsafe { libc::getppid() } != parent_pid {
                std::process::exit(0);
            }
        })
        .map_err(|error| format!("could not start parent process watchdog: {error}"))?;
    Ok(())
}

#[cfg(windows)]
fn install_windows_parent_watchdog(parent_pid: u32) -> Result<(), String> {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, INFINITE, PROCESS_SYNCHRONIZE,
    };

    let parent = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, parent_pid) };
    if parent.is_null() {
        return Err(format!(
            "parent process {parent_pid} is no longer available"
        ));
    }
    let parent_handle = parent as usize;
    let watchdog = std::thread::Builder::new()
        .name(WATCHDOG_THREAD_NAME.to_string())
        .spawn(move || {
            let parent = parent_handle as HANDLE;
            let wait_result = unsafe { WaitForSingleObject(parent, INFINITE) };
            unsafe {
                CloseHandle(parent);
            }
            std::process::exit(if wait_result == WAIT_OBJECT_0 { 0 } else { 1 });
        });
    match watchdog {
        Ok(watchdog) => {
            drop(watchdog);
            Ok(())
        }
        Err(error) => {
            unsafe {
                CloseHandle(parent);
            }
            Err(format!("could not start parent process watchdog: {error}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VARIABLE_NAME: &str = "PORTMATE_TEST_PARENT_PID";

    #[test]
    fn parses_positive_decimal_process_ids() {
        assert_eq!(
            parse_parent_pid(VARIABLE_NAME, OsString::from("42")),
            Ok(42)
        );
        assert_eq!(
            parse_parent_pid(VARIABLE_NAME, OsString::from("00042")),
            Ok(42)
        );
    }

    #[test]
    fn rejects_empty_or_non_decimal_process_ids() {
        for value in ["", " ", "+1", "-1", "1.0", "pid"] {
            assert!(parse_parent_pid(VARIABLE_NAME, OsString::from(value)).is_err());
        }
    }

    #[test]
    fn rejects_zero_and_out_of_range_process_ids() {
        assert!(parse_parent_pid(VARIABLE_NAME, OsString::from("0")).is_err());
        assert!(parse_parent_pid(VARIABLE_NAME, OsString::from("4294967296")).is_err());
        assert!(parse_parent_pid(VARIABLE_NAME, OsString::from("18446744073709551616")).is_err());
        assert!(install_parent_watchdog(0).is_err());
    }

    #[test]
    fn rejects_invalid_environment_variable_names() {
        assert!(validate_environment_variable_name("").is_err());
        assert!(validate_environment_variable_name("PORTMATE=PID").is_err());
        assert!(validate_environment_variable_name("PORTMATE\0PID").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_process_ids_larger_than_unix_pid_t() {
        assert!(install_parent_watchdog(u32::MAX).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_non_utf8_process_ids() {
        use std::os::unix::ffi::OsStringExt;

        assert!(parse_parent_pid(VARIABLE_NAME, OsString::from_vec(vec![0xff])).is_err());
    }
}
