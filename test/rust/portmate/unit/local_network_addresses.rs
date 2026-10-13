use super::*;

#[test]
fn usable_network_addresses_keep_adapter_identity_and_family() {
    for (ip, family, loopback) in [
        ("192.0.2.42", "ipv4", false),
        ("127.0.0.1", "ipv4", true),
        ("2001:db8::42", "ipv6", false),
        ("::1", "ipv6", true),
    ] {
        let entry = address_entry(
            "adapter-guid".into(),
            "Ethernet 2".into(),
            ip.parse().unwrap(),
        )
        .unwrap();
        assert_eq!(entry.interface_id, "adapter-guid");
        assert_eq!(entry.interface_name, "Ethernet 2");
        assert_eq!(entry.family, family);
        assert_eq!(entry.loopback, loopback);
        assert_eq!(entry.address, ip);
    }
}

#[test]
fn unreachable_url_addresses_are_excluded() {
    for ip in ["0.0.0.0", "::", "224.0.0.1", "ff02::1", "fe80::1234"] {
        assert!(address_entry("nic".into(), "NIC".into(), ip.parse().unwrap()).is_none());
    }
}

#[test]
fn native_address_enumeration_only_returns_numeric_unicast_hosts() {
    for entry in read_addresses().unwrap() {
        let ip: IpAddr = entry.address.parse().unwrap();
        assert!(!entry.interface_id.is_empty());
        assert!(address_entry(entry.interface_id, entry.interface_name, ip).is_some());
    }
}
