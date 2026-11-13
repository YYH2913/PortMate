use crypto::signatures::secp256k1_ecdsa::SecretKey;
use rand::{CryptoRng, RngCore};

struct CallerRng {
    value: u8,
    supplied: usize,
}

impl RngCore for CallerRng {
    fn next_u32(&mut self) -> u32 {
        let mut bytes = [0; 4];
        self.fill_bytes(&mut bytes);
        u32::from_le_bytes(bytes)
    }

    fn next_u64(&mut self) -> u64 {
        let mut bytes = [0; 8];
        self.fill_bytes(&mut bytes);
        u64::from_le_bytes(bytes)
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        assert!(self.value != 0, "test entropy unavailable");
        dest.fill(self.value);
        self.supplied += dest.len();
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}

impl CryptoRng for CallerRng {}

#[test]
fn uses_caller_entropy_and_preserves_signing_and_recovery() {
    let mut first = CallerRng {
        value: 1,
        supplied: 0,
    };
    let key = SecretKey::generate_with(&mut first);
    assert!(first.supplied >= 32);
    let mut second = CallerRng {
        value: 2,
        supplied: 0,
    };
    let different = SecretKey::generate_with(&mut second);
    assert!(second.supplied >= 32);
    assert_ne!(key.public_key(), different.public_key());
    let mut repeated = CallerRng {
        value: 1,
        supplied: 0,
    };
    assert_eq!(
        key.to_bytes(),
        SecretKey::generate_with(&mut repeated).to_bytes()
    );

    let message = b"PortMate Stronghold crypto dependency regression";
    let signature = key.try_sign_sha256(message).unwrap();
    assert!(key.public_key().verify_sha256(signature.as_ref(), message));
    assert_eq!(key.public_key(), signature.recover_sha256(message).unwrap());
    assert!(!key
        .public_key()
        .verify_sha256(signature.as_ref(), b"changed"));
}

#[test]
#[should_panic(expected = "test entropy unavailable")]
fn never_replaces_failed_caller_entropy_with_a_fallback() {
    SecretKey::generate_with(&mut CallerRng {
        value: 0,
        supplied: 0,
    });
}
