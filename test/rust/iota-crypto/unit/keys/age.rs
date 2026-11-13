// Copyright 2020-2023 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0
use super::*;

const K64: usize = 64 * 1024;
const TEST_LENS: [usize; 12] = [
    0,
    1,
    K64 - 16,
    K64 - 1,
    K64,
    K64 + 1,
    K64 + 16,
    2 * K64 - 16,
    2 * K64 - 1,
    2 * K64,
    2 * K64 + 1,
    2 * K64 + 16,
];

#[test]
fn test_payload_len() {
    for len in TEST_LENS {
        assert_eq!(Some(len), dec_payload_len(enc_payload_len(len)));
    }
    assert_eq!(None, dec_payload_len(0));
    assert_eq!(None, dec_payload_len(15));
    assert_eq!(None, dec_payload_len(16));
    assert_eq!(None, dec_payload_len(31));
    assert_eq!(Some(0), dec_payload_len(32));

    assert_eq!(Some(K64), dec_payload_len(16 + K64 + 16));
    assert_eq!(None, dec_payload_len(16 + K64 + 16 + 1));
    assert_eq!(None, dec_payload_len(16 + K64 + 16 + 16));
    assert_eq!(Some(K64 + 1), dec_payload_len(16 + K64 + 16 + 17));

    assert_eq!(Some(2 * K64), dec_payload_len(16 + 2 * (K64 + 16)));
    assert_eq!(None, dec_payload_len(16 + 2 * (K64 + 16) + 1));
    assert_eq!(None, dec_payload_len(16 + 2 * (K64 + 16) + 16));
    assert_eq!(Some(2 * K64 + 1), dec_payload_len(16 + 2 * (K64 + 16) + 17));
}

#[test]
fn test_nonce() {
    let mut nonce = [0_u8; 12];
    for i in 1_usize..258_usize {
        inc_nonce(&mut nonce);
        assert_eq!(i.to_be_bytes(), &nonce[3..11]);
    }
}

fn run_header(
    password: &[u8],
    salt: &[u8; 16],
    file_key: &[u8; 16],
    work_factor: u8,
    max_work_factor: u8,
) -> Result<(), DecError> {
    let mut header = [0_u8; SCRYPT_MAX_HEADER_LEN];
    let h = enc_header(
        password,
        salt,
        file_key,
        work_factor,
        &mut header[..header_len(work_factor)],
    );
    let mut dec_file_key = [0_u8; 16];
    let r = dec_header(password, max_work_factor, &header, &mut dec_file_key);
    if r.is_ok() {
        assert_eq!(h, r.unwrap());
        assert_eq!(file_key, &dec_file_key);
    }
    r.map(|_| ())
}

#[test]
fn test_header() {
    let password = [0xaa_u8; 1025];
    let pwd_lens = [0, 1, 33, 65, 1025];
    let bits = [[0x00_u8; 16], [0xaa_u8; 16], [0xff_u8; 16]];
    let work_factor = 1_u8;

    // run_header(&password[..0], &bits[2], &bits[2], 10);
    for pwd_len in pwd_lens {
        for salt in bits {
            for file_key in bits {
                let r = run_header(
                    &password[..pwd_len],
                    &salt,
                    &file_key,
                    work_factor,
                    work_factor,
                );
                assert!(r.is_ok());
            }
        }
    }
}

#[cfg(feature = "std")]
fn enc_crate(plaintext: &[u8]) -> Vec<u8> {
    let password = "password".as_bytes();
    let work_factor = 1_u8.try_into().unwrap();
    let salt = [0x11_u8; 16];
    let file_key = [0x22_u8; 16];
    let nonce = [0x33_u8; 16];
    enc_vec(password, &salt, &file_key, work_factor, &nonce, plaintext)
}

#[cfg(feature = "std")]
fn enc_rage(plaintext: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let password = "password".to_owned().into();
    let mut age = Vec::new();
    let mut writer = age::Encryptor::with_user_passphrase(password)
        .wrap_output(&mut age)
        .unwrap();
    writer.write_all(plaintext).unwrap();
    writer.finish().unwrap();
    age
}

#[cfg(feature = "std")]
fn dec_crate(age: &[u8], max_work_factor: u8) -> Option<Vec<u8>> {
    decrypt_vec("password".as_bytes(), max_work_factor, age).ok()
}

#[cfg(feature = "std")]
fn dec_rage(age: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;
    let pass = "password".to_owned().into();
    let mut reader = match age::Decryptor::new(age).unwrap() {
        age::Decryptor::Recipients(_) => panic!("internal error"),
        age::Decryptor::Passphrase(d) => d.decrypt(&pass, Some(14)).unwrap(),
    };
    let mut decrypted = Vec::new();
    reader.read_to_end(&mut decrypted).ok()?;
    Some(decrypted)
}

