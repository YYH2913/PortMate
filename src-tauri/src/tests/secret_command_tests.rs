use crate::secret_commands::{prepare_stored_secret, MAX_STORED_SECRET_BYTES};

#[test]
fn stored_secret_normalization_enforces_content_boundaries() {
    assert_eq!(
        prepare_stored_secret("private-key\r\n".to_string()).unwrap(),
        "private-key"
    );
    assert_eq!(
        prepare_stored_secret("x".repeat(MAX_STORED_SECRET_BYTES))
            .unwrap()
            .len(),
        MAX_STORED_SECRET_BYTES
    );

    assert!(prepare_stored_secret(" \r\n".to_string())
        .unwrap_err()
        .contains("不能为空"));
    assert!(prepare_stored_secret("secret\0suffix".to_string())
        .unwrap_err()
        .contains("NUL"));
    assert!(
        prepare_stored_secret("x".repeat(MAX_STORED_SECRET_BYTES + 1))
            .unwrap_err()
            .contains("不能超过")
    );
}
