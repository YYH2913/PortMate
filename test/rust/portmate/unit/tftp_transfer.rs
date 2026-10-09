use super::*;

#[test]
fn default_port_binding_has_an_unprivileged_fallback() {
    tauri::async_runtime::block_on(async {
        let blocker = UdpSocket::bind((Ipv4Addr::LOCALHOST, DEFAULT_TFTP_PORT))
            .await
            .ok();
        let binding = bind_tftp_socket(Ipv4Addr::LOCALHOST, DEFAULT_TFTP_PORT)
            .await
            .expect("TFTP should fall back when port 69 cannot be used");
        if blocker.is_some() {
            assert_ne!(binding.port, DEFAULT_TFTP_PORT);
        }
        drop(binding);
        drop(blocker);
    });
}

#[test]
fn parses_binary_rrq_and_negotiates_bounded_options() {
    let request = parse_tftp_read_request(
        b"\x00\x01firmware.bin\x00octet\x00blksize\x0065464\x00tsize\x000\x00timeout\x009\x00",
    )
    .unwrap();
    assert_eq!(request.file_name, "firmware.bin");
    let negotiation = negotiate_tftp_options(&request.options, 4_096).unwrap();
    assert_eq!(negotiation.block_size, TFTP_MAX_BLOCK_SIZE);
    assert_eq!(negotiation.retry_timeout, Duration::from_secs(9));
    let option_ack = negotiation.option_ack.unwrap();
    assert!(option_ack
        .windows(b"blksize\x001468\x00".len())
        .any(|window| window == b"blksize\x001468\x00"));
    assert!(option_ack
        .windows(b"tsize\x004096\x00".len())
        .any(|window| window == b"tsize\x004096\x00"));
}

#[test]
fn rejects_command_injection_in_rrq_file_names() {
    let error = parse_tftp_read_request(b"\x00\x01fw.bin;saveenv\x00octet\x00")
        .expect_err("unsafe filenames must be rejected");
    assert_eq!(error.code, 1);
    assert!(parse_tftp_read_request(b"\x00\x01../fw.bin\x00octet\x00").is_err());
    assert!(parse_tftp_read_request(b"\x00\x01fw.bin\x00netascii\x00").is_err());
}

#[test]
fn accepts_fixed_length_rrq_padding_after_mode() {
    let mut packet = b"\x00\x01firmware.bin\x00octet\x00".to_vec();
    packet.resize(516, 0xa5);
    let request = parse_tftp_read_request(&packet).expect("padded RRQ should remain compatible");
    assert_eq!(request.file_name, "firmware.bin");
    assert!(request.options.is_empty());
}

#[test]
fn keeps_complete_options_before_fixed_length_rrq_padding() {
    let mut packet = b"\x00\x01firmware.bin\x00octet\x00blksize\x001024\x00".to_vec();
    packet.resize(516, 0xa5);
    let request = parse_tftp_read_request(&packet).expect("valid options should survive padding");
    assert_eq!(
        request.options,
        vec![("blksize".to_string(), "1024".to_string())]
    );
}

#[test]
fn malformed_rrq_uses_illegal_operation_error_code() {
    let error = parse_tftp_read_request(b"\x00\x02firmware.bin\x00octet\x00")
        .expect_err("WRQ is not a supported RRQ");
    assert_eq!(error.code, 4);
}
