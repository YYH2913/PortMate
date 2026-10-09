use super::*;

#[test]
fn native_smoke_configuration_is_explicit_bounded_and_isolated() {
    assert!(parse_native_smoke_config(None, None).unwrap().is_none());
    assert!(parse_native_smoke_config(
        Some(std::env::temp_dir().join("portmate-smoke").into_os_string()),
        None,
    )
    .unwrap_err()
    .contains("must be set together"));

    let data_dir = std::env::temp_dir().join("portmate-smoke");
    let config =
        parse_native_smoke_config(Some(data_dir.clone().into_os_string()), Some("2500".into()))
            .unwrap()
            .expect("complete smoke configuration should be enabled");
    assert_eq!(config.data_dir, data_dir);
    assert_eq!(config.data_root, data_dir.parent().unwrap());
    assert_eq!(config.exit_after, Duration::from_millis(2_500));

    for delay in ["999", "60001", "not-a-number"] {
        assert!(parse_native_smoke_config(
            Some(data_dir.clone().into_os_string()),
            Some(delay.into()),
        )
        .is_err());
    }
}
