use super::terminal_key_sequence_for_protocol;

#[test]
fn terminal_key_sequences_preserve_literal_case_and_symbols() {
    for key in ["A", "G", "a", "_", "-", "É", "λ", "中", "🦀"] {
        for is_telnet in [false, true] {
            assert_eq!(
                terminal_key_sequence_for_protocol(key, is_telnet).unwrap(),
                key
            );
        }
    }
    assert_eq!(
        terminal_key_sequence_for_protocol("  A  ", false).unwrap(),
        "A"
    );
}

#[test]
fn terminal_key_sequences_normalize_names_but_preserve_control_symbols() {
    for (key, expected) in [
        ("EnTeR", "\r"),
        ("Arrow_Up", "\x1b[A"),
        ("PAGE_DOWN", "\x1b[6~"),
        ("CTRL_C", "\x03"),
        ("Ctrl+A", "\x01"),
        ("Ctrl+_", "\x1f"),
        ("CTRL-_", "\x1f"),
        ("ctrl__", "\x1f"),
        ("Ctrl+Space", "\0"),
        ("Ctrl+[", "\x1b"),
        ("Ctrl+\\", "\x1c"),
        ("Ctrl+]", "\x1d"),
        ("Ctrl+^", "\x1e"),
    ] {
        assert_eq!(
            terminal_key_sequence_for_protocol(key, false).unwrap(),
            expected,
            "{key}"
        );
    }
    assert_eq!(
        terminal_key_sequence_for_protocol("ENTER", true).unwrap(),
        "\r\n"
    );
}

#[test]
fn terminal_key_sequences_reject_multiple_keys_and_escape_payloads() {
    for key in [
        "",
        "  ",
        "AB",
        "echo hello",
        "\x1b[31m",
        "Ctrl+Ctrl+A",
        "Ctrl+1",
        "F13",
    ] {
        assert!(
            terminal_key_sequence_for_protocol(key, false).is_err(),
            "{key:?}"
        );
    }
}
