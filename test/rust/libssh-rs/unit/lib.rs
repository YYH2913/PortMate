use super::*;

fn generated_private_key() -> SshKey {
    let mut key = std::ptr::null_mut();
    let status =
        unsafe { sys::ssh_pki_generate(sys::ssh_keytypes_e_SSH_KEYTYPE_ED25519, 0, &mut key) };
    SshKey::from_ffi_result(status, key, Error::fatal("failed to generate test key")).unwrap()
}

fn export_private_key_base64(key: &SshKey) -> String {
    let mut encoded = std::ptr::null_mut();
    let status = unsafe {
        sys::ssh_pki_export_privkey_base64(
            key.key,
            std::ptr::null(),
            None,
            std::ptr::null_mut(),
            &mut encoded,
        )
    };
    assert_eq!(status, sys::SSH_OK as c_int);
    assert!(!encoded.is_null());
    let value = unsafe { CStr::from_ptr(encoded) }
        .to_string_lossy()
        .into_owned();
    unsafe { sys::ssh_string_free_char(encoded) };
    value
}

#[test]
fn init() {
    let sess = Session::new().unwrap();
    assert!(!sess.is_connected());
    assert_eq!(sess.connect(), Err(Error::fatal("Hostname required")));
}

#[test]
fn gssapi_entrypoint_links_without_calling_an_unconnected_session() {
    let entrypoint: fn(&Session) -> SshResult<AuthStatus> = Session::userauth_gssapi;
    std::hint::black_box(entrypoint);
}

#[test]
fn send_ignore_rejects_embedded_nul_without_panicking() {
    let sess = Session::new().unwrap();
    let error = sess.send_ignore(b"prefix\0suffix").unwrap_err();
    assert!(matches!(error, Error::Fatal(message) if message.contains("nul byte")));
}

#[test]
fn ffi_numeric_conversions_do_not_wrap_or_panic() {
    let huge = Duration::from_secs(u64::MAX);
    assert_eq!(duration_millis_c_int(Duration::from_millis(17)), 17);
    assert_eq!(duration_millis_c_int(huge), c_int::MAX);
    assert_eq!(duration_millis_u32(huge), u32::MAX);
    assert_eq!(duration_micros_c_long(huge), c_long::MAX);
    assert_eq!(ffi_io_count(usize::MAX), c_int::MAX as u32);
    assert_eq!(
        checked_c_int(c_int::MAX as u32, "PTY columns"),
        Ok(c_int::MAX)
    );
    assert!(matches!(
        checked_c_int(c_int::MAX as u32 + 1, "PTY columns"),
        Err(Error::Fatal(message)) if message.contains("PTY columns")
    ));
    assert_eq!(checked_ffi_port(0, "port"), Ok(0));
    assert_eq!(checked_ffi_port(u16::MAX as c_int, "port"), Ok(u16::MAX));
    assert!(checked_ffi_port(-1, "port").is_err());
    assert!(checked_ffi_port(u16::MAX as c_int + 1, "port").is_err());

    let sess = Session::new().unwrap();
    sess.set_option(SshOption::Timeout(huge)).unwrap();
    sess.set_option(SshOption::ProcessConfig(true)).unwrap();
    sess.set_option(SshOption::ProcessConfig(false)).unwrap();
}

#[test]
fn deadline_timeout_is_computed_after_the_session_lock_is_acquired() {
    let session = Session::new().unwrap();
    let locked_session = Arc::clone(&session.sess);
    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let _guard = locked_session.lock().unwrap();
        locked_tx.send(()).unwrap();
        std::thread::sleep(Duration::from_millis(80));
    });
    locked_rx.recv().unwrap();

    let error = session
        .set_timeout_until(Instant::now() + Duration::from_millis(20))
        .unwrap_err();
    assert!(matches!(
        error,
        Error::Fatal(message) if message.contains("deadline expired")
    ));
    holder.join().unwrap();

    let remaining = session
        .set_timeout_until(Instant::now() + Duration::from_secs(1))
        .unwrap();
    assert!(!remaining.is_zero());
    assert!(remaining <= Duration::from_secs(1));
}

