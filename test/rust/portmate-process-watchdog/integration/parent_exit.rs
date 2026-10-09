#![cfg(any(unix, windows))]

use std::fs;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const TEST_NAME: &str = "watchdog_exits_after_parent_process_terminates";
const TEST_ROLE_ENV: &str = "PORTMATE_PROCESS_WATCHDOG_TEST_ROLE";
const TEST_ROOT_ENV: &str = "PORTMATE_PROCESS_WATCHDOG_TEST_ROOT";
const TEST_PARENT_PID_ENV: &str = "PORTMATE_PROCESS_WATCHDOG_TEST_PARENT_PID";
const HELPER_READY_TIMEOUT: Duration = Duration::from_secs(5);
const WATCHDOG_EXIT_TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn watchdog_exits_after_parent_process_terminates() {
    match std::env::var(TEST_ROLE_ENV) {
        Ok(role) if role == "parent" => run_parent_helper(),
        Ok(role) if role == "child" => run_watched_child(),
        Ok(role) => panic!("unknown watchdog test role: {role}"),
        Err(std::env::VarError::NotPresent) => run_test_controller(),
        Err(std::env::VarError::NotUnicode(_)) => panic!("watchdog test role is not valid UTF-8"),
    }
}

fn run_test_controller() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "portmate-process-watchdog-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();

    let output = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg(TEST_NAME)
        .arg("--nocapture")
        .env(TEST_ROLE_ENV, "parent")
        .env(TEST_ROOT_ENV, &root)
        .stdin(Stdio::null())
        .output()
        .unwrap();

    let pid_path = root.join("child.pid");
    let child_pid = fs::read_to_string(&pid_path)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok());
    if !output.status.success() {
        if let Some(child_pid) = child_pid {
            terminate_process(child_pid);
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        panic!("watchdog parent helper failed:\nstdout: {stdout}\nstderr: {stderr}");
    }
    let child_pid = child_pid.expect("watchdog parent helper did not publish its child PID");

    let deadline = Instant::now() + WATCHDOG_EXIT_TIMEOUT;
    while process_exists(child_pid) {
        if Instant::now() >= deadline {
            terminate_process(child_pid);
            panic!("watched process {child_pid} survived its parent");
        }
        thread::sleep(Duration::from_millis(20));
    }

    fs::remove_dir_all(root).unwrap();
}

fn run_parent_helper() {
    let root = test_root();
    let ready_path = root.join("child.ready");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg(TEST_NAME)
        .arg("--nocapture")
        .env(TEST_ROLE_ENV, "child")
        .env(TEST_ROOT_ENV, &root)
        .env(TEST_PARENT_PID_ENV, std::process::id().to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let deadline = Instant::now() + HELPER_READY_TIMEOUT;
    loop {
        if ready_path.is_file() {
            break;
        }
        if let Some(status) = child.try_wait().unwrap() {
            panic!("watched child exited before becoming ready: {status}");
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("watched child did not become ready");
        }
        thread::sleep(Duration::from_millis(20));
    }

    fs::write(root.join("child.pid"), child.id().to_string()).unwrap();
    assert!(
        child.try_wait().unwrap().is_none(),
        "watched child exited while its parent was alive"
    );
}

fn run_watched_child() {
    let parent_pid = std::env::var(TEST_PARENT_PID_ENV)
        .unwrap()
        .parse::<u32>()
        .unwrap();
    portmate_process_watchdog::install_parent_watchdog(parent_pid).unwrap();
    fs::write(test_root().join("child.ready"), b"ready").unwrap();
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

fn test_root() -> std::path::PathBuf {
    std::env::var_os(TEST_ROOT_ENV)
        .map(std::path::PathBuf::from)
        .expect("watchdog test root is missing")
}

#[cfg(unix)]
fn process_exists(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

#[cfg(unix)]
fn terminate_process(pid: u32) {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return;
    };
    if unsafe { libc::kill(pid, 0) } == 0 {
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
}

#[cfg(windows)]
fn process_exists(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
    };

    let process = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
    if process.is_null() {
        return false;
    }
    let wait_result = unsafe { WaitForSingleObject(process, 0) };
    unsafe {
        CloseHandle(process);
    }
    wait_result == WAIT_TIMEOUT
}

#[cfg(windows)]
fn terminate_process(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

    let process = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
    if process.is_null() {
        return;
    }
    unsafe {
        TerminateProcess(process, 1);
        CloseHandle(process);
    }
}
