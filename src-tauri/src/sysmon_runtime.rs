use super::*;

pub(super) const MAX_SYSMON_HISTORY_QUERY_LIMIT: usize = 240;
pub(super) const MAX_SYSMON_PROCESSES: usize = 8;
pub(super) const MAX_SYSMON_DISKS: usize = 16;
pub(super) const LOCAL_SYSMON_SAMPLE_SECONDS: f32 = 0.12;
pub(super) const LOCAL_SYSMON_COMMAND_TIMEOUT: Duration = Duration::from_secs(12);
pub(super) const MAX_CONCURRENT_SYSMON_REFRESHES: usize = 4;
pub(super) const MAX_LOCAL_SYSMON_STDOUT_BYTES: usize = 4 * 1024 * 1024;
pub(super) const MAX_LOCAL_SYSMON_STDERR_BYTES: usize = 64 * 1024;
pub(super) const REMOTE_OPENWRT_SYSMON_NETWORK_COMMAND: &str = r#"sh -c 'PATH=/usr/bin:/bin:/usr/sbin:/sbin:$PATH; export PATH; ubus call network.interface dump 2>/dev/null'"#;
pub(super) const REMOTE_SYSMON_PLATFORM_COMMAND: &str = r#"sh -c 'PATH=/usr/bin:/bin:/usr/sbin:/sbin:$PATH; export PATH; uname -s 2>/dev/null | head -n 1'"#;
pub(super) const REMOTE_LINUX_SYSMON_COMMAND: &str = r#"sh -c 'PATH=/usr/bin:/bin:/usr/sbin:/sbin:$PATH; export PATH LC_ALL=C; head -n 1 /proc/uptime 2>/dev/null; echo __PORTMATE_MEMINFO__; head -n 64 /proc/meminfo 2>/dev/null; echo __PORTMATE_STAT1__; head -n 1 /proc/stat 2>/dev/null; echo __PORTMATE_NET1__; head -n 258 /proc/net/dev 2>/dev/null; sleep 0.2; echo __PORTMATE_STAT2__; head -n 1 /proc/stat 2>/dev/null; echo __PORTMATE_NET2__; head -n 258 /proc/net/dev 2>/dev/null; echo __PORTMATE_ADDRS__; { ip -o addr show 2>/dev/null | head -n 64; ip addr show 2>/dev/null | head -n 64; ifconfig -a 2>/dev/null | head -n 64; busybox ip -o addr show 2>/dev/null | head -n 64; busybox ip addr show 2>/dev/null | head -n 64; busybox ifconfig -a 2>/dev/null | head -n 64; } | head -n 384; echo __PORTMATE_KERNEL_IPV6__; head -n 128 /proc/net/if_inet6 2>/dev/null; echo __PORTMATE_KERNEL_IPV4__; head -n 384 /proc/net/fib_trie 2>/dev/null; echo __PORTMATE_KERNEL_ROUTE__; head -n 64 /proc/net/route 2>/dev/null; echo __PORTMATE_HOSTNAME_ADDRS__; { hostname -I 2>/dev/null; hostname -i 2>/dev/null; busybox hostname -i 2>/dev/null; } | head -n 32; echo __PORTMATE_LOADAVG__; head -n 1 /proc/loadavg 2>/dev/null; echo __PORTMATE_PROCESSES__; ps -eo pid=,pcpu=,pmem=,rss=,comm= --sort=-pcpu,-rss 2>/dev/null | head -n 8; echo __PORTMATE_DISKS__; (df -Pk -x tmpfs -x devtmpfs 2>/dev/null || df -Pk 2>/dev/null) | head -n 17'"#;
pub(super) const REMOTE_MACOS_SYSMON_COMMAND: &str = r#"sh -c 'PATH=/usr/bin:/bin:/usr/sbin:/sbin:$PATH; export PATH LC_ALL=C; echo __PORTMATE_BOOT__; sysctl -n kern.boottime 2>/dev/null | head -n 1; echo __PORTMATE_CPU__; top -l 2 -s 1 -F -n 0 2>/dev/null | grep "CPU usage" | tail -n 1; echo __PORTMATE_MEMORY__; sysctl -n hw.memsize 2>/dev/null | head -n 1; vm_stat 2>/dev/null | head -n 32; echo __PORTMATE_NET1__; netstat -ibn 2>/dev/null | head -n 258; sleep 0.2; echo __PORTMATE_NET2__; netstat -ibn 2>/dev/null | head -n 258; echo __PORTMATE_LOADAVG__; sysctl -n vm.loadavg 2>/dev/null | head -n 1; echo __PORTMATE_PROCESSES__; ps -Arcwwwxo pid=,pcpu=,pmem=,rss=,comm= 2>/dev/null | head -n 8; echo __PORTMATE_DISKS__; df -Pk 2>/dev/null | head -n 17'"#;
pub(super) const REMOTE_FREEBSD_SYSMON_COMMAND: &str = r#"sh -c 'PATH=/usr/bin:/bin:/usr/sbin:/sbin:$PATH; export PATH LC_ALL=C; echo __PORTMATE_BOOT__; sysctl -n kern.boottime 2>/dev/null | head -n 1; echo __PORTMATE_STAT1__; sysctl -n kern.cp_time 2>/dev/null | head -n 1; echo __PORTMATE_NET1__; netstat -ibn 2>/dev/null | head -n 258; sleep 0.2; echo __PORTMATE_STAT2__; sysctl -n kern.cp_time 2>/dev/null | head -n 1; echo __PORTMATE_NET2__; netstat -ibn 2>/dev/null | head -n 258; echo __PORTMATE_MEMORY__; printf "total %s\n" "$(sysctl -n hw.physmem 2>/dev/null | head -n 1)"; printf "page_size %s\n" "$(sysctl -n hw.pagesize 2>/dev/null | head -n 1)"; printf "free %s\n" "$(sysctl -n vm.stats.vm.v_free_count 2>/dev/null | head -n 1)"; printf "inactive %s\n" "$(sysctl -n vm.stats.vm.v_inactive_count 2>/dev/null | head -n 1)"; printf "cache %s\n" "$(sysctl -n vm.stats.vm.v_cache_count 2>/dev/null | head -n 1)"; echo __PORTMATE_LOADAVG__; sysctl -n vm.loadavg 2>/dev/null | head -n 1; echo __PORTMATE_PROCESSES__; ps -axr -o pid=,pcpu=,pmem=,rss=,comm= 2>/dev/null | head -n 8; echo __PORTMATE_DISKS__; df -Pk 2>/dev/null | head -n 17'"#;

