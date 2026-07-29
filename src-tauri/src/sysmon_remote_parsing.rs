use super::*;

pub(super) fn remote_sysmon_platform_label(output: &str) -> Option<String> {
    let platform = output.lines().find(|line| !line.trim().is_empty())?.trim();
    if !platform.chars().all(|character| {
        character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '/')
    }) {
        return None;
    }
    let platform = bounded_sysmon_label(platform, 64);
    (!platform.is_empty()).then_some(platform)
}

pub(super) fn parse_remote_sysmon_output(
    session_id: &str,
    output: &str,
) -> Result<SysmonSnapshot, String> {
    let uptime_raw = output
        .split("__PORTMATE_MEMINFO__")
        .next()
        .unwrap_or_default();
    let meminfo = section_between(output, "__PORTMATE_MEMINFO__", "__PORTMATE_STAT1__");
    let stat1 = section_between(output, "__PORTMATE_STAT1__", "__PORTMATE_NET1__");
    let net1 = section_between(output, "__PORTMATE_NET1__", "__PORTMATE_STAT2__");
    let stat2 = section_between(output, "__PORTMATE_STAT2__", "__PORTMATE_NET2__");
    let net2 = section_between(output, "__PORTMATE_NET2__", "__PORTMATE_ADDRS__");
    let address_output = section_between(output, "__PORTMATE_ADDRS__", "__PORTMATE_LOADAVG__");
    let loadavg = section_between(output, "__PORTMATE_LOADAVG__", "__PORTMATE_PROCESSES__");
    let processes = section_between(output, "__PORTMATE_PROCESSES__", "__PORTMATE_DISKS__");
    let disks = output
        .split("__PORTMATE_DISKS__")
        .nth(1)
        .unwrap_or_default();

    let uptime_seconds = parse_uptime_seconds(uptime_raw).unwrap_or_default();
    let (memory_total_bytes, memory_available_bytes, memory_percent) =
        parse_memory_usage(meminfo).unwrap_or_default();
    let cpu_percent = cpu_percent_between(parse_cpu_times(stat1), parse_cpu_times(stat2));
    let network_interfaces = network_interface_rates(
        parse_network_interfaces(net1),
        parse_network_interfaces(net2),
        parse_linux_network_addresses(address_output),
        REMOTE_SYSMON_SAMPLE_SECONDS,
    );
    let (rx_kbps, tx_kbps) = aggregate_network_rates(&network_interfaces);

    if uptime_seconds == 0 && memory_percent == 0.0 && cpu_percent == 0.0 {
        return Err("远端未提供 Linux /proc Sysmon 数据".to_string());
    }

    Ok(SysmonSnapshot {
        session_id: session_id.to_string(),
        ts: Utc::now(),
        uptime_seconds,
        cpu_percent,
        memory_percent,
        rx_kbps,
        tx_kbps,
        load_average: parse_load_average(loadavg).unwrap_or_default(),
        memory_total_bytes,
        memory_available_bytes,
        processes: parse_sysmon_processes(processes),
        disks: parse_sysmon_disks(disks),
        network_interfaces,
    })
}

pub(super) fn parse_remote_macos_sysmon_output(
    session_id: &str,
    output: &str,
) -> Result<SysmonSnapshot, String> {
    parse_remote_macos_sysmon_output_at(session_id, output, Utc::now())
}

pub(super) fn parse_remote_macos_sysmon_output_at(
    session_id: &str,
    output: &str,
    sampled_at: DateTime<Utc>,
) -> Result<SysmonSnapshot, String> {
    let boot = section_between(output, "__PORTMATE_BOOT__", "__PORTMATE_CPU__");
    let cpu = section_between(output, "__PORTMATE_CPU__", "__PORTMATE_MEMORY__");
    let memory = section_between(output, "__PORTMATE_MEMORY__", "__PORTMATE_NET1__");
    let net1 = section_between(output, "__PORTMATE_NET1__", "__PORTMATE_NET2__");
    let net2 = section_between(output, "__PORTMATE_NET2__", "__PORTMATE_LOADAVG__");
    let loadavg = section_between(output, "__PORTMATE_LOADAVG__", "__PORTMATE_PROCESSES__");
    let processes = section_between(output, "__PORTMATE_PROCESSES__", "__PORTMATE_DISKS__");
    let disks = output
        .split("__PORTMATE_DISKS__")
        .nth(1)
        .unwrap_or_default();

    let sampled_epoch = u64::try_from(sampled_at.timestamp()).unwrap_or_default();
    let uptime_seconds = parse_bsd_boot_time_seconds(boot)
        .map(|boot_epoch| sampled_epoch.saturating_sub(boot_epoch))
        .unwrap_or_default();
    let cpu_percent = parse_macos_cpu_percent(cpu).unwrap_or_default();
    let (memory_total_bytes, memory_available_bytes, memory_percent) =
        parse_macos_memory_usage(memory).unwrap_or_default();
    let network_interfaces = network_interface_rates(
        parse_bsd_network_interfaces(net1),
        parse_bsd_network_interfaces(net2),
        parse_bsd_network_addresses(net2),
        REMOTE_SYSMON_SAMPLE_SECONDS,
    );
    let (rx_kbps, tx_kbps) = aggregate_network_rates(&network_interfaces);

    if uptime_seconds == 0 && memory_total_bytes == 0 && cpu_percent == 0.0 {
        return Err("远端未提供 macOS Sysmon 数据".to_string());
    }

    Ok(SysmonSnapshot {
        session_id: session_id.to_string(),
        ts: sampled_at,
        uptime_seconds,
        cpu_percent,
        memory_percent,
        rx_kbps,
        tx_kbps,
        load_average: parse_bsd_load_average(loadavg).unwrap_or_default(),
        memory_total_bytes,
        memory_available_bytes,
        processes: parse_sysmon_processes(processes),
        disks: parse_sysmon_disks(disks),
        network_interfaces,
    })
}

