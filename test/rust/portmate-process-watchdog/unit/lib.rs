use super::*;

const VARIABLE_NAME: &str = "PORTMATE_TEST_PARENT_PID";

#[test]
fn parses_positive_decimal_process_ids() {
    assert_eq!(
        parse_parent_pid(VARIABLE_NAME, OsString::from("42")),
        Ok(42)
    );
    assert_eq!(
        parse_parent_pid(VARIABLE_NAME, OsString::from("00042")),
        Ok(42)
    );
}

#[test]
fn rejects_empty_or_non_decimal_process_ids() {
    for value in ["", " ", "+1", "-1", "1.0", "pid"] {
        assert!(parse_parent_pid(VARIABLE_NAME, OsString::from(value)).is_err());
    }
}

#[test]
fn rejects_zero_and_out_of_range_process_ids() {
    assert!(parse_parent_pid(VARIABLE_NAME, OsString::from("0")).is_err());
    assert!(parse_parent_pid(VARIABLE_NAME, OsString::from("4294967296")).is_err());
    assert!(parse_parent_pid(VARIABLE_NAME, OsString::from("18446744073709551616")).is_err());
    assert!(install_parent_watchdog(0).is_err());
}

#[test]
fn rejects_invalid_environment_variable_names() {
    assert!(validate_environment_variable_name("").is_err());
    assert!(validate_environment_variable_name("PORTMATE=PID").is_err());
    assert!(validate_environment_variable_name("PORTMATE\0PID").is_err());
}

#[cfg(unix)]
#[test]
fn rejects_process_ids_larger_than_unix_pid_t() {
    assert!(install_parent_watchdog(u32::MAX).is_err());
}

#[cfg(unix)]
#[test]
fn rejects_non_utf8_process_ids() {
    use std::os::unix::ffi::OsStringExt;

    assert!(parse_parent_pid(VARIABLE_NAME, OsString::from_vec(vec![0xff])).is_err());
}
