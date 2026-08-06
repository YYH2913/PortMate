#![cfg(unix)]

use std::fs;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const WATCHDOG_EXIT_TIMEOUT: Duration = Duration::from_secs(5);

#[test]
fn managed_http_sidecar_exits_after_its_parent_dies() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "portmate-mcp-parent-watchdog-{}-{unique}",
        std::process::id()
    ));
    fs::create_dir_all(&root).unwrap();
    let pid_path = root.join("sidecar.pid");
    let stderr_path = root.join("sidecar.stderr");

    let parent = Command::new("sh")
        .arg("-c")
        .arg(
            r#"
PORTMATE_MCP_PARENT_PID=$$ \
PORTMATE_MCP_HTTP=1 \
PORTMATE_MCP_HTTP_ADDR=127.0.0.1:0 \
PORTMATE_MCP_HTTP_TOKEN=portmate-parent-watchdog-test-token \
"$1" --http </dev/null >/dev/null 2>"$3" &
child=$!
printf '%s\n' "$child" >"$2"
sleep 1
kill -0 "$child"
"#,
        )
        .arg("portmate-watchdog-parent")
        .arg(env!("CARGO_BIN_EXE_portmate-mcp"))
        .arg(&pid_path)
        .arg(&stderr_path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();

    let pid = fs::read_to_string(&pid_path)
        .unwrap()
        .trim()
        .parse::<libc::pid_t>()
        .unwrap();
    if !parent.success() {
        terminate_process(pid);
        let diagnostics = fs::read_to_string(&stderr_path).unwrap_or_default();
        panic!("watchdog parent could not keep the sidecar alive: {diagnostics}");
    }

    let deadline = Instant::now() + WATCHDOG_EXIT_TIMEOUT;
    while process_exists(pid) {
        if Instant::now() >= deadline {
            terminate_process(pid);
            let diagnostics = fs::read_to_string(&stderr_path).unwrap_or_default();
            panic!("managed MCP HTTP sidecar {pid} survived its parent: {diagnostics}");
        }
        thread::sleep(Duration::from_millis(20));
    }

    fs::remove_dir_all(root).unwrap();
}

fn process_exists(pid: libc::pid_t) -> bool {
    if unsafe { libc::kill(pid, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

fn terminate_process(pid: libc::pid_t) {
    if process_exists(pid) {
        unsafe {
            libc::kill(pid, libc::SIGKILL);
        }
    }
}
