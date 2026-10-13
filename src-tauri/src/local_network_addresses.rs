use serde::Serialize;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalNetworkAddress {
    interface_id: String,
    interface_name: String,
    address: String,
    family: &'static str,
    loopback: bool,
}

fn address_entry(id: String, name: String, ip: IpAddr) -> Option<LocalNetworkAddress> {
    // A client URL must be usable without a host-specific scope ID. Multicast,
    // unspecified and IPv6 link-local addresses cannot serve that purpose.
    if ip.is_unspecified()
        || ip.is_multicast()
        || matches!(ip, IpAddr::V6(value) if value.is_unicast_link_local())
    {
        return None;
    }
    Some(LocalNetworkAddress {
        interface_id: id,
        interface_name: name,
        address: ip.to_string(),
        family: if ip.is_ipv4() { "ipv4" } else { "ipv6" },
        loopback: ip.is_loopback(),
    })
}

#[tauri::command]
pub(crate) async fn list_local_network_addresses() -> Result<Vec<LocalNetworkAddress>, String> {
    tokio::task::spawn_blocking(|| {
        let mut addresses = read_addresses()?;
        addresses.sort_by(|a, b| {
            (a.loopback, &a.interface_name, &a.family, &a.address).cmp(&(
                b.loopback,
                &b.interface_name,
                &b.family,
                &b.address,
            ))
        });
        addresses.dedup_by(|a, b| a.interface_id == b.interface_id && a.address == b.address);
        addresses.truncate(1024);
        Ok(addresses)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg(unix)]
fn read_addresses() -> Result<Vec<LocalNetworkAddress>, String> {
    use std::ffi::CStr;
    struct AddressList(*mut libc::ifaddrs);
    impl Drop for AddressList {
        fn drop(&mut self) {
            // The successful getifaddrs call owns the whole linked list.
            unsafe { libc::freeifaddrs(self.0) };
        }
    }
    let mut raw = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut raw) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    let list = AddressList(raw);
    let mut current = list.0;
    let mut addresses = Vec::new();
    while !current.is_null() {
        // Pointers and socket addresses remain valid until AddressList drops.
        let entry = unsafe { &*current };
        current = entry.ifa_next;
        if entry.ifa_addr.is_null()
            || entry.ifa_name.is_null()
            || entry.ifa_flags & libc::IFF_UP as u32 == 0
        {
            continue;
        }
        let name = unsafe { CStr::from_ptr(entry.ifa_name) }
            .to_string_lossy()
            .into_owned();
        let family = unsafe { (*entry.ifa_addr).sa_family } as i32;
        let ip = match family {
            libc::AF_INET => {
                let socket = unsafe { &*entry.ifa_addr.cast::<libc::sockaddr_in>() };
                IpAddr::V4(Ipv4Addr::from(socket.sin_addr.s_addr.to_ne_bytes()))
            }
            libc::AF_INET6 => {
                let socket = unsafe { &*entry.ifa_addr.cast::<libc::sockaddr_in6>() };
                IpAddr::V6(Ipv6Addr::from(socket.sin6_addr.s6_addr))
            }
            _ => continue,
        };
        if let Some(address) = address_entry(name.clone(), name, ip) {
            addresses.push(address);
        }
    }
    Ok(addresses)
}

#[cfg(windows)]
fn read_addresses() -> Result<Vec<LocalNetworkAddress>, String> {
    use std::ffi::CStr;
    use windows_sys::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_NO_DATA, NO_ERROR};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
        GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
    use windows_sys::Win32::Networking::WinSock::{
        AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR_IN, SOCKADDR_IN6,
    };

    let mut size = 16 * 1024u32;
    for _ in 0..3 {
        if size > 4 * 1024 * 1024 {
            return Err(
                std::io::Error::from_raw_os_error(ERROR_BUFFER_OVERFLOW as i32).to_string(),
            );
        }
        // The API writes a buffer of linked structs. u64 storage provides the
        // alignment required by Windows IP_ADAPTER_ADDRESSES on supported hosts.
        let mut storage = vec![0u64; (size as usize).div_ceil(8)];
        let first = storage.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        let result = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC as u32,
                GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER,
                std::ptr::null(),
                first,
                &mut size,
            )
        };
        if result == ERROR_BUFFER_OVERFLOW {
            continue;
        }
        if result == ERROR_NO_DATA {
            return Ok(Vec::new());
        }
        if result != NO_ERROR {
            return Err(std::io::Error::from_raw_os_error(result as i32).to_string());
        }
        let mut addresses = Vec::new();
        let mut current = first;
        while !current.is_null() {
            // Returned pointers refer to the live storage buffer above.
            let adapter = unsafe { &*current };
            current = adapter.Next;
            if adapter.OperStatus != IfOperStatusUp || adapter.AdapterName.is_null() {
                continue;
            }
            let id = unsafe { CStr::from_ptr(adapter.AdapterName.cast()) }
                .to_string_lossy()
                .into_owned();
            let name = if adapter.FriendlyName.is_null() {
                id.clone()
            } else {
                let mut length = 0;
                while length < 32768 && unsafe { *adapter.FriendlyName.add(length) } != 0 {
                    length += 1;
                }
                String::from_utf16_lossy(unsafe {
                    std::slice::from_raw_parts(adapter.FriendlyName, length)
                })
            };
            let mut unicast = adapter.FirstUnicastAddress;
            while !unicast.is_null() {
                let entry = unsafe { &*unicast };
                unicast = entry.Next;
                let socket = entry.Address.lpSockaddr;
                if socket.is_null() {
                    continue;
                }
                let ip = match unsafe { (*socket).sa_family } {
                    AF_INET
                        if entry.Address.iSockaddrLength as usize
                            >= std::mem::size_of::<SOCKADDR_IN>() =>
                    {
                        let socket = unsafe { &*socket.cast::<SOCKADDR_IN>() };
                        IpAddr::V4(Ipv4Addr::from(
                            unsafe { socket.sin_addr.S_un.S_addr }.to_ne_bytes(),
                        ))
                    }
                    AF_INET6
                        if entry.Address.iSockaddrLength as usize
                            >= std::mem::size_of::<SOCKADDR_IN6>() =>
                    {
                        let socket = unsafe { &*socket.cast::<SOCKADDR_IN6>() };
                        IpAddr::V6(Ipv6Addr::from(unsafe { socket.sin6_addr.u.Byte }))
                    }
                    _ => continue,
                };
                if let Some(address) = address_entry(id.clone(), name.clone(), ip) {
                    addresses.push(address);
                }
            }
        }
        return Ok(addresses);
    }
    Err(std::io::Error::from_raw_os_error(ERROR_BUFFER_OVERFLOW as i32).to_string())
}

#[cfg(not(any(unix, windows)))]
fn read_addresses() -> Result<Vec<LocalNetworkAddress>, String> {
    Ok(Vec::new())
}

#[cfg(test)]
#[path = "../../test/rust/portmate/unit/local_network_addresses.rs"]
mod tests;
