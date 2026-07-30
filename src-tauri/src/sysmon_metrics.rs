use super::*;

pub(super) const MAX_SYSMON_PROCESSES: usize = 8;
pub(super) const MAX_SYSMON_DISKS: usize = 16;

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
