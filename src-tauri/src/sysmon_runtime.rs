use super::*;

pub(super) const DEFAULT_SYSMON_HISTORY_QUERY_LIMIT: usize = 120;

pub(super) const MAX_SYSMON_HISTORY_QUERY_LIMIT: usize = 240;
pub(super) const LOCAL_SYSMON_SAMPLE_SECONDS: f32 = 0.12;
pub(super) const MAX_CONCURRENT_SYSMON_REFRESHES: usize = 4;
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