#[test]
fn timeout_scoped_operations_share_a_gate_across_session_handles() {
    let session = Session::new().unwrap();
    let sftp = Sftp::new(
        Arc::clone(&session.sess),
        Arc::clone(&session.operation_gate),
        std::ptr::null_mut(),
    );
    let (held_tx, held_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let holder_session = session.clone();
    let holder = std::thread::spawn(move || {
        holder_session
            .with_session_operation_until(Instant::now() + Duration::from_secs(1), || {
                holder_session
                    .set_timeout_until(Instant::now() + Duration::from_secs(1))
                    .unwrap();
                held_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            })
            .unwrap();
    });
    held_rx.recv().unwrap();

    let error = sftp
        .with_session_operation_until(Instant::now() + Duration::from_millis(20), || ())
        .unwrap_err();
    assert!(matches!(
        error,
        Error::Fatal(message) if message.contains("session operation deadline expired")
    ));

    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let waiter = std::thread::spawn(move || {
        sftp.with_session_operation_until(Instant::now() + Duration::from_secs(1), || {
            entered_tx.send(()).unwrap()
        })
        .unwrap();
    });
    assert!(entered_rx.recv_timeout(Duration::from_millis(30)).is_err());
    release_tx.send(()).unwrap();
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    holder.join().unwrap();
    waiter.join().unwrap();

    let mut expired_operation_ran = false;
    let expired = session
        .with_session_operation_until(
            Instant::now()
                .checked_sub(Duration::from_millis(1))
                .unwrap(),
            || expired_operation_ran = true,
        )
        .unwrap_err();
    assert!(!expired_operation_ran);
    assert!(matches!(
        expired,
        Error::Fatal(message) if message.contains("session operation deadline expired")
    ));
}

#[test]
fn optional_c_strings_reject_embedded_nul_instead_of_becoming_none() {
    assert!(opt_str_to_cstring(None).unwrap().is_none());
    assert_eq!(
        opt_str_to_cstring(Some("portmate"))
            .unwrap()
            .unwrap()
            .as_bytes(),
        b"portmate"
    );
    assert!(matches!(
        opt_str_to_cstring(Some("user\0fallback")),
        Err(Error::Fatal(message)) if message.contains("nul byte")
    ));

    let sess = Session::new().unwrap();
    assert!(matches!(
        sess.set_option(SshOption::User(Some("user\0fallback".to_string()))),
        Err(Error::Fatal(message)) if message.contains("nul byte")
    ));
    assert!(matches!(
        sess.userauth_password(None, Some("secret\0ignored")),
        Err(Error::Fatal(message)) if message.contains("nul byte")
    ));
}

fn invoke_auth_callback<F>(callback: F) -> [u8; 16]
where
    F: FnMut(&str, bool, bool, Option<String>) -> SshResult<String> + Send + 'static,
{
    let session = Session::new().unwrap();
    session.set_auth_callback(callback);
    let prompt = CString::new("passphrase").unwrap();
    let mut buf = [0xa5; 16];
    let status = {
        let mut sess = session.sess.lock().unwrap();
        let userdata = &mut *sess as *mut SessionHolder as *mut _;
        unsafe {
            Session::bridge_auth_callback(
                prompt.as_ptr(),
                buf.as_mut_ptr() as *mut _,
                buf.len(),
                0,
                0,
                userdata,
            )
        }
    };
    assert_eq!(status, sys::SSH_ERROR);
    buf
}

#[test]
fn auth_callback_failures_clear_the_ffi_output_buffer() {
    let error_buffer =
        invoke_auth_callback(|_, _, _, _| Err(Error::fatal("test callback failure")));
    assert_eq!(error_buffer, [0; 16]);

    let panic_buffer =
        invoke_auth_callback(|_, _, _, _| -> SshResult<String> { panic!("test callback panic") });
    assert_eq!(panic_buffer, [0; 16]);
}

#[test]
fn auth_agent_callback_rejects_invalid_context() {
    let channel = unsafe {
        Session::channel_open_request_auth_agent_callback(
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert!(channel.is_null());
}

#[test]
fn auth_callback_response_is_bounded_terminated_and_cleared() {
    let mut buf = [0xa5; 8];
    write_auth_callback_response(&mut buf, "secret").unwrap();
    assert_eq!(&buf, b"secret\0\0");

    write_auth_callback_response(&mut buf, "1234567").unwrap();
    assert_eq!(&buf, b"1234567\0");

    let error = write_auth_callback_response(&mut buf, "12345678").unwrap_err();
    assert!(matches!(error, Error::Fatal(message) if message.contains("NUL terminator")));
    assert_eq!(buf, [0; 8]);

    let error = write_auth_callback_response(&mut buf, "secret\0tail").unwrap_err();
    assert!(matches!(error, Error::Fatal(message) if message.contains("embedded NUL")));
    assert_eq!(buf, [0; 8]);
}

#[test]
fn keyboard_interactive_info_rejects_missing_or_oversized_data() {
    let sess = Session::new().unwrap();
    assert!(sess.userauth_keyboard_interactive_info().is_err());

    let exact = CString::new(vec![b'x'; MAX_KEYBOARD_INTERACTIVE_TEXT_BYTES]).unwrap();
    let value = unsafe {
        bounded_c_string(
            exact.as_ptr(),
            "keyboard-interactive prompt",
            MAX_KEYBOARD_INTERACTIVE_TEXT_BYTES,
        )
    }
    .unwrap();
    assert_eq!(value.len(), MAX_KEYBOARD_INTERACTIVE_TEXT_BYTES);

    let oversized = CString::new(vec![b'x'; MAX_KEYBOARD_INTERACTIVE_TEXT_BYTES + 1]).unwrap();
    assert!(matches!(
        unsafe {
            bounded_c_string(
                oversized.as_ptr(),
                "keyboard-interactive prompt",
                MAX_KEYBOARD_INTERACTIVE_TEXT_BYTES,
            )
        },
        Err(Error::Fatal(message)) if message.contains("byte limit")
    ));
    assert!(matches!(
        unsafe {
            bounded_c_string(
                std::ptr::null(),
                "keyboard-interactive prompt",
                MAX_KEYBOARD_INTERACTIVE_TEXT_BYTES,
            )
        },
        Err(Error::Fatal(message)) if message.contains("null")
    ));
}

#[test]
fn getpass_default_always_leaves_a_nul_terminator() {
    let mut buf = [0xa5; 8];
    initialize_getpass_buffer(&mut buf, Some("portmat")).unwrap();
    assert_eq!(&buf, b"portmat\0");

    assert!(initialize_getpass_buffer(&mut buf, Some("portmate")).is_none());
    assert_eq!(buf, [0; 8]);
    assert!(initialize_getpass_buffer(&mut buf, Some("bad\0default")).is_none());
    assert_eq!(buf, [0; 8]);

    initialize_getpass_buffer(&mut buf, None).unwrap();
    assert_eq!(buf, [0; 8]);
}

#[test]
fn owned_ssh_strings_are_converted_and_released_safely() {
    let text = CString::new("portmate-public-key").unwrap();
    let value = unsafe { sys::ssh_string_from_char(text.as_ptr()) };
    assert_eq!(
        take_ssh_string(value, "test value").unwrap(),
        "portmate-public-key"
    );
    assert!(matches!(
        take_ssh_string(std::ptr::null_mut(), "test value"),
        Err(Error::Fatal(message)) if message.contains("null test value")
    ));
}

#[test]
fn private_key_imports_require_a_successful_non_null_output() {
    let generated = generated_private_key();
    let encoded = export_private_key_base64(&generated);
    let imported = SshKey::from_privkey_base64(&encoded, None).unwrap();
    assert_eq!(imported.key_type_name().unwrap(), "ssh-ed25519");
    assert!(!imported.export_public_key_base64().unwrap().is_empty());
    assert_eq!(
        imported
            .get_public_key_hash(PublicKeyHashType::Sha256)
            .unwrap()
            .len(),
        32
    );
    assert!(!imported
        .get_public_key_hash_hexa(PublicKeyHashType::Sha256)
        .unwrap()
        .is_empty());

    assert!(SshKey::from_privkey_base64("not a private key", None).is_err());
    assert!(matches!(
        SshKey::from_ffi_result(
            sys::SSH_OK as c_int,
            std::ptr::null_mut(),
            Error::fatal("missing imported key"),
        ),
        Err(Error::Fatal(message)) if message == "missing imported key"
    ));

    let rejected = generated_private_key();
    let rejected_raw = rejected.key;
    std::mem::forget(rejected);
    assert!(matches!(
        SshKey::from_ffi_result(
            sys::SSH_ERROR,
            rejected_raw,
            Error::RequestDenied("preserved failure".to_string()),
        ),
        Err(Error::RequestDenied(message)) if message == "preserved failure"
    ));
}

#[test]
fn private_key_file_import_accepts_a_valid_export() {
    static NEXT_TEMP_KEY_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    let generated = generated_private_key();
    let path = std::env::temp_dir().join(format!(
        "portmate-libssh-import-{}-{}.key",
        std::process::id(),
        NEXT_TEMP_KEY_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let path_cstr = CString::new(path.to_string_lossy().as_bytes()).unwrap();
    let status = unsafe {
        sys::ssh_pki_export_privkey_file(
            generated.key,
            std::ptr::null(),
            None,
            std::ptr::null_mut(),
            path_cstr.as_ptr(),
        )
    };
    assert_eq!(status, sys::SSH_OK as c_int);

    let imported = SshKey::from_privkey_file(path.to_str().unwrap(), None).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(imported.key_type_name().unwrap(), "ssh-ed25519");
}
