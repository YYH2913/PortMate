use super::*;

#[test]
fn sftp_directory_entry_names_cannot_escape_the_listed_directory() {
    for name in [
        "",
        ".",
        "..",
        "../escape",
        "child/file",
        "child\\file",
        "line\nfeed",
        "nul\0name",
    ] {
        assert!(
            validate_sftp_directory_entry_name(name).is_err(),
            "{name:?}"
        );
    }
    assert!(validate_sftp_directory_entry_name("normal file.txt").is_ok());
    assert!(validate_sftp_directory_entry_name(
        &"a".repeat(MAX_SFTP_DIRECTORY_ENTRY_NAME_BYTES + 1)
    )
    .is_err());
}
