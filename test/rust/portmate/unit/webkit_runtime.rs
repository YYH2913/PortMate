use super::*;

#[test]
fn detects_vmware_from_dmi_identity() {
    assert!(dmi_identifies_vmware([
        "VMware, Inc.".to_owned(),
        "VMware20,1".to_owned(),
    ]));
    assert!(should_apply_vmware_webkit_fallback(None, true));
}

#[test]
fn keeps_webkit_defaults_on_non_vmware_linux_hosts() {
    assert!(!dmi_identifies_vmware([
        "Dell Inc.".to_owned(),
        "Precision 5680".to_owned(),
    ]));
    assert!(!should_apply_vmware_webkit_fallback(None, false));
}

#[test]
fn honors_an_explicit_webkit_renderer_override() {
    assert!(!should_apply_vmware_webkit_fallback(
        Some(OsStr::new("0")),
        true,
    ));
}

#[test]
fn repairs_stale_ibus_environment_when_fcitx5_is_active() {
    assert!(should_repair_fcitx5_environment(
        true,
        None,
        Some(OsStr::new("ibus")),
        Some(OsStr::new("@im=ibus")),
    ));
}

#[test]
fn keeps_coherent_or_custom_input_method_environment() {
    assert!(!should_repair_fcitx5_environment(
        false,
        None,
        Some(OsStr::new("ibus")),
        Some(OsStr::new("@im=ibus")),
    ));
    assert!(!should_repair_fcitx5_environment(
        true,
        Some(OsStr::new("fcitx")),
        Some(OsStr::new("fcitx")),
        Some(OsStr::new("@im=fcitx")),
    ));
    assert!(!should_repair_fcitx5_environment(
        true,
        Some(OsStr::new("xim")),
        None,
        None,
    ));
}
