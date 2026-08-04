#[cfg(target_os = "linux")]
use std::ffi::OsStr;
#[cfg(target_os = "linux")]
use std::os::unix::fs::MetadataExt;

#[cfg(target_os = "linux")]
const DMI_IDENTITY_PATHS: [&str; 3] = [
    "/sys/class/dmi/id/sys_vendor",
    "/sys/class/dmi/id/product_name",
    "/sys/class/dmi/id/board_vendor",
];

pub(super) fn configure_webkit_runtime() {
    #[cfg(target_os = "linux")]
    {
        if should_repair_fcitx5_environment(
            linux_process_is_running("fcitx5"),
            std::env::var_os("GTK_IM_MODULE").as_deref(),
            std::env::var_os("QT_IM_MODULE").as_deref(),
            std::env::var_os("XMODIFIERS").as_deref(),
        ) {
            std::env::set_var("GTK_IM_MODULE", "fcitx");
            std::env::set_var("QT_IM_MODULE", "fcitx");
            std::env::set_var("XMODIFIERS", "@im=fcitx");
            eprintln!("PortMate: repaired stale IBus environment for the active Fcitx5 session");
        }

        let vmware = dmi_identifies_vmware(
            DMI_IDENTITY_PATHS
                .iter()
                .filter_map(|path| std::fs::read_to_string(path).ok()),
        );
        if should_apply_vmware_webkit_fallback(
            std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").as_deref(),
            vmware,
        ) {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
            eprintln!("PortMate: disabled WebKit DMABUF rendering for VMware compatibility");
        }
        if should_apply_vmware_webkit_fallback(
            std::env::var_os("WEBKIT_DISABLE_COMPOSITING_MODE").as_deref(),
            vmware,
        ) {
            std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
            eprintln!("PortMate: disabled WebKit accelerated compositing for VMware compatibility");
        }
    }
}

#[cfg(target_os = "linux")]
fn linux_process_is_running(name: &str) -> bool {
    let current_uid = std::fs::metadata("/proc/self")
        .map(|metadata| metadata.uid())
        .ok();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return false;
    };
    entries.filter_map(Result::ok).any(|entry| {
        let file_name = entry.file_name();
        if !file_name.as_encoded_bytes().iter().all(u8::is_ascii_digit) {
            return false;
        }
        let process_path = entry.path();
        if current_uid.is_some_and(|uid| {
            std::fs::metadata(&process_path)
                .map(|metadata| metadata.uid() != uid)
                .unwrap_or(true)
        }) {
            return false;
        }
        std::fs::read_to_string(process_path.join("comm")).is_ok_and(|comm| comm.trim() == name)
    })
}

#[cfg(target_os = "linux")]
fn should_repair_fcitx5_environment(
    fcitx5_running: bool,
    gtk_module: Option<&OsStr>,
    qt_module: Option<&OsStr>,
    xmodifiers: Option<&OsStr>,
) -> bool {
    fn is_missing_or(value: Option<&OsStr>, expected: &str) -> bool {
        value.is_none_or(|value| value.to_string_lossy().eq_ignore_ascii_case(expected))
    }

    fcitx5_running
        && is_missing_or(gtk_module, "ibus")
        && is_missing_or(qt_module, "ibus")
        && is_missing_or(xmodifiers, "@im=ibus")
}

#[cfg(target_os = "linux")]
fn dmi_identifies_vmware(dmi_identity: impl IntoIterator<Item = String>) -> bool {
    dmi_identity
        .into_iter()
        .any(|value| value.to_ascii_lowercase().contains("vmware"))
}

#[cfg(target_os = "linux")]
fn should_apply_vmware_webkit_fallback(explicit_setting: Option<&OsStr>, vmware: bool) -> bool {
    explicit_setting.is_none() && vmware
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
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
}
