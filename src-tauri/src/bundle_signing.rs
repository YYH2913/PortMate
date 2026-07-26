use super::*;

pub(super) const BUNDLE_SIGNING_KEY_REF: &str = "keychain:bundle-signing-ed25519-v1";
pub(super) const BUNDLE_SIGNING_KEY_PORTABLE_REF: &str = "stronghold:bundle-signing-ed25519-v1";
const BUNDLE_SIGNATURE_PAYLOAD_FORMAT: &str = "portmate-session-bundle-signature-v1";

pub(super) fn decode_bundle_signing_key(encoded: &str) -> Result<SigningKey, String> {
    let decoded = Zeroizing::new(
        BASE64_STANDARD
            .decode(encoded.trim())
            .map_err(|_| "stored bundle signing key is not valid Base64".to_string())?,
    );
    let seed = <&[u8; 32]>::try_from(decoded.as_slice())
        .map_err(|_| "stored bundle signing key has an invalid length".to_string())?;
    Ok(SigningKey::from_bytes(seed))
}

pub(super) fn load_or_create_bundle_signing_key() -> Result<SigningKey, String> {
    static KEY_INIT_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = KEY_INIT_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| "bundle signing key initialization lock is poisoned".to_string())?;

    match probe_secret_from_keyring(BUNDLE_SIGNING_KEY_REF) {
        SecretProbeResult::Present(encoded) => decode_bundle_signing_key(encoded.as_str()),
        SecretProbeResult::Unavailable(keyring_error) => {
            load_or_create_bundle_signing_key_in_portable_vault(keyring_error)
        }
        SecretProbeResult::Missing => {
            if let Some(signing_key) = existing_portable_bundle_signing_key()? {
                return Ok(signing_key);
            }
            let mut seed = Zeroizing::new([0_u8; 32]);
            getrandom::fill(seed.as_mut())
                .map_err(|error| format!("failed to generate bundle signing key: {error}"))?;
            let encoded = Zeroizing::new(BASE64_STANDARD.encode(seed.as_ref()));
            if let Err(keyring_error) =
                write_secret_to_keyring(BUNDLE_SIGNING_KEY_REF, encoded.as_str())
            {
                return persist_bundle_signing_key_in_portable_vault(
                    encoded.as_str(),
                    keyring_error,
                );
            }
            let persisted =
                Zeroizing::new(read_secret_from_keyring(BUNDLE_SIGNING_KEY_REF).map_err(
                    |error| format!("failed to verify persisted bundle signing key: {error}"),
                )?);
            if persisted.as_str() != encoded.as_str() {
                return Err(
                    "persisted bundle signing key did not pass read-back verification".to_string(),
                );
            }
            decode_bundle_signing_key(persisted.as_str())
        }
    }
}

fn existing_portable_bundle_signing_key() -> Result<Option<SigningKey>, String> {
    let status = match portable_vault_status_inner() {
        Ok(status) => status,
        Err(_) => return Ok(None),
    };
    if status.exists && !status.unlocked {
        return Err(
            "portable vault is locked; unlock it before creating or loading the bundle signing identity"
                .to_string(),
        );
    }
    if !status.unlocked {
        return Ok(None);
    }
    match probe_secret_from_portable_vault(BUNDLE_SIGNING_KEY_PORTABLE_REF) {
        SecretProbeResult::Present(encoded) => {
            decode_bundle_signing_key(encoded.as_str()).map(Some)
        }
        SecretProbeResult::Missing => Ok(None),
        SecretProbeResult::Unavailable(error) => Err(format!(
            "failed to inspect portable bundle signing identity: {error}"
        )),
    }
}

fn load_or_create_bundle_signing_key_in_portable_vault(
    keyring_error: String,
) -> Result<SigningKey, String> {
    let secret_ref = BUNDLE_SIGNING_KEY_PORTABLE_REF;
    match probe_secret_from_portable_vault(secret_ref) {
        SecretProbeResult::Present(encoded) => decode_bundle_signing_key(encoded.as_str()),
        SecretProbeResult::Missing => {
            let mut seed = Zeroizing::new([0_u8; 32]);
            getrandom::fill(seed.as_mut())
                .map_err(|error| format!("failed to generate bundle signing key: {error}"))?;
            let encoded = Zeroizing::new(BASE64_STANDARD.encode(seed.as_ref()));
            persist_bundle_signing_key_in_portable_vault(encoded.as_str(), keyring_error)
        }
        SecretProbeResult::Unavailable(portable_error) => Err(format!(
            "bundle signing key is unavailable: system keyring failed ({keyring_error}); portable vault failed ({portable_error})"
        )),
    }
}

fn persist_bundle_signing_key_in_portable_vault(
    encoded: &str,
    keyring_error: String,
) -> Result<SigningKey, String> {
    let secret_ref = BUNDLE_SIGNING_KEY_PORTABLE_REF;
    write_secret_to_portable_vault(secret_ref, encoded).map_err(|portable_error| {
        format!(
            "failed to persist bundle signing key: system keyring failed ({keyring_error}); portable vault failed ({portable_error})"
        )
    })?;
    let persisted = Zeroizing::new(
        read_secret_from_portable_vault(secret_ref).map_err(|portable_error| {
            format!(
                "failed to verify persisted bundle signing key: system keyring failed ({keyring_error}); portable vault failed ({portable_error})"
            )
        })?,
    );
    if persisted.as_str() != encoded {
        return Err("persisted bundle signing key did not pass read-back verification".to_string());
    }
    decode_bundle_signing_key(persisted.as_str())
}

