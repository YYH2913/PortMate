use super::*;

#[test]
fn exit_signal_callback_tolerates_missing_and_poisoned_state() {
    unsafe {
        handle_exit_signal(
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null(),
            0,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null_mut(),
        );
    }

    let callback_state = CallbackState {
        signal_state: Mutex::new(None),
    };
    let _ = std::panic::catch_unwind(|| {
        let _guard = callback_state.signal_state.lock().unwrap();
        panic!("poison callback state for regression coverage");
    });
    let signal = CString::new("TERM").unwrap();
    let error = CString::new("terminated").unwrap();
    let language = CString::new("en-US").unwrap();

    unsafe {
        handle_exit_signal(
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            signal.as_ptr(),
            1,
            error.as_ptr(),
            language.as_ptr(),
            &callback_state as *const CallbackState as *mut _,
        );
    }
    let state = callback_state
        .signal_state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
        .unwrap();
    assert_eq!(state.signal_name.as_deref(), Some("TERM"));
    assert!(state.core_dumped);
    assert_eq!(state.error_message.as_deref(), Some("terminated"));
    assert_eq!(state.language.as_deref(), Some("en-US"));
}

#[test]
fn channel_construction_rejects_callback_registration_failure() {
    let session = crate::Session::new().unwrap();
    assert!(matches!(
        Channel::new(
            &session.sess,
            &session.operation_gate,
            std::ptr::null_mut(),
        ),
        Err(Error::Fatal(message)) if message.contains("register libssh channel callbacks")
    ));
}

#[test]
fn timeout_read_results_preserve_retry_and_eof_states() {
    assert!(matches!(
        classify_timeout_read_result(sys::SSH_AGAIN),
        Some(Err(Error::TryAgain))
    ));
    assert_eq!(classify_timeout_read_result(0), Some(Ok(0)));
    assert_eq!(classify_timeout_read_result(17), Some(Ok(17)));
    assert_eq!(classify_timeout_read_result(sys::SSH_ERROR), None);
    assert_eq!(classify_timeout_read_result(-99), None);
}

#[test]
fn nonblocking_read_results_distinguish_unavailable_data_from_eof() {
    assert!(matches!(
        classify_nonblocking_read_result(sys::SSH_AGAIN, false),
        Some(Err(Error::TryAgain))
    ));
    assert_eq!(
        classify_nonblocking_read_result(sys::SSH_EOF, false),
        Some(Ok(0))
    );
    assert!(matches!(
        classify_nonblocking_read_result(0, false),
        Some(Err(Error::TryAgain))
    ));
    assert_eq!(classify_nonblocking_read_result(0, true), Some(Ok(0)));
    assert_eq!(classify_nonblocking_read_result(17, false), Some(Ok(17)));
    assert_eq!(
        classify_nonblocking_read_result(sys::SSH_ERROR, false),
        None
    );
    assert_eq!(classify_nonblocking_read_result(-99, false), None);
}
