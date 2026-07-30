#[cfg(target_os = "linux")]
use std::ffi::OsStr;

#[cfg(target_os = "linux")]
const DMI_IDENTITY_PATHS: [&str; 3] = [
    "/sys/class/dmi/id/sys_vendor",
    "/sys/class/dmi/id/product_name",
    "/sys/class/dmi/id/board_vendor",
];

pub(super) fn configure_webkit_runtime() {
    #[cfg(target_os = "linux")]
    {
        if !should_disable_dmabuf_renderer(
            std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").as_deref(),
            DMI_IDENTITY_PATHS
                .iter()
                .filter_map(|path| std::fs::read_to_string(path).ok()),
        ) {
            return;
        }

        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        eprintln!("PortMate: disabled WebKit DMABUF rendering for VMware compatibility");
    }
}

#[cfg(target_os = "linux")]
fn should_disable_dmabuf_renderer(
    explicit_setting: Option<&OsStr>,
    dmi_identity: impl IntoIterator<Item = String>,
) -> bool {
    explicit_setting.is_none()
        && dmi_identity
            .into_iter()
            .any(|value| value.to_ascii_lowercase().contains("vmware"))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn detects_vmware_from_dmi_identity() {
        assert!(should_disable_dmabuf_renderer(
            None,
            ["VMware, Inc.".to_owned(), "VMware20,1".to_owned()],
        ));
    }

    #[test]
    fn keeps_webkit_defaults_on_non_vmware_linux_hosts() {
        assert!(!should_disable_dmabuf_renderer(
            None,
            ["Dell Inc.".to_owned(), "Precision 5680".to_owned()],
        ));
    }

    #[test]
    fn honors_an_explicit_webkit_renderer_override() {
        assert!(!should_disable_dmabuf_renderer(
            Some(OsStr::new("0")),
            ["VMware, Inc.".to_owned()],
        ));
    }
}
