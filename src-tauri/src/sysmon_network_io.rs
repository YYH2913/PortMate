use super::*;

#[cfg(target_os = "linux")]
pub(super) const LOCAL_LINUX_SYSMON_COMMAND_DIRECTORIES: [&str; 4] =
    ["/usr/sbin", "/usr/bin", "/sbin", "/bin"];
#[cfg(target_os = "linux")]
pub(super) const LOCAL_LINUX_ADDRESS_COMMAND_TIMEOUT: Duration = Duration::from_secs(2);
#[cfg(target_os = "linux")]
pub(super) const MAX_LOCAL_LINUX_KERNEL_ADDRESS_BYTES: usize = 64 * 1024;

#[cfg(target_os = "linux")]
pub(super) fn read_network_interfaces() -> Option<BTreeMap<String, (u64, u64)>> {
    let raw = fs::read_to_string("/proc/net/dev").ok()?;
    Some(parse_network_interfaces(&raw))
}

#[cfg(target_os = "linux")]
pub(super) fn read_network_addresses() -> BTreeMap<String, Vec<String>> {
    let mut addresses = read_linux_network_addresses_from_getifaddrs();
    merge_linux_network_address_maps(&mut addresses, read_local_linux_kernel_network_addresses());
    if addresses
        .values()
        .flatten()
        .any(|address| is_usable_sysmon_network_address(address))
    {
        return addresses;
    }

    merge_linux_network_address_maps(
        &mut addresses,
        read_local_linux_hostname_network_addresses(),
    );
    if addresses
        .values()
        .flatten()
        .any(|address| is_usable_sysmon_network_address(address))
    {
        return addresses;
    }

    let commands: [(&str, &[&str]); 3] = [
        ("ip", &["-o", "addr", "show"]),
        ("ip", &["addr", "show"]),
        ("ifconfig", &["-a"]),
    ];
    merge_linux_network_address_maps(
        &mut addresses,
        first_nonempty_linux_network_addresses(
            commands
                .into_iter()
                .filter_map(|(program, args)| exec_sync_sysmon_command(program, args)),
        ),
    );
    addresses
}

#[cfg(target_os = "linux")]
pub(super) fn read_local_linux_kernel_network_addresses() -> BTreeMap<String, Vec<String>> {
    let ipv6 = read_bounded_local_linux_proc_file(
        Path::new("/proc/net/if_inet6"),
        MAX_LOCAL_LINUX_KERNEL_ADDRESS_BYTES,
    )
    .unwrap_or_default();
    let ipv4 = read_bounded_local_linux_proc_file(
        Path::new("/proc/net/fib_trie"),
        MAX_LOCAL_LINUX_KERNEL_ADDRESS_BYTES,
    )
    .unwrap_or_default();
    let route = read_bounded_local_linux_proc_file(
        Path::new("/proc/net/route"),
        MAX_LOCAL_LINUX_KERNEL_ADDRESS_BYTES,
    )
    .unwrap_or_default();
    collect_linux_kernel_network_addresses(&ipv6, &ipv4, &route, "")
}

#[cfg(target_os = "linux")]
pub(super) fn read_local_linux_hostname_network_addresses() -> BTreeMap<String, Vec<String>> {
    let commands: [(&str, &[&str]); 4] = [
        ("hostname", &["-I"]),
        ("hostname", &["-i"]),
        ("busybox", &["hostname", "-I"]),
        ("busybox", &["hostname", "-i"]),
    ];
    let hostname = first_nonempty_linux_hostname_address_output(
        commands
            .into_iter()
            .filter_map(|(program, args)| exec_sync_sysmon_command(program, args)),
    );
    if hostname.is_empty() {
        return BTreeMap::new();
    }
    let route = read_bounded_local_linux_proc_file(
        Path::new("/proc/net/route"),
        MAX_LOCAL_LINUX_KERNEL_ADDRESS_BYTES,
    )
    .unwrap_or_default();
    collect_linux_kernel_network_addresses("", "", &route, &hostname)
}

#[cfg(target_os = "linux")]
pub(super) fn read_bounded_local_linux_proc_file(path: &Path, max_bytes: usize) -> Option<String> {
    let file = fs::File::open(path).ok()?;
    let mut bytes = Vec::with_capacity(max_bytes.min(8192).saturating_add(1));
    file.take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > max_bytes {
        return None;
    }
    String::from_utf8(bytes).ok()
}

