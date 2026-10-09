use super::*;
use serde_json::json;

#[test]
fn structured_tftp_destination_normalizes_and_validates() {
    let destination: McpTransferDestination = serde_json::from_value(json!({
        "kind": "tftpboot",
        "deviceIp": "192.168.255.1",
        "address": "0x81800000",
        "fileName": "images/firmware.bin",
        "serverIp": "192.168.255.2",
        "bindHost": "0.0.0.0",
        "bindPort": 0,
        "timeoutSeconds": 3600
    }))
    .unwrap();
    assert_eq!(
        destination.normalize(&TransferProtocol::Tftp).unwrap(),
        "load:tftpboot?address=0x81800000&fileName=images%2Ffirmware.bin&deviceIp=192.168.255.1&serverIp=192.168.255.2&bindHost=0.0.0.0&bindPort=0&timeoutSeconds=3600"
    );

    let missing_device = serde_json::from_value::<McpTransferDestination>(json!({
        "kind": "tftpboot"
    }))
    .unwrap_err()
    .to_string();
    assert!(missing_device.contains("deviceIp"), "{missing_device}");
    let wrong_protocol: McpTransferDestination = serde_json::from_value(json!({
        "kind": "tftpboot",
        "deviceIp": "192.168.255.1"
    }))
    .unwrap();
    assert!(wrong_protocol.normalize(&TransferProtocol::Xmodem).is_err());
}

#[test]
fn source_classifier_reports_misplaced_tftp_options() {
    let arguments = json!({
        "uploadId": "8d23c9bd-4d7f-45dc-86a5-c702e5ac2bce",
        "deviceIp": "192.168.255.1"
    });
    let error = classify_mcp_start_transfer_source(arguments.as_object().unwrap()).unwrap_err();
    assert!(error.contains("structured `destination`"));
    assert!(error.contains("begin_content_upload"));
    assert!(!error.contains("another source mode"));
}

#[test]
fn nonlocal_transfer_endpoints_require_an_explicit_prefix() {
    assert!(is_nonlocal_transfer_endpoint("remote:/tmp/firmware.bin"));
    assert!(is_nonlocal_transfer_endpoint("ssh:/tmp/firmware.bin"));
    assert!(is_nonlocal_transfer_endpoint("load:loady"));
    assert!(!is_nonlocal_transfer_endpoint("/tmp/firmware.bin"));
    assert!(!is_nonlocal_transfer_endpoint("Remote:/tmp/firmware.bin"));
}