#[cfg(feature = "std")]
#[test]
fn test_crate_rage() {
    for text_len in TEST_LENS {
        let mut plaintext = Vec::new();
        plaintext.resize(text_len, 0xaa_u8);
        let decrypted = dec_rage(&enc_crate(&plaintext));
        assert_eq!(Some(plaintext), decrypted);
    }
}

#[cfg(feature = "std")]
#[test]
fn test_crate_crate() {
    for text_len in TEST_LENS {
        let mut plaintext = Vec::new();
        plaintext.resize(text_len, 0xaa_u8);
        let decrypted = dec_crate(&enc_crate(&plaintext), 1_u8);
        assert_eq!(Some(plaintext), decrypted);
    }
}

#[cfg(feature = "std")]
#[test]
fn test_rage_crate() {
    for text_len in [0, 1, 64 * 1024 + 1] {
        let mut plaintext = Vec::new();
        plaintext.resize(text_len, 0xaa_u8);
        let max_work_factor = 22_u8;
        // dec_crate can fail if max_work_factor is too small
        let decrypted = dec_crate(&enc_rage(&plaintext), max_work_factor);
        assert_eq!(Some(plaintext), decrypted);
    }
}

#[test]
fn test_err() {
    const TEXT_LEN: usize = 5;
    const TEXT_LEN1: usize = TEXT_LEN - 1;
    let plain = [0xdd_u8; TEXT_LEN];
    let mut decrypted = [0_u8; TEXT_LEN];
    const WORK_FACTOR: WorkFactor = WorkFactor::new(1_u8);
    const AGE_LEN: usize = enc_len(WORK_FACTOR, TEXT_LEN);
    const AGE_LEN1: usize = AGE_LEN - 1;
    let mut age = [0_u8; AGE_LEN];

    let salt = [0x11_u8; 16];
    let file_key = [0x22_u8; 16];
    let nonce = [0x33_u8; 16];

    let err = enc(
        b"password",
        &salt,
        &file_key,
        WORK_FACTOR,
        &nonce,
        &plain,
        &mut age[..AGE_LEN1],
    )
    .err()
    .unwrap();
    assert!(
        matches!(
            err,
            EncError::BufferTooSmall {
                expected: AGE_LEN,
                provided: AGE_LEN1,
            }
        ),
        "wrong expected error result"
    );

    assert_eq!(
        AGE_LEN,
        enc(
            b"password",
            &salt,
            &file_key,
            WORK_FACTOR,
            &nonce,
            &plain,
            &mut age
        )
        .unwrap()
    );

    let err = decrypt(b"password", 0_u8, &age, &mut decrypted)
        .err()
        .unwrap();
    assert!(
        matches!(
            err,
            DecError::WorkFactorTooBig {
                required: 1_u8,
                allowed: 0_u8,
            }
        ),
        "wrong expected error result"
    );

    let err = decrypt(b"password", 1_u8, &age, &mut decrypted[..TEXT_LEN1])
        .err()
        .unwrap();
    assert!(
        matches!(
            err,
            DecError::BufferTooSmall {
                expected: TEXT_LEN,
                provided: TEXT_LEN1,
            }
        ),
        "wrong expected error result"
    );

    assert_eq!(
        TEXT_LEN,
        decrypt(b"password", 1_u8, &age, &mut decrypted).unwrap()
    );
}

#[test]
fn test_fuzz() {
    let plain = [0xdd_u8; 5];
    let mut decrypted = [0_u8; 5];
    const WORK_FACTOR: WorkFactor = WorkFactor::new(1_u8);
    let mut age = [0_u8; enc_len(WORK_FACTOR, 5)];

    let salt = [0x11_u8; 16];
    let file_key = [0x22_u8; 16];
    let nonce = [0x33_u8; 16];
    assert!(enc(
        b"password",
        &salt,
        &file_key,
        WORK_FACTOR,
        &nonce,
        &plain,
        &mut age
    )
    .is_ok());

    assert!(decrypt(b"password", 1_u8, &age, &mut decrypted).is_ok());
    assert_eq!(&plain, &decrypted);
    assert!(decrypt(b"password", 0_u8, &age, &mut decrypted).is_err());

    assert!(decrypt(b"passphrase", 1_u8, &age, &mut decrypted).is_err());
    for i in 0..age.len() {
        age[i] ^= 1;
        assert!(decrypt(b"password", 2_u8, &age, &mut decrypted).is_err());
        age[i] ^= 1;
    }
}
