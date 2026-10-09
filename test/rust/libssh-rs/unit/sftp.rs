use super::*;

#[test]
fn sftp_v3_timestamp_accepts_its_full_wire_range() {
    assert_eq!(sftp_v3_timestamp(SystemTime::UNIX_EPOCH).unwrap(), 0);
    assert_eq!(
        sftp_v3_timestamp(SystemTime::UNIX_EPOCH + Duration::from_secs(u64::from(u32::MAX)))
            .unwrap(),
        u32::MAX
    );
}

#[test]
fn sftp_v3_timestamp_rejects_unrepresentable_times_without_panicking() {
    let before_epoch = SystemTime::UNIX_EPOCH
        .checked_sub(Duration::from_secs(1))
        .unwrap();
    assert!(sftp_v3_timestamp(before_epoch)
        .unwrap_err()
        .to_string()
        .contains("before Unix epoch"));

    let after_wire_range = SystemTime::UNIX_EPOCH + Duration::from_secs(u64::from(u32::MAX) + 1);
    assert!(sftp_v3_timestamp(after_wire_range)
        .unwrap_err()
        .to_string()
        .contains("32-bit seconds range"));
}

#[test]
fn remote_sftp_timestamp_overflow_is_treated_as_unavailable() {
    assert_eq!(
        system_time_from_sftp_timestamp(1, 500_000_000),
        Some(SystemTime::UNIX_EPOCH + Duration::from_millis(1_500))
    );
    assert_eq!(system_time_from_sftp_timestamp(u64::MAX, u32::MAX), None);
}

#[test]
fn sftp_seek_offsets_reject_underflow_and_overflow() {
    assert_eq!(checked_seek_target(10, -10).unwrap(), 0);
    assert_eq!(checked_seek_target(10, 5).unwrap(), 15);
    assert_eq!(
        checked_seek_target(0, -1).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
    assert_eq!(
        checked_seek_target(u64::MAX, 1).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
    assert_eq!(
        checked_seek_target(0, i64::MIN).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
}

#[test]
fn file_handles_keep_the_sftp_session_alive() {
    let session = crate::Session::new().unwrap();
    let sftp = Sftp::new(
        Arc::clone(&session.sess),
        Arc::clone(&session.operation_gate),
        std::ptr::null_mut(),
    );
    let owner = Arc::downgrade(&sftp.inner);
    let file = SftpFile {
        file_inner: std::ptr::null_mut(),
        sftp: Arc::clone(&sftp.inner),
    };

    drop(sftp);
    assert!(owner.upgrade().is_some());
    drop(file);
    assert!(owner.upgrade().is_none());
}
