use bytes::{BufMut, BytesMut};

use super::*;

fn decode_status(code: u32) -> Status {
    let mut encoded = BytesMut::with_capacity(16);
    encoded.put_u32(7);
    encoded.put_u32(code);
    encoded.put_u32(0);
    encoded.put_u32(0);
    let mut encoded = encoded.freeze();
    crate::de::from_bytes(&mut encoded).unwrap()
}

#[test]
fn decodes_standard_extended_status_codes() {
    assert_eq!(decode_status(10).status_code, StatusCode::NoSuchPath);
    assert_eq!(decode_status(11).status_code, StatusCode::FileAlreadyExists);
    assert_eq!(decode_status(18).status_code, StatusCode::DirNotEmpty);
}

#[test]
fn decodes_unknown_status_codes_without_dropping_the_packet() {
    assert_eq!(decode_status(99).status_code, StatusCode::Unknown);
}