pub(super) const REMOTE_WINDOWS_SYSMON_JSON_MARKER: &str = "__PORTMATE_WINDOWS_SYSMON_JSON__";
pub(super) const REMOTE_WINDOWS_PLATFORM_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
if ($env:OS -eq 'Windows_NT') {
    [Console]::Out.WriteLine('Windows')
} else {
    [Console]::Out.WriteLine([Environment]::OSVersion.Platform)
}
"#;
pub(super) const REMOTE_WINDOWS_SYSMON_SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$os = Get-CimInstance -ClassName Win32_OperatingSystem
$computer = Get-CimInstance -ClassName Win32_ComputerSystem -ErrorAction SilentlyContinue
$logicalProcessors = [Math]::Max(1, [int]$computer.NumberOfLogicalProcessors)
$memoryTotal = [uint64]([uint64]$os.TotalVisibleMemorySize * 1024)
$memoryAvailable = [uint64]([uint64]$os.FreePhysicalMemory * 1024)
$uptime = [uint64][Math]::Max(0, ((Get-Date) - $os.LastBootUpTime).TotalSeconds)

$cpuSample = Get-CimInstance -ClassName Win32_PerfFormattedData_PerfOS_Processor -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -eq '_Total' } |
    Select-Object -First 1
$cpuPercent = if ($null -eq $cpuSample) {
    0.0
} else {
    [Math]::Min(100.0, [Math]::Max(0.0, [double]$cpuSample.PercentProcessorTime))
}

$processes = @(Get-CimInstance -ClassName Win32_PerfFormattedData_PerfProc_Process -ErrorAction SilentlyContinue |
    Where-Object { [uint32]$_.IDProcess -gt 0 -and $_.Name -ne '_Total' } |
    Sort-Object -Property @{Expression = {[uint64]$_.PercentProcessorTime}; Descending = $true}, @{Expression = {[uint64]$_.WorkingSet}; Descending = $true} |
    Select-Object -First 8 |
    ForEach-Object {
        [ordered]@{
            pid = [uint32]$_.IDProcess
            name = [string]$_.Name
            cpuPercent = [Math]::Min(100.0, [Math]::Max(0.0, [double]$_.PercentProcessorTime / $logicalProcessors))
            rssBytes = [uint64]$_.WorkingSet
        }
    })

