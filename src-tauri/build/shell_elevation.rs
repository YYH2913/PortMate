use sha2::{Digest, Sha256};
use std::{env, fs, io::Read, path::PathBuf};

pub fn embed() {
    println!("cargo:rerun-if-changed=shell-elevation-manifest.json");
    if env::var_os("CARGO_FEATURE_BUNDLED_SHELL_ELEVATION").is_none()
        || env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
    {
        return;
    }
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read("shell-elevation-manifest.json").unwrap()).unwrap();
    let target = env::var("TARGET").unwrap();
    let entry = manifest["targets"][&target]
        .as_str()
        .expect("Unsupported Windows elevation target");
    let archive = PathBuf::from("binaries/shell-elevation/gsudo.portable.zip");
    println!("cargo:rerun-if-changed={}", archive.display());
    let bytes = fs::read(archive)
        .expect("Run npm run shell-elevation:prepare for this Windows target before Cargo builds");
    let digest = |bytes: &[u8]| {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    assert_eq!(
        digest(&bytes),
        manifest["sha256"].as_str().unwrap(),
        "Elevation archive SHA-256 mismatch"
    );
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut binary = zip.by_name(entry).expect("Missing elevation executable");
    assert!(binary.size() <= 8 * 1024 * 1024);
    assert!(!binary
        .unix_mode()
        .is_some_and(|mode| mode & 0o170000 == 0o120000));
    let mut content = Vec::new();
    binary.read_to_end(&mut content).unwrap();
    assert!(content.starts_with(b"MZ"));
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let executable = out.join("portmate-gsudo.exe");
    fs::write(&executable, &content).unwrap();
    let path_literal = serde_json::to_string(executable.to_str().unwrap()).unwrap();
    let generated = format!("const HELPER_BYTES: &[u8] = include_bytes!({path_literal});\nconst HELPER_SHA256: &str = {:?};\n", digest(&content));
    fs::write(out.join("shell_elevation_payload.rs"), generated).unwrap();
}