#[cfg(target_os = "linux")]
struct LinuxInterfaceAddressList(*mut libc::ifaddrs);

#[cfg(target_os = "linux")]
impl Drop for LinuxInterfaceAddressList {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // A successful getifaddrs call transfers this allocation to freeifaddrs.
            unsafe { libc::freeifaddrs(self.0) };
        }
    }
}

#[cfg(target_os = "linux")]
pub(super) fn read_linux_network_addresses_from_getifaddrs() -> BTreeMap<String, Vec<String>> {
    let mut raw_addresses = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut raw_addresses) } != 0 || raw_addresses.is_null() {
        return BTreeMap::new();
    }
    let addresses_guard = LinuxInterfaceAddressList(raw_addresses);
    let mut addresses = BTreeMap::<String, Vec<String>>::new();
    let mut current = addresses_guard.0;
    while !current.is_null() {
        // getifaddrs returns a null-terminated linked list whose entries remain valid until freeifaddrs.
        let entry = unsafe { &*current };
        current = entry.ifa_next;
        if entry.ifa_name.is_null() || entry.ifa_addr.is_null() {
            continue;
        }
        let name = unsafe { CStr::from_ptr(entry.ifa_name) }.to_string_lossy();
        let name = normalize_linux_sysmon_interface_name(&name);
        if name.is_empty() {
            continue;
        }
        let Some(address) = linux_sysmon_interface_address(
            entry.ifa_addr.cast_const(),
            entry.ifa_netmask.cast_const(),
        ) else {
            continue;
        };
        push_sysmon_network_address(addresses.entry(name).or_default(), address);
    }
    addresses
}

#[cfg(target_os = "linux")]
pub(super) fn linux_sysmon_interface_address(
    address: *const libc::sockaddr,
    netmask: *const libc::sockaddr,
) -> Option<String> {
    if address.is_null() {
        return None;
    }
    let family = unsafe { (*address).sa_family as libc::c_int };
    match family {
        libc::AF_INET => {
            let socket = unsafe { &*address.cast::<libc::sockaddr_in>() };
            let address = Ipv4Addr::from(socket.sin_addr.s_addr.to_ne_bytes()).to_string();
            Some(append_linux_sysmon_prefix(
                address,
                linux_sysmon_netmask_prefix(netmask, family),
            ))
        }
        libc::AF_INET6 => {
            let socket = unsafe { &*address.cast::<libc::sockaddr_in6>() };
            let address = Ipv6Addr::from(socket.sin6_addr.s6_addr).to_string();
            Some(append_linux_sysmon_prefix(
                address,
                linux_sysmon_netmask_prefix(netmask, family),
            ))
        }
        _ => None,
    }
}

#[cfg(target_os = "linux")]
pub(super) fn append_linux_sysmon_prefix(address: String, prefix: Option<u8>) -> String {
    match prefix {
        Some(prefix) => format!("{address}/{prefix}"),
        None => address,
    }
}

#[cfg(target_os = "linux")]
pub(super) fn linux_sysmon_netmask_prefix(
    netmask: *const libc::sockaddr,
    expected_family: libc::c_int,
) -> Option<u8> {
    if netmask.is_null() || unsafe { (*netmask).sa_family as libc::c_int } != expected_family {
        return None;
    }
    match expected_family {
        libc::AF_INET => {
            let socket = unsafe { &*netmask.cast::<libc::sockaddr_in>() };
            linux_sysmon_prefix_length(&socket.sin_addr.s_addr.to_ne_bytes())
        }
        libc::AF_INET6 => {
            let socket = unsafe { &*netmask.cast::<libc::sockaddr_in6>() };
            linux_sysmon_prefix_length(&socket.sin6_addr.s6_addr)
        }
        _ => None,
    }
}

#[cfg(target_os = "linux")]
pub(super) fn linux_sysmon_prefix_length(mask: &[u8]) -> Option<u8> {
    let mut prefix = 0_u8;
    let mut encountered_zero = false;
    for byte in mask {
        for bit in (0..8).rev() {
            if byte & (1_u8 << bit) != 0 {
                if encountered_zero {
                    return None;
                }
                prefix = prefix.checked_add(1)?;
            } else {
                encountered_zero = true;
            }
        }
    }
    Some(prefix)
}

pub(super) fn first_nonempty_linux_network_addresses(
    sources: impl IntoIterator<Item = String>,
) -> BTreeMap<String, Vec<String>> {
    for source in sources {
        let addresses = parse_linux_network_addresses(&source);
        if !addresses.is_empty() {
            return addresses;
        }
    }
    BTreeMap::new()
}