$disks = @(Get-CimInstance -ClassName Win32_LogicalDisk -Filter 'DriveType=3' -ErrorAction SilentlyContinue |
    Sort-Object -Property DeviceID |
    Select-Object -First 16 |
    ForEach-Object {
        $filesystem = if ([string]::IsNullOrWhiteSpace([string]$_.FileSystem)) { [string]$_.DeviceID } else { [string]$_.FileSystem }
        [ordered]@{
            filesystem = $filesystem
            mountPoint = [string]$_.DeviceID
            totalBytes = [uint64]$_.Size
            availableBytes = [uint64]$_.FreeSpace
        }
    })

$rawNetworks = @{}
@(Get-CimInstance -ClassName Win32_PerfRawData_Tcpip_NetworkInterface -ErrorAction SilentlyContinue) |
    ForEach-Object { $rawNetworks[([string]$_.Name).Trim()] = $_ }
$ipAddresses = @{}
@(Get-CimInstance -ClassName Win32_NetworkAdapterConfiguration -Filter 'IPEnabled=True' -ErrorAction SilentlyContinue) |
    ForEach-Object {
        $addresses = @($_.IPAddress | Where-Object { -not [string]::IsNullOrWhiteSpace([string]$_) })
        $description = ([string]$_.Description).Trim()
        if ($addresses.Count -gt 0 -and -not [string]::IsNullOrWhiteSpace($description)) {
            $ipAddresses[$description] = $addresses
        }
    }
$matchedIpAddressNames = @{}
$performanceNetworkInterfaces = @(Get-CimInstance -ClassName Win32_PerfFormattedData_Tcpip_NetworkInterface -ErrorAction SilentlyContinue |
    ForEach-Object {
        $name = ([string]$_.Name).Trim()
        $raw = $rawNetworks[$name]
        $addresses = if ($ipAddresses.ContainsKey($name)) {
            $matchedIpAddressNames[$name] = $true
            @($ipAddresses[$name])
        } else {
            @()
        }
        [ordered]@{
            name = $name
            addresses = $addresses
            rxBytes = if ($null -eq $raw) { [uint64]0 } else { [uint64]$raw.BytesReceivedPersec }
            txBytes = if ($null -eq $raw) { [uint64]0 } else { [uint64]$raw.BytesSentPersec }
            rxKbps = [Math]::Max(0.0, [double]$_.BytesReceivedPersec / 1024.0)
            txKbps = [Math]::Max(0.0, [double]$_.BytesSentPersec / 1024.0)
        }
    })
$addressOnlyNetworkInterfaces = @($ipAddresses.GetEnumerator() |
    Where-Object { -not $matchedIpAddressNames.ContainsKey([string]$_.Key) } |
    ForEach-Object {
        [ordered]@{
            name = [string]$_.Key
            addresses = @($_.Value)
            rxBytes = [uint64]0
            txBytes = [uint64]0
            rxKbps = 0.0
            txKbps = 0.0
        }
    })
$networkInterfaces = @(@($performanceNetworkInterfaces + $addressOnlyNetworkInterfaces) |
    Sort-Object -Property @(
        @{Expression = { if (@($_.addresses).Count -gt 0) { 0 } else { 1 } }; Ascending = $true},
        @{Expression = {[double]$_.rxKbps + [double]$_.txKbps}; Descending = $true},
        @{Expression = {[string]$_.name}; Ascending = $true}
    ) |
    Select-Object -First 128)

$payload = [ordered]@{
    uptimeSeconds = $uptime
    cpuPercent = $cpuPercent
    memoryTotalBytes = $memoryTotal
    memoryAvailableBytes = $memoryAvailable
    processes = $processes
    disks = $disks
    networkInterfaces = $networkInterfaces
}
[Console]::Out.WriteLine('__PORTMATE_WINDOWS_SYSMON_JSON__')
[Console]::Out.WriteLine(($payload | ConvertTo-Json -Depth 4 -Compress))
"#;

