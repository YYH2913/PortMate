use super::*;

#[test]
fn key_derivation_is_deterministic_and_salt_bound() {
    let first = derive_key(b"correct horse", &[1; SALT_LENGTH]).unwrap();
    // Captured with Argon2 0.5.3; keep existing vault keys stable across upgrades.
    assert_eq!(
        first,
        [
            173, 183, 43, 103, 77, 251, 42, 50, 40, 195, 119, 15, 43, 166, 182, 186, 199, 239, 137,
            249, 106, 172, 183, 147, 94, 38, 0, 81, 162, 227, 87, 213
        ]
    );
    let repeated = derive_key(b"correct horse", &[1; SALT_LENGTH]).unwrap();
    let other_salt = derive_key(b"correct horse", &[2; SALT_LENGTH]).unwrap();
    assert_eq!(first, repeated);
    assert_ne!(first, other_salt);
    assert!(derive_key(b"correct horse", &[1; SALT_LENGTH - 1]).is_err());
}
