use super::*;

#[derive(Default)]
struct CountingSystemRng {
    requests: usize,
    requested_bytes: usize,
}

impl CountingSystemRng {
    fn record(&mut self, bytes: usize) {
        self.requests += 1;
        self.requested_bytes += bytes;
    }
}

impl rsa::rand_core::TryRng for CountingSystemRng {
    type Error = getrandom::Error;

    fn try_next_u32(&mut self) -> Result<u32, Self::Error> {
        self.record(std::mem::size_of::<u32>());
        rsa::rand_core::TryRng::try_next_u32(&mut getrandom::SysRng)
    }

    fn try_next_u64(&mut self) -> Result<u64, Self::Error> {
        self.record(std::mem::size_of::<u64>());
        rsa::rand_core::TryRng::try_next_u64(&mut getrandom::SysRng)
    }

    fn try_fill_bytes(&mut self, destination: &mut [u8]) -> Result<(), Self::Error> {
        self.record(destination.len());
        rsa::rand_core::TryRng::try_fill_bytes(&mut getrandom::SysRng, destination)
    }
}

impl rsa::rand_core::TryCryptoRng for CountingSystemRng {}

#[test]
fn every_rsa_signature_requests_blinding_randomness() {
    let mut key_rng = rsa::rand_core::UnwrapErr(getrandom::SysRng);
    let private_key = rsa::RsaPrivateKey::new(&mut key_rng, 2048).unwrap();
    let private_key = ssh_key::private::RsaKeypair::try_from(private_key).unwrap();
    let signer = BlindedRsaSigner::new(private_key.into()).unwrap();
    let message = b"PortMate RSA authentication blinding regression";
    let mut rng = CountingSystemRng::default();

    for hash_alg in [
        Some(ssh_key::HashAlg::Sha512),
        Some(ssh_key::HashAlg::Sha256),
        None,
    ] {
        let previous_requests = rng.requests;
        let previous_bytes = rng.requested_bytes;
        let encoded = signer.sign_with_rng(hash_alg, message, &mut rng).unwrap();
        assert!(rng.requests > previous_requests);
        assert!(rng.requested_bytes > previous_bytes);

        let mut reader = encoded.as_slice();
        let signature =
            <ssh_key::Signature as russh::keys::ssh_encoding::Decode>::decode(&mut reader).unwrap();
        assert!(reader.is_empty());
        russh::keys::signature::Verifier::verify(
            signer.private_key.public_key(),
            message,
            &signature,
        )
        .unwrap();
    }
}
