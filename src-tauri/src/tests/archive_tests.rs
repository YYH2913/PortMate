use super::*;

include!("archive_log_shard_tests.rs");
include!("archive_bundle_tests.rs");
include!("archive_safety_tests.rs");

fn read_test_bundle_entries(path: &Path) -> HashMap<String, Vec<u8>> {
    let file = fs::File::open(path).unwrap();
    let decoder = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let mut result = HashMap::new();
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let path = entry.path().unwrap().to_string_lossy().to_string();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        result.insert(path, bytes);
    }
    result
}
