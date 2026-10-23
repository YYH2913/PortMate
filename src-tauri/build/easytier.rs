use sha2::{Digest, Sha256};
use std::{collections::HashMap, env, fs, io::Read, path::PathBuf};

pub fn embed() {
    println!("cargo:rerun-if-changed=easytier-manifest.json");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read("easytier-manifest.json").unwrap()).unwrap();
    let version = manifest["version"].as_str().unwrap();
    println!("cargo:rustc-env=PORTMATE_EASYTIER_VERSION={version}");
    if env::var_os("CARGO_FEATURE_BUNDLED_EASYTIER").is_none() {
        return;
    }
    let target = env::var("TARGET").unwrap();
    let release_key = manifest["targets"][&target]
        .as_str()
        .unwrap_or_else(|| panic!("Bundled EasyTier does not support {target}"));
    let release = &manifest["releases"][release_key];
    let archive = PathBuf::from("binaries/easytier").join(release["archive"].as_str().unwrap());
    println!("cargo:rerun-if-changed={}", archive.display());
    let bytes = fs::read(&archive).unwrap_or_else(|_| {
        panic!("Bundled EasyTier is missing. Run `npm run easytier:prepare -- --target {target}` before building PortMate.");
    });
    let expected = release["sha256"].as_str().unwrap();
    assert_eq!(
        digest(&bytes),
        expected,
        "Bundled EasyTier archive SHA-256 mismatch"
    );
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("easytier");
    fs::create_dir_all(&out).unwrap();
    let names: Vec<&str> = release["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|name| name.as_str().unwrap())
        .collect();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut found = HashMap::new();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).unwrap();
        let path = entry.enclosed_name().expect("Unsafe upstream archive path");
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !names.contains(&name) || entry.is_dir() {
            continue;
        }
        assert!(
            !entry
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000),
            "Symlink in upstream archive"
        );
        assert!(
            entry.size() <= 128 * 1024 * 1024,
            "Oversized upstream component"
        );
        let mut content = Vec::new();
        entry.read_to_end(&mut content).unwrap();
        let destination = out.join(name);
        fs::write(&destination, &content).unwrap();
        assert!(
            found
                .insert(name.to_string(), (destination, digest(&content)))
                .is_none(),
            "Duplicate upstream component"
        );
    }
    let mut generated = format!("pub(super) const PAYLOAD_ID: &str = {expected:?};\npub(super) static FILES: &[EmbeddedFile] = &[\n");
    for name in names {
        let (path, sha256) = found
            .get(name)
            .expect("Missing required upstream component");
        // JSON string quoting also produces a valid Rust path literal on Windows.
        let literal = serde_json::to_string(path.to_str().unwrap()).unwrap();
        generated += &format!("EmbeddedFile {{ name: {name:?}, sha256: {sha256:?}, bytes: include_bytes!({literal}) }},\n");
    }
    generated += "];\n";
    fs::write(out.join("payload.rs"), generated).unwrap();
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
