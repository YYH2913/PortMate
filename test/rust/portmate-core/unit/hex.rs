use super::*;
use sha2::{Digest, Sha256};

#[test]
fn digest_encoding_preserves_existing_checksums() {
    assert_eq!(encode_hex(&[]), "");
    assert_eq!(encode_hex(&[0, 1, 15, 16, 255]), "00010f10ff");
    assert_eq!(
        encode_hex(&Sha256::digest(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