pub(super) fn parse_remote_freebsd_sysmon_output(
    session_id: &str,
    output: &str,
) -> Result<SysmonSnapshot, String> {
    parse_remote_freebsd_sysmon_output_at(session_id, output, Utc::now())
}

pub(super) fn parse_remote_freebsd_sysmon_output_at(
    session_id: &str,
    output: &str,
    sampled_at: DateTime<Utc>,
) -> Result<SysmonSnapshot, String> {
    let boot = section_between(output, "__PORTMATE_BOOT__", "__PORTMATE_STAT1__");
    let stat1 = section_between(output, "__PORTMATE_STAT1__", "__PORTMATE_NET1__");
    let net1 = section_between(output, "__PORTMATE_NET1__", "__PORTMATE_STAT2__");
    let stat2 = section_between(output, "__PORTMATE_STAT2__", "__PORTMATE_NET2__");
    let net2 = section_between(output, "__PORTMATE_NET2__", "__PORTMATE_MEMORY__");
    let memory = section_between(output, "__PORTMATE_MEMORY__", "__PORTMATE_LOADAVG__");
    let loadavg = section_between(output, "__PORTMATE_LOADAVG__", "__PORTMATE_PROCESSES__");
    let processes = section_between(output, "__PORTMATE_PROCESSES__", "__PORTMATE_DISKS__");
    let disks = output
        .split("__PORTMATE_DISKS__")
        .nth(1)
        .unwrap_or_default();

    let sampled_epoch = u64::try_from(sampled_at.timestamp()).unwrap_or_default();
    let uptime_seconds = parse_bsd_boot_time_seconds(boot)
        .map(|boot_epoch| sampled_epoch.saturating_sub(boot_epoch))
        .unwrap_or_default();
    let cpu_percent = cpu_percent_between(
        parse_freebsd_cpu_times(stat1),
        parse_freebsd_cpu_times(stat2),
    );
    let (memory_total_bytes, memory_available_bytes, memory_percent) =
        parse_freebsd_memory_usage(memory).unwrap_or_default();
    let network_interfaces = network_interface_rates(
        parse_bsd_network_interfaces(net1),
        parse_bsd_network_interfaces(net2),
        parse_bsd_network_addresses(net2),
        REMOTE_SYSMON_SAMPLE_SECONDS,
    );
    let (rx_kbps, tx_kbps) = aggregate_network_rates(&network_interfaces);

    if uptime_seconds == 0 && memory_total_bytes == 0 && cpu_percent == 0.0 {
        return Err("远端未提供 FreeBSD Sysmon 数据".to_string());
    }

    Ok(SysmonSnapshot {
        session_id: session_id.to_string(),
        ts: sampled_at,
        uptime_seconds,
        cpu_percent,
        memory_percent,
        rx_kbps,
        tx_kbps,
        load_average: parse_bsd_load_average(loadavg).unwrap_or_default(),
        memory_total_bytes,
        memory_available_bytes,
        processes: parse_sysmon_processes(processes),
        disks: parse_sysmon_disks(disks),
        network_interfaces,
    })
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteWindowsSysmonSample {
    uptime_seconds: u64,
    cpu_percent: f32,
    memory_total_bytes: u64,
    memory_available_bytes: u64,
    #[serde(default)]
    processes: Vec<RemoteWindowsSysmonProcess>,
    #[serde(default)]
    disks: Vec<RemoteWindowsSysmonDisk>,
    #[serde(default)]
    network_interfaces: Vec<RemoteWindowsSysmonNetworkInterface>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteWindowsSysmonProcess {
    pid: u32,
    name: String,
    cpu_percent: f32,
    rss_bytes: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteWindowsSysmonDisk {
    filesystem: String,
    mount_point: String,
    total_bytes: u64,
    available_bytes: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemoteWindowsSysmonNetworkInterface {
    name: String,
    #[serde(default)]
    addresses: Vec<String>,
    rx_bytes: u64,
    tx_bytes: u64,
    rx_kbps: f32,
    tx_kbps: f32,
}

pub(super) fn parse_remote_windows_sysmon_output(
    session_id: &str,
    output: &str,
) -> Result<SysmonSnapshot, String> {
    parse_remote_windows_sysmon_output_at(session_id, output, Utc::now())
}

pub(super) fn parse_remote_windows_sysmon_output_at(
    session_id: &str,
    output: &str,
    sampled_at: DateTime<Utc>,
) -> Result<SysmonSnapshot, String> {
    let payload = output
        .split_once(REMOTE_WINDOWS_SYSMON_JSON_MARKER)
        .map(|(_, payload)| payload.trim())
        .filter(|payload| !payload.is_empty())
        .ok_or_else(|| "远端未提供 Windows Sysmon JSON 标记".to_string())?;
    let sample = serde_json::from_str::<RemoteWindowsSysmonSample>(payload)
        .map_err(|error| format!("远端 Windows Sysmon JSON 无效: {error}"))?;

    let cpu_percent = bounded_sysmon_percent(sample.cpu_percent);
    let memory_total_bytes = sample.memory_total_bytes;
    let memory_available_bytes = sample.memory_available_bytes.min(memory_total_bytes);
    let memory_percent = if memory_total_bytes == 0 {
        0.0
    } else {
        (memory_total_bytes - memory_available_bytes) as f32 / memory_total_bytes as f32 * 100.0
    };

    let mut processes = sample
        .processes
        .into_iter()
        .filter_map(|process| {
            let name = bounded_sysmon_label(&process.name, 128);
            if process.pid == 0 || name.is_empty() {
                return None;
            }
            Some(SysmonProcess {
                pid: process.pid,
                name,
                cpu_percent: bounded_sysmon_percent(process.cpu_percent),
                memory_percent: if memory_total_bytes == 0 {
                    0.0
                } else {
                    (process.rss_bytes as f32 / memory_total_bytes as f32 * 100.0).clamp(0.0, 100.0)
                },
                rss_bytes: process.rss_bytes,
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

    let mut mount_points = HashSet::new();
    let mut disks = sample
        .disks
        .into_iter()
        .filter_map(|disk| {
            let filesystem = bounded_sysmon_label(&disk.filesystem, 256);
            let mount_point = bounded_sysmon_label(&disk.mount_point, 256);
            let mount_key = mount_point.to_ascii_lowercase();
            if filesystem.is_empty()
                || mount_point.is_empty()
                || disk.total_bytes == 0
                || !mount_points.insert(mount_key)
            {
                return None;
            }
            let available_bytes = disk.available_bytes.min(disk.total_bytes);
            Some(SysmonDisk {
                filesystem,
                mount_point,
                total_bytes: disk.total_bytes,
                available_bytes,
                used_percent: (disk.total_bytes - available_bytes) as f32 / disk.total_bytes as f32
                    * 100.0,
            })
        })
        .collect::<Vec<_>>();
    disks.sort_by(|left, right| left.mount_point.cmp(&right.mount_point));
    disks.truncate(MAX_SYSMON_DISKS);

    let mut interface_indices = HashMap::<String, usize>::new();
    let mut network_interfaces = Vec::<SysmonNetworkInterface>::new();
    for interface in sample.network_interfaces {
        let name = bounded_sysmon_label(&interface.name, 64);
        if name.is_empty() {
            continue;
        }
        let addresses = normalize_sysmon_addresses(interface.addresses);
        let key = name.to_ascii_lowercase();
        if let Some(index) = interface_indices.get(&key).copied() {
            let merged_addresses = network_interfaces[index]
                .addresses
                .iter()
                .cloned()
                .chain(addresses)
                .collect();
            network_interfaces[index].addresses = normalize_sysmon_addresses(merged_addresses);
            continue;
        }
        interface_indices.insert(key, network_interfaces.len());
        network_interfaces.push(SysmonNetworkInterface {
            name,
            addresses,
            rx_bytes: interface.rx_bytes,
            tx_bytes: interface.tx_bytes,
            rx_kbps: bounded_sysmon_rate(interface.rx_kbps),
            tx_kbps: bounded_sysmon_rate(interface.tx_kbps),
        });
    }
    sort_sysmon_network_interfaces(&mut network_interfaces);
    network_interfaces.truncate(MAX_SYSMON_NETWORK_INTERFACES);
    let (rx_kbps, tx_kbps) = aggregate_network_rates(&network_interfaces);

    if sample.uptime_seconds == 0 && memory_total_bytes == 0 && cpu_percent == 0.0 {
        return Err("远端未提供 Windows Sysmon 数据".to_string());
    }

    Ok(SysmonSnapshot {
        session_id: session_id.to_string(),
        ts: sampled_at,
        uptime_seconds: sample.uptime_seconds,
        cpu_percent,
        memory_percent,
        rx_kbps,
        tx_kbps,
        load_average: [0.0; 3],
        memory_total_bytes,
        memory_available_bytes,
        processes,
        disks,
        network_interfaces,
    })
}