#[derive(Debug)]
pub(super) struct SignedFinalizedArchive {
    pub(super) checksum_path: PathBuf,
    pub(super) signature_path: PathBuf,
    pub(super) sha256: String,
    pub(super) signing_public_key: String,
    pub(super) size: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BundleDetachedSignature {
    format: &'static str,
    version: u32,
    algorithm: &'static str,
    payload_format: &'static str,
    archive_file: String,
    archive_size: u64,
    archive_sha256: String,
    created_at: String,
    public_key_base64: String,
    key_id: String,
    signed_payload_base64: String,
    signature_base64: String,
}

pub(super) fn bundle_signature_payload(
    archive_name: &str,
    archive_sha256: &str,
    archive_size: u64,
    created_at: &str,
) -> Vec<u8> {
    [
        BUNDLE_SIGNATURE_PAYLOAD_FORMAT.to_string(),
        archive_name.to_string(),
        archive_sha256.to_string(),
        archive_size.to_string(),
        created_at.to_string(),
    ]
    .join("\0")
    .into_bytes()
}

pub(super) fn finalize_signed_bundle_archive(
    temp_path: &Path,
    final_path: &Path,
    label: &str,
    signing_key: &SigningKey,
    created_at: &str,
) -> Result<SignedFinalizedArchive, String> {
    let sha256 = sha256_file(temp_path)?;
    let size = fs::metadata(temp_path)
        .map_err(|error| format!("failed to read {label} metadata: {error}"))?
        .len();
    let archive_name = final_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("invalid {label} file name"))?;
    let signed_payload = bundle_signature_payload(archive_name, &sha256, size, created_at);
    let signature = signing_key.sign(&signed_payload);
    let public_key = signing_key.verifying_key().to_bytes();
    let public_key_base64 = BASE64_STANDARD.encode(public_key);
    let signature_document = BundleDetachedSignature {
        format: "portmate-detached-signature",
        version: 1,
        algorithm: "Ed25519",
        payload_format: BUNDLE_SIGNATURE_PAYLOAD_FORMAT,
        archive_file: archive_name.to_string(),
        archive_size: size,
        archive_sha256: sha256.clone(),
        created_at: created_at.to_string(),
        public_key_base64: public_key_base64.clone(),
        key_id: format!("sha256:{}", sha256_hex(&public_key)),
        signed_payload_base64: BASE64_STANDARD.encode(&signed_payload),
        signature_base64: BASE64_STANDARD.encode(signature.to_bytes()),
    };
    let mut signature_bytes = serde_json::to_vec_pretty(&signature_document)
        .map_err(|error| format!("failed to serialize {label} signature: {error}"))?;
    signature_bytes.push(b'\n');

    let checksum_path = path_with_appended_suffix(final_path, ".sha256")?;
    let checksum_temp_path = path_with_appended_suffix(final_path, ".sha256.part")?;
    let signature_path = path_with_appended_suffix(final_path, ".sig.json")?;
    let signature_temp_path = path_with_appended_suffix(final_path, ".sig.json.part")?;
    for artifact in [
        final_path,
        checksum_path.as_path(),
        checksum_temp_path.as_path(),
        signature_path.as_path(),
        signature_temp_path.as_path(),
    ] {
        if fs::symlink_metadata(artifact).is_ok() {
            let _ = fs::remove_file(temp_path);
            return Err(format!(
                "refusing to overwrite existing {label} artifact {}",
                artifact.display()
            ));
        }
    }
    let cleanup = || {
        let _ = fs::remove_file(temp_path);
        let _ = fs::remove_file(final_path);
        let _ = fs::remove_file(&checksum_temp_path);
        let _ = fs::remove_file(&checksum_path);
        let _ = fs::remove_file(&signature_temp_path);
        let _ = fs::remove_file(&signature_path);
    };

    if let Err(error) = write_new_synced_file(
        &checksum_temp_path,
        format!("{sha256}  {archive_name}\n").as_bytes(),
        &format!("{label} checksum"),
    ) {
        cleanup();
        return Err(error);
    }
    if let Err(error) = write_new_synced_file(
        &signature_temp_path,
        &signature_bytes,
        &format!("{label} signature"),
    ) {
        cleanup();
        return Err(error);
    }
    if let Err(error) = fs::rename(temp_path, final_path) {
        cleanup();
        return Err(format!(
            "failed to finalize {label} {}: {error}",
            final_path.display()
        ));
    }
    if let Err(error) = fs::rename(&checksum_temp_path, &checksum_path) {
        cleanup();
        return Err(format!(
            "failed to finalize {label} checksum {}: {error}",
            checksum_path.display()
        ));
    }
    if let Err(error) = fs::rename(&signature_temp_path, &signature_path) {
        cleanup();
        return Err(format!(
            "failed to finalize {label} signature {}: {error}",
            signature_path.display()
        ));
    }

    Ok(SignedFinalizedArchive {
        checksum_path,
        signature_path,
        sha256,
        signing_public_key: public_key_base64,
        size,
    })
}