pub(super) async fn refresh_sysmon_inner(
    state: &AppState,
    session_id: &str,
) -> Result<SysmonSnapshot, String> {
    let _permit = Arc::clone(&state.sysmon_slots)
        .try_acquire_owned()
        .map_err(|_| format!("Sysmon refresh limit reached ({MAX_CONCURRENT_SYSMON_REFRESHES})"))?;
    let profile = {
        let store = state.store.lock().map_err(|error| error.to_string())?;
        store
            .profile(session_id)
            .ok_or_else(|| format!("unknown session: {session_id}"))?
    };

    let snapshot = if matches!(
        profile.connection,
        ConnectionConfig::Ssh(_) | ConnectionConfig::Tmux(_)
    ) {
        let auxiliary = ssh_auxiliary_lease(state, session_id)?;
        collect_remote_sysmon(session_id, auxiliary.handle()).await?
    } else {
        collect_local_sysmon(session_id).await?
    };

    commit_sysmon_snapshot(state, session_id, snapshot)
}

pub(super) fn commit_sysmon_snapshot(
    state: &AppState,
    session_id: &str,
    snapshot: SysmonSnapshot,
) -> Result<SysmonSnapshot, String> {
    if snapshot.session_id != session_id {
        return Err("Sysmon snapshot session does not match the requested session".to_string());
    }
    let mut store = state.store.lock().map_err(|error| error.to_string())?;
    commit_tracked_store_mutation(&mut store, &state.store_path, |next_store| {
        if next_store.profile(session_id).is_none() {
            return Err(format!("unknown session: {session_id}"));
        }
        next_store.record_sysmon_snapshot(snapshot.clone());
        let event_ids = next_store
            .record_system_event_tracked(session_id, "PortMate: sysmon snapshot refreshed")
            .into_iter()
            .collect();
        Ok((snapshot, event_ids))
    })
}

pub(super) fn validate_sysmon_history_query_limit(limit: Option<usize>) -> Result<usize, String> {
    let limit = limit.unwrap_or(DEFAULT_SYSMON_HISTORY_QUERY_LIMIT);
    if !(1..=MAX_SYSMON_HISTORY_QUERY_LIMIT).contains(&limit) {
        return Err(format!(
            "Sysmon history limit must be between 1 and {MAX_SYSMON_HISTORY_QUERY_LIMIT}"
        ));
    }
    Ok(limit)
}

pub(super) async fn collect_local_sysmon(session_id: &str) -> Result<SysmonSnapshot, String> {
    match std::env::consts::OS {
        "linux" => {
            let session_id = session_id.to_string();
            let sample = tauri::async_runtime::spawn_blocking(move || {
                collect_local_linux_sysmon(&session_id)
            });
            let mut snapshot = sample
                .await
                .map_err(|error| format!("本机 Linux Sysmon 任务失败: {error}"))?;
            let (processes, disks) = tokio::join!(
                exec_local_sysmon_command(
                    "ps",
                    &["-eo", "pid=,pcpu=,pmem=,rss=,comm=", "--sort=-pcpu,-rss"],
                    LOCAL_SYSMON_COMMAND_TIMEOUT,
                ),
                exec_local_sysmon_command("df", &["-Pk"], LOCAL_SYSMON_COMMAND_TIMEOUT),
            );
            let processes = processes?;
            let disks = disks?;
            snapshot.processes = parse_sysmon_processes(&processes);
            snapshot.disks = parse_sysmon_disks(&disks);
            Ok(snapshot)
        }
        "macos" => {
            let output = exec_local_sysmon_command(
                "sh",
                &["-c", REMOTE_MACOS_SYSMON_COMMAND],
                LOCAL_SYSMON_COMMAND_TIMEOUT,
            )
            .await?;
            parse_remote_macos_sysmon_output(session_id, &output)
                .map_err(|error| error.replacen("远端", "本机", 1))
        }
        "windows" => {
            let encoded = windows_powershell_encoded_script(REMOTE_WINDOWS_SYSMON_SCRIPT);
            let output = exec_local_sysmon_command(
                "powershell.exe",
                &[
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-EncodedCommand",
                    &encoded,
                ],
                LOCAL_SYSMON_COMMAND_TIMEOUT,
            )
            .await?;
            parse_remote_windows_sysmon_output(session_id, &output)
                .map_err(|error| error.replacen("远端", "本机", 1))
        }
        platform => Err(format!(
            "本机 Sysmon 暂不支持 {}",
            bounded_sysmon_label(platform, 64)
        )),
    }
}

