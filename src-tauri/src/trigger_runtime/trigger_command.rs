use super::*;

const TRIGGER_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const TRIGGER_COMMAND_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_TRIGGER_COMMAND_STDOUT_BYTES: usize = 64 * 1024;
const MAX_TRIGGER_COMMAND_STDERR_BYTES: usize = 64 * 1024;

pub(super) async fn run_shell_command(command: &str) -> Result<(i32, String, String), String> {
    run_shell_command_bounded(
        command,
        TRIGGER_COMMAND_TIMEOUT,
        MAX_TRIGGER_COMMAND_STDOUT_BYTES,
        MAX_TRIGGER_COMMAND_STDERR_BYTES,
    )
    .await
}

pub(crate) async fn run_shell_command_bounded(
    command: &str,
    timeout: Duration,
    max_stdout_bytes: usize,
    max_stderr_bytes: usize,
) -> Result<(i32, String, String), String> {
    #[cfg(windows)]
    let mut process = {
        let mut process = tokio::process::Command::new("cmd");
        process.args(["/D", "/S", "/C", command]);
        process
    };
    #[cfg(not(windows))]
    let mut process = {
        let mut process = tokio::process::Command::new("sh");
        process.args(["-lc", command]);
        process
    };
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        process.as_std_mut().process_group(0);
    }
    let mut child = process
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("could not start trigger command: {error}"))?;
    let process_id = child.id();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "could not capture trigger command stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "could not capture trigger command stderr".to_string())?;
    let mut stdout_task = tokio::spawn(read_bounded_trigger_command_output(
        stdout,
        max_stdout_bytes,
        "stdout",
    ));
    let mut stderr_task = tokio::spawn(read_bounded_trigger_command_output(
        stderr,
        max_stderr_bytes,
        "stderr",
    ));
    let started = Instant::now();
    let status = match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            let cleanup_warning = terminate_trigger_command(&mut child, process_id).await;
            stdout_task.abort();
            stderr_task.abort();
            return Err(format!(
                "could not wait for trigger command: {error}{}",
                trigger_command_cleanup_suffix(cleanup_warning)
            ));
        }
        Err(_) => {
            let cleanup_warning = terminate_trigger_command(&mut child, process_id).await;
            stdout_task.abort();
            stderr_task.abort();
            return Err(format!(
                "trigger command timed out after {} ms{}",
                timeout.as_millis(),
                trigger_command_cleanup_suffix(cleanup_warning)
            ));
        }
    };
    let Some(remaining) = timeout.checked_sub(started.elapsed()) else {
        let cleanup_warning = terminate_trigger_command(&mut child, process_id).await;
        stdout_task.abort();
        stderr_task.abort();
        return Err(format!(
            "trigger command output timed out after {} ms{}",
            timeout.as_millis(),
            trigger_command_cleanup_suffix(cleanup_warning)
        ));
    };
    let outputs = tokio::time::timeout(remaining, async {
        let (stdout, stderr) = tokio::try_join!(&mut stdout_task, &mut stderr_task)
            .map_err(|error| format!("trigger command output task failed: {error}"))?;
        Ok::<_, String>((stdout?, stderr?))
    })
    .await;
    let (stdout, stderr) = match outputs {
        Ok(Ok(outputs)) => outputs,
        Ok(Err(error)) => {
            stdout_task.abort();
            stderr_task.abort();
            return Err(error);
        }
        Err(_) => {
            let cleanup_warning = terminate_trigger_command(&mut child, process_id).await;
            stdout_task.abort();
            stderr_task.abort();
            return Err(format!(
                "trigger command output timed out after {} ms{}",
                timeout.as_millis(),
                trigger_command_cleanup_suffix(cleanup_warning)
            ));
        }
    };
    Ok((
        status.code().unwrap_or(-1),
        String::from_utf8_lossy(&stdout).to_string(),
        String::from_utf8_lossy(&stderr).to_string(),
    ))
}

async fn terminate_trigger_command(
    child: &mut tokio::process::Child,
    process_id: Option<u32>,
) -> Option<String> {
    let mut warnings = Vec::new();
    #[cfg(unix)]
    if let Some(process_id) = process_id.filter(|process_id| *process_id <= i32::MAX as u32) {
        let result = unsafe { libc::kill(-(process_id as i32), libc::SIGKILL) };
        if result != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                warnings.push(format!("process group termination failed: {error}"));
            }
        }
    }
    #[cfg(not(unix))]
    let _ = process_id;
    match tokio::time::timeout(TRIGGER_COMMAND_CLEANUP_TIMEOUT, child.kill()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) if error.kind() == std::io::ErrorKind::InvalidInput => {}
        Ok(Err(error)) => warnings.push(format!("child cleanup failed: {error}")),
        Err(_) => warnings.push(format!(
            "child cleanup timed out after {} ms",
            TRIGGER_COMMAND_CLEANUP_TIMEOUT.as_millis()
        )),
    }
    (!warnings.is_empty()).then(|| warnings.join("; "))
}

fn trigger_command_cleanup_suffix(warning: Option<String>) -> String {
    warning
        .map(|warning| format!("; {warning}"))
        .unwrap_or_default()
}

async fn read_bounded_trigger_command_output<R>(
    mut reader: R,
    max_bytes: usize,
    stream: &'static str,
) -> Result<Vec<u8>, String>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut output = Vec::new();
    let mut overflow = false;
    let mut chunk = [0_u8; 8192];
    loop {
        let count = reader
            .read(&mut chunk)
            .await
            .map_err(|error| format!("could not read trigger command {stream}: {error}"))?;
        if count == 0 {
            break;
        }
        if overflow {
            continue;
        }
        let next_len = output
            .len()
            .checked_add(count)
            .ok_or_else(|| format!("trigger command {stream} length overflow"))?;
        if next_len > max_bytes {
            overflow = true;
        } else {
            output.extend_from_slice(&chunk[..count]);
        }
    }
    if overflow {
        Err(format!(
            "trigger command {stream} exceeded {max_bytes} byte limit"
        ))
    } else {
        Ok(output)
    }
}
