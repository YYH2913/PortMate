#[cfg(feature = "vendored")]
#[test]
fn vendored_runtime_reports_its_source_version() {
    let version = unsafe { std::ffi::CStr::from_ptr(super::ssh_version(0)) }.to_string_lossy();
    assert!(
        version.starts_with("0.11.4/"),
        "unexpected vendored libssh version: {}",
        version
    );
}

#[test]
fn socket_type_matches_platform_handle() {
    #[cfg(windows)]
    assert_eq!(
        std::mem::size_of::<super::socket_t>(),
        std::mem::size_of::<std::os::windows::io::RawSocket>()
    );

    #[cfg(not(windows))]
    assert_eq!(
        std::mem::size_of::<super::socket_t>(),
        std::mem::size_of::<std::os::raw::c_int>()
    );
}

#[test]
fn hardlink_binding_is_optional_on_older_system_libraries() {
    let result = unsafe {
        super::sftp_hardlink_if_available(std::ptr::null_mut(), std::ptr::null(), std::ptr::null())
    };
    #[cfg(libssh_has_sftp_hardlink)]
    assert_eq!(result, Some(super::SSH_ERROR));
    #[cfg(not(libssh_has_sftp_hardlink))]
    assert_eq!(result, None);
}