pub(super) fn collect_local_linux_sysmon(session_id: &str) -> SysmonSnapshot {
    let uptime_seconds = read_uptime_seconds().unwrap_or_default();
    let (memory_total_bytes, memory_available_bytes, memory_percent) =
        read_memory_usage().unwrap_or_default();
    let load_average = read_load_average().unwrap_or_default();
    let cpu_a = read_cpu_times();
    let net_a = read_network_interfaces();
    std::thread::sleep(Duration::from_millis(120));
    let cpu_b = read_cpu_times();
    let net_b = read_network_interfaces();
    let cpu_percent = cpu_percent_between(cpu_a, cpu_b);
    let network_interfaces = network_interface_rates(
        net_a.unwrap_or_default(),
        net_b.unwrap_or_default(),
        read_network_addresses(),
        LOCAL_SYSMON_SAMPLE_SECONDS,
    );
    let (rx_kbps, tx_kbps) = aggregate_network_rates(&network_interfaces);
    SysmonSnapshot {
        session_id: session_id.to_string(),
        ts: Utc::now(),
        uptime_seconds,
        cpu_percent,
        memory_percent,
        rx_kbps,
        tx_kbps,
        load_average,
        memory_total_bytes,
        memory_available_bytes,
        processes: Vec::new(),
        disks: Vec::new(),
        network_interfaces,
    }
}

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

pub(super) async fn collect_remote_sysmon(
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

pub(super) async fn detect_remote_sysmon_platform(
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

pub(super) fn section_between<'a>(value: &'a str, start: &str, end: &str) -> &'a str {
    value
        .split(start)
        .nth(1)
        .and_then(|tail| tail.split(end).next())
        .unwrap_or_default()
}

pub(super) fn cpu_percent_between(before: Option<(u64, u64)>, after: Option<(u64, u64)>) -> f32 {
    match (before, after) {
        (Some((idle_a, total_a)), Some((idle_b, total_b))) if total_b > total_a => {
            let idle_delta = idle_b.saturating_sub(idle_a) as f32;
            let total_delta = total_b.saturating_sub(total_a) as f32;
            if total_delta > 0.0 {
                ((1.0 - idle_delta / total_delta) * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            }
        }
        _ => 0.0,
    }
}

#[cfg(target_os = "linux")]
pub(super) fn read_uptime_seconds() -> Option<u64> {
    let raw = fs::read_to_string("/proc/uptime").ok()?;
    parse_uptime_seconds(&raw)
}

#[cfg(not(target_os = "linux"))]
pub(super) fn read_uptime_seconds() -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
pub(super) fn read_memory_usage() -> Option<(u64, u64, f32)> {
    let raw = fs::read_to_string("/proc/meminfo").ok()?;
    parse_memory_usage(&raw)
}

pub(super) fn parse_memory_usage(raw: &str) -> Option<(u64, u64, f32)> {
    let mut total = None;
    let mut available = None;
    for line in raw.lines() {
        let mut parts = line.split_whitespace();
        let Some(key) = parts.next() else {
            continue;
        };
        match key {
            "MemTotal:" => total = parts.next().and_then(|value| value.parse::<u64>().ok()),
            "MemAvailable:" => available = parts.next().and_then(|value| value.parse::<u64>().ok()),
            _ => {}
        }
    }
    let total = total?;
    let available = available?.min(total);
    if total == 0 {
        return None;
    }
    let percent = ((total - available) as f32 / total as f32 * 100.0).clamp(0.0, 100.0);
    Some((
        total.saturating_mul(1024),
        available.saturating_mul(1024),
        percent,
    ))
}

#[cfg(not(target_os = "linux"))]
pub(super) fn read_memory_usage() -> Option<(u64, u64, f32)> {
    None
}

#[cfg(target_os = "linux")]
pub(super) fn read_load_average() -> Option<[f32; 3]> {
    let raw = fs::read_to_string("/proc/loadavg").ok()?;
    parse_load_average(&raw)
}

#[cfg(not(target_os = "linux"))]
pub(super) fn read_load_average() -> Option<[f32; 3]> {
    None
}

pub(super) fn parse_load_average(raw: &str) -> Option<[f32; 3]> {
    let mut values = raw
        .split_whitespace()
        .take(3)
        .map(|value| value.parse::<f32>().ok().filter(|value| value.is_finite()));
    Some([values.next()??, values.next()??, values.next()??])
}

pub(super) fn parse_bsd_load_average(raw: &str) -> Option<[f32; 3]> {
    let values = raw
        .split_whitespace()
        .filter_map(|value| {
            value
                .trim_matches(|character: char| character == '{' || character == '}')
                .parse::<f32>()
                .ok()
                .filter(|value| value.is_finite())
        })
        .take(3)
        .collect::<Vec<_>>();
    (values.len() == 3).then_some([values[0], values[1], values[2]])
}

pub(super) fn parse_bsd_boot_time_seconds(raw: &str) -> Option<u64> {
    let seconds = raw.split("sec =").nth(1)?.trim_start();
    seconds
        .split(|character: char| !character.is_ascii_digit())
        .next()?
        .parse::<u64>()
        .ok()
}

pub(super) fn parse_macos_cpu_percent(raw: &str) -> Option<f32> {
    let line = raw.lines().find(|line| line.contains("CPU usage:"))?;
    let idle = line
        .split(',')
        .find(|field| field.contains(" idle"))?
        .split('%')
        .next()?
        .split_whitespace()
        .last()
        .and_then(parse_nonnegative_f32)?
        .clamp(0.0, 100.0);
    Some(100.0 - idle)
}

pub(super) fn parse_macos_memory_usage(raw: &str) -> Option<(u64, u64, f32)> {
    let total = raw
        .lines()
        .find_map(|line| line.trim().parse::<u64>().ok())?;
    if total == 0 {
        return None;
    }
    let page_size = raw
        .lines()
        .find(|line| line.contains("page size of"))
        .and_then(|line| line.split("page size of").nth(1))
        .and_then(first_ascii_u64)?;
    let mut available_pages = 0_u64;
    for line in raw.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if matches!(
            name.trim(),
            "Pages free" | "Pages inactive" | "Pages speculative"
        ) {
            available_pages =
                available_pages.saturating_add(first_ascii_u64(value).unwrap_or_default());
        }
    }
    let available = available_pages.saturating_mul(page_size).min(total);
    let percent = ((total - available) as f32 / total as f32 * 100.0).clamp(0.0, 100.0);
    Some((total, available, percent))
}