pub(super) fn first_nonempty_linux_hostname_address_output(
    sources: impl IntoIterator<Item = String>,
) -> String {
    sources
        .into_iter()
        .find(|source| !parse_linux_hostname_network_addresses(source).is_empty())
        .unwrap_or_default()
}

pub(super) fn merge_linux_network_address_maps(
    target: &mut BTreeMap<String, Vec<String>>,
    source: BTreeMap<String, Vec<String>>,
) {
    for (name, candidates) in source {
        let entry = target.entry(name).or_default();
        for candidate in candidates {
            let Some(candidate) = normalize_sysmon_address(&candidate) else {
                continue;
            };
            push_sysmon_network_address(entry, candidate);
        }
    }
}

#[cfg(target_os = "linux")]
pub(super) fn exec_sync_sysmon_command(program: &str, args: &[&str]) -> Option<String> {
    let deadline = Instant::now() + LOCAL_LINUX_ADDRESS_COMMAND_TIMEOUT;
    for candidate in linux_sysmon_command_candidates(program) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        if let Some(output) = exec_bounded_sync_sysmon_command(
            &candidate,
            args,
            remaining,
            MAX_LOCAL_SYSMON_STDOUT_BYTES,
        ) {
            return Some(output);
        }
    }
    None
}

#[cfg(target_os = "linux")]
pub(super) fn exec_bounded_sync_sysmon_command(
    program: &str,
    args: &[&str],
    timeout: Duration,
    max_bytes: usize,
) -> Option<String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // The fallback command may be a shell wrapper. Isolate it so timeout cleanup also
    // closes stdout inherited by any descendants before the reader can block indefinitely.
    command.process_group(0);
    let mut child = command.spawn().ok()?;
    let process_id = child.id();
    let stdout = child.stdout.take()?;
    let (capture_sender, capture_receiver) = std::sync::mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let result = (|| -> std::io::Result<(Vec<u8>, bool)> {
            let mut reader = BufReader::new(stdout);
            let mut output = Vec::with_capacity(max_bytes.min(8192));
            let mut overflow = false;
            let mut chunk = [0_u8; 8192];
            loop {
                let count = reader.read(&mut chunk)?;
                if count == 0 {
                    break;
                }
                let available = max_bytes.saturating_sub(output.len());
                if count > available {
                    overflow = true;
                }
                if available > 0 {
                    output.extend_from_slice(&chunk[..count.min(available)]);
                }
            }
            Ok((output, overflow))
        })();
        let _ = capture_sender.send(result);
    });

    let deadline = Instant::now() + timeout;
    let mut status = None;
    let mut capture = None;
    let mut timed_out = false;
    loop {
        if status.is_none() {
            match child.try_wait() {
                Ok(next_status) => status = next_status,
                Err(_) => timed_out = true,
            }
        }
        if capture.is_none() {
            match capture_receiver.try_recv() {
                Ok(next_capture) => capture = Some(next_capture),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => timed_out = true,
            }
        }
        if status.is_some() && capture.is_some() {
            break;
        }
        if timed_out || Instant::now() >= deadline {
            timed_out = true;
            if process_id <= i32::MAX as u32 {
                // The child created this process group, so descendants that inherited stdout
                // are terminated with their command rather than holding the reader open.
                unsafe {
                    libc::kill(-(process_id as i32), libc::SIGKILL);
                }
            }
            let _ = child.kill();
            if status.is_none() {
                status = child.wait().ok();
            }
            if capture.is_none() {
                capture = capture_receiver
                    .recv_timeout(Duration::from_millis(250))
                    .ok();
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let capture = capture?;
    let _ = reader.join();
    let (output, overflow) = capture.ok()?;
    if timed_out || !status.is_some_and(|status| status.success()) || overflow {
        return None;
    }
    Some(String::from_utf8_lossy(&output).to_string())
}

#[cfg(target_os = "linux")]
pub(super) fn linux_sysmon_command_candidates(program: &str) -> Vec<String> {
    std::iter::once(program.to_string())
        .chain(
            LOCAL_LINUX_SYSMON_COMMAND_DIRECTORIES
                .iter()
                .map(|directory| format!("{directory}/{program}")),
        )
        .collect()
}

#[cfg(not(target_os = "linux"))]
pub(super) fn read_network_interfaces() -> Option<BTreeMap<String, (u64, u64)>> {
    None
}
