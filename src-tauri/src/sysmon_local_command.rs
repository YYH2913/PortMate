use super::*;

pub(super) const LOCAL_SYSMON_COMMAND_TIMEOUT: Duration = Duration::from_secs(12);
pub(super) const MAX_LOCAL_SYSMON_STDOUT_BYTES: usize = 4 * 1024 * 1024;
pub(super) const MAX_LOCAL_SYSMON_STDERR_BYTES: usize = 64 * 1024;

pub(super) async fn exec_local_sysmon_command(
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> Result<String, String> {
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .map_err(|error| format!("无法启动本机 Sysmon 命令 {program}: {error}"))?;
    let process_id = child.id();
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "无法捕获本机 Sysmon stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "无法捕获本机 Sysmon stderr".to_string())?;
    let mut stdout_task = tokio::spawn(read_bounded_local_sysmon_output(
        stdout,
        MAX_LOCAL_SYSMON_STDOUT_BYTES,
        "stdout",
    ));
    let mut stderr_task = tokio::spawn(read_bounded_local_sysmon_output(
        stderr,
        MAX_LOCAL_SYSMON_STDERR_BYTES,
        "stderr",
    ));

    let (status, stdout, stderr) = match tokio::time::timeout(timeout, async {
        let status = child
            .wait()
            .await
            .map_err(|error| format!("等待本机 Sysmon 命令失败: {error}"))?;
        let stdout = (&mut stdout_task)
            .await
            .map_err(|error| format!("读取本机 Sysmon stdout 任务失败: {error}"))??;
        let stderr = (&mut stderr_task)
            .await
            .map_err(|error| format!("读取本机 Sysmon stderr 任务失败: {error}"))??;
        Ok::<_, String>((status, stdout, stderr))
    })
    .await
    {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => {
            terminate_local_sysmon_process_group(process_id);
            let _ = child.kill().await;
            stdout_task.abort();
            stderr_task.abort();
            return Err(error);
        }
        Err(_) => {
            terminate_local_sysmon_process_group(process_id);
            let _ = child.kill().await;
            stdout_task.abort();
            stderr_task.abort();
            return Err(format!(
                "本机 Sysmon 命令在 {} ms 后超时",
                timeout.as_millis()
            ));
        }
    };
    if !status.success() {
        return Err(format!(
            "本机 Sysmon 命令返回状态 {:?}: {}",
            status.code(),
            String::from_utf8_lossy(&stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&stdout).to_string())
}

#[cfg(unix)]
pub(super) fn terminate_local_sysmon_process_group(process_id: Option<u32>) {
    let Some(process_id) = process_id.filter(|process_id| *process_id <= i32::MAX as u32) else {
        return;
    };
    // `process_group(0)` gives each local command its own group, so this only reaches
    // descendants started by the aborted Sysmon command.
    unsafe {
        libc::kill(-(process_id as i32), libc::SIGKILL);
    }
}

#[cfg(not(unix))]
pub(super) fn terminate_local_sysmon_process_group(_process_id: Option<u32>) {}

pub(super) async fn read_bounded_local_sysmon_output<R>(
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
            .map_err(|error| format!("读取本机 Sysmon {stream} 失败: {error}"))?;
        if count == 0 {
            break;
        }
        if overflow {
            continue;
        }
        let next_len = output
            .len()
            .checked_add(count)
            .ok_or_else(|| format!("本机 Sysmon {stream} 长度溢出"))?;
        if next_len > max_bytes {
            overflow = true;
        } else {
            output.extend_from_slice(&chunk[..count]);
        }
    }
    if overflow {
        Err(format!("本机 Sysmon {stream} 超过 {max_bytes} 字节上限"))
    } else {
        Ok(output)
    }
}