pub(super) fn parse_freebsd_cpu_times(raw: &str) -> Option<(u64, u64)> {
    let values = raw
        .split_whitespace()
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if values.len() != 5 {
        return None;
    }
    let idle = values[4];
    let total = values
        .into_iter()
        .fold(0_u64, |total, value| total.saturating_add(value));
    Some((idle, total))
}

pub(super) fn parse_freebsd_memory_usage(raw: &str) -> Option<(u64, u64, f32)> {
    let mut total = None;
    let mut page_size = None;
    let mut available_pages = 0_u64;
    for line in raw.lines() {
        let mut fields = line.split_whitespace();
        let Some(name) = fields.next() else {
            continue;
        };
        let value = fields.next().and_then(|value| value.parse::<u64>().ok());
        match name {
            "total" => total = value,
            "page_size" => page_size = value,
            "free" | "inactive" | "cache" => {
                available_pages = available_pages.saturating_add(value.unwrap_or_default());
            }
            _ => {}
        }
    }
    let total = total?;
    let page_size = page_size?;
    if total == 0 || page_size == 0 {
        return None;
    }
    let available = available_pages.saturating_mul(page_size).min(total);
    let percent = ((total - available) as f32 / total as f32 * 100.0).clamp(0.0, 100.0);
    Some((total, available, percent))
}

pub(super) fn first_ascii_u64(value: &str) -> Option<u64> {
    value
        .split(|character: char| !character.is_ascii_digit())
        .find(|part| !part.is_empty())?
        .parse::<u64>()
        .ok()
}

#[cfg(target_os = "linux")]
pub(super) fn read_cpu_times() -> Option<(u64, u64)> {
    let raw = fs::read_to_string("/proc/stat").ok()?;
    parse_cpu_times(&raw)
}

