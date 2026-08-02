use super::*;

pub(crate) async fn collect_remote_sysmon(
    session_id: &str,
    handle: Arc<tokio::sync::Mutex<SshBackendSession>>,
) -> Result<SysmonSnapshot, String> {
    let platform = detect_remote_sysmon_platform(handle.clone()).await?;
    match platform
        .lines()
        .find(|line| !line.trim().is_empty())
        .map(str::trim)
    {
        Some("Linux") => {
            let output = exec_ssh_command_capture(
                handle.clone(),
                REMOTE_LINUX_SYSMON_COMMAND,
                Duration::from_secs(8),
            )
            .await?;
            let mut snapshot = parse_remote_sysmon_output(session_id, &output)?;
            let kernel_addresses = parse_remote_linux_kernel_network_addresses(&output);
            if remote_linux_sysmon_needs_network_address_fallback(&snapshot.network_interfaces)
                || remote_linux_kernel_addresses_need_merge(
                    &snapshot.network_interfaces,
                    &kernel_addresses,
                )
            {
                merge_remote_linux_sysmon_network_addresses(
                    &mut snapshot.network_interfaces,
                    &mut snapshot.rx_kbps,
                    &mut snapshot.tx_kbps,
                    &output,
                    kernel_addresses,
                );
            }
            if remote_linux_sysmon_needs_network_address_fallback(&snapshot.network_interfaces) {
                if let Ok(ubus_output) = exec_ssh_command_capture(
                    handle,
                    REMOTE_OPENWRT_SYSMON_NETWORK_COMMAND,
                    Duration::from_secs(3),
                )
                .await
                {
                    merge_remote_linux_sysmon_network_addresses(
                        &mut snapshot.network_interfaces,
                        &mut snapshot.rx_kbps,
                        &mut snapshot.tx_kbps,
                        &output,
                        parse_openwrt_network_interface_dump(&ubus_output),
                    );
                }
            }
            Ok(snapshot)
        }
        Some("Darwin") => {
            let output = exec_ssh_command_capture(
                handle,
                REMOTE_MACOS_SYSMON_COMMAND,
                Duration::from_secs(8),
            )
            .await?;
            parse_remote_macos_sysmon_output(session_id, &output)
        }
        Some("FreeBSD") => {
            let output = exec_ssh_command_capture(
                handle,
                REMOTE_FREEBSD_SYSMON_COMMAND,
                Duration::from_secs(8),
            )
            .await?;
            parse_remote_freebsd_sysmon_output(session_id, &output)
        }
        Some("Windows") => {
            let command = windows_powershell_command(REMOTE_WINDOWS_SYSMON_SCRIPT);
            let output =
                exec_ssh_command_capture(handle, &command, Duration::from_secs(12)).await?;
            parse_remote_windows_sysmon_output(session_id, &output)
        }
        Some(platform) => Err(format!(
            "远端 Sysmon 暂不支持 {}",
            bounded_sysmon_label(platform, 64)
        )),
        None => Err("远端 Sysmon 无法识别操作系统".to_string()),
    }
}

pub(crate) async fn detect_remote_sysmon_platform(
    handle: Arc<tokio::sync::Mutex<SshBackendSession>>,
) -> Result<String, String> {
    if let Ok(output) = exec_ssh_command_capture(
        handle.clone(),
        REMOTE_SYSMON_PLATFORM_COMMAND,
        Duration::from_secs(3),
    )
    .await
    {
        if let Some(platform) = remote_sysmon_platform_label(&output) {
            return Ok(platform);
        }
    }

    let command = windows_powershell_command(REMOTE_WINDOWS_PLATFORM_SCRIPT);
    let output = exec_ssh_command_capture(handle, &command, Duration::from_secs(4))
        .await
        .map_err(|_| {
            "远端 Sysmon 无法识别操作系统（uname 与 Windows PowerShell 探测均失败）".to_string()
        })?;
    remote_sysmon_platform_label(&output)
        .map(|platform| {
            if platform.eq_ignore_ascii_case("Win32NT") {
                "Windows".to_string()
            } else {
                platform
            }
        })
        .ok_or_else(|| "远端 Sysmon 无法识别操作系统".to_string())
}