pub(super) fn parse_cpu_times(raw: &str) -> Option<(u64, u64)> {
    let line = raw.lines().find(|line| line.starts_with("cpu "))?;
    let values = line
        .split_whitespace()
        .skip(1)
        .filter_map(|value| value.parse::<u64>().ok())
        .collect::<Vec<_>>();
    if values.len() < 4 {
        return None;
    }
    let idle =
        values.get(3).copied().unwrap_or_default() + values.get(4).copied().unwrap_or_default();
    let total = values
        .iter()
        .fold(0_u64, |total, value| total.saturating_add(*value));
    Some((idle, total))
}

#[cfg(not(target_os = "linux"))]
pub(super) fn read_cpu_times() -> Option<(u64, u64)> {
    None
}

pub(super) fn parse_sysmon_processes(raw: &str) -> Vec<SysmonProcess> {
    let mut processes = raw
        .lines()
        .filter_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() < 5 {
                return None;
            }
            let pid = fields[0].parse::<u32>().ok()?;
            let cpu_percent = parse_nonnegative_f32(fields[1])?;
            let memory_percent = parse_nonnegative_f32(fields[2])?;
            let rss_bytes = fields[3].parse::<u64>().ok()?.saturating_mul(1024);
            let name = bounded_sysmon_label(&fields[4..].join(" "), 128);
            if name.is_empty() {
                return None;
            }
            Some(SysmonProcess {
                pid,
                name,
                cpu_percent,
                memory_percent,
                rss_bytes,
            })
        })
        .collect::<Vec<_>>();
    processes.sort_by(|left, right| {
        right
            .cpu_percent
            .total_cmp(&left.cpu_percent)
            .then_with(|| right.rss_bytes.cmp(&left.rss_bytes))
            .then_with(|| left.pid.cmp(&right.pid))
    });
    processes.truncate(MAX_SYSMON_PROCESSES);
    processes
}

pub(super) fn parse_sysmon_disks(raw: &str) -> Vec<SysmonDisk> {
    let mut mount_points = HashSet::new();
    let mut disks = raw
        .lines()
        .filter_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() < 6 || fields[0].eq_ignore_ascii_case("filesystem") {
                return None;
            }
            let total_blocks = fields[1].parse::<u64>().ok()?;
            let available_blocks = fields[3].parse::<u64>().ok()?.min(total_blocks);
            if total_blocks == 0 {
                return None;
            }
            let mount_point =
                bounded_sysmon_label(&fields[5..].join(" ").replace("\\040", " "), 256);
            if mount_point.is_empty() || !mount_points.insert(mount_point.clone()) {
                return None;
            }
            let computed_percent =
                (total_blocks - available_blocks) as f32 / total_blocks as f32 * 100.0;
            let used_percent = fields[4]
                .trim_end_matches('%')
                .parse::<f32>()
                .ok()
                .filter(|value| value.is_finite())
                .unwrap_or(computed_percent)
                .clamp(0.0, 100.0);
            Some(SysmonDisk {
                filesystem: bounded_sysmon_label(fields[0], 256),
                mount_point,
                total_bytes: total_blocks.saturating_mul(1024),
                available_bytes: available_blocks.saturating_mul(1024),
                used_percent,
            })
        })
        .collect::<Vec<_>>();
    disks.sort_by(|left, right| {
        (left.mount_point != "/")
            .cmp(&(right.mount_point != "/"))
            .then_with(|| left.mount_point.cmp(&right.mount_point))
    });
    disks.truncate(MAX_SYSMON_DISKS);
    disks
}

pub(super) fn parse_nonnegative_f32(value: &str) -> Option<f32> {
    value
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)
}

pub(super) fn bounded_sysmon_percent(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.0, 100.0)
    } else {
        0.0
    }
}

pub(super) fn bounded_sysmon_rate(value: f32) -> f32 {
    const MAX_RATE_KIB_PER_SECOND: f32 = u64::MAX as f32 / 1024.0;
    if value.is_finite() {
        value.clamp(0.0, MAX_RATE_KIB_PER_SECOND)
    } else {
        0.0
    }
}

pub(super) fn bounded_sysmon_label(value: &str, max_chars: usize) -> String {
    value
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .take(max_chars)
        .collect()
}

pub(super) fn parse_uptime_seconds(raw: &str) -> Option<u64> {
    raw.split_whitespace()
        .next()?
        .parse::<f64>()
        .ok()
        .map(|value| value as u64)
}
