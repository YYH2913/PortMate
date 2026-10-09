use super::*;

#[test]
fn transfer_file_identity_rejects_same_size_path_replacement() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.bin");
    fs::write(&source, b"first").unwrap();
    let before = local_transfer_entry(&source, "source").unwrap().unwrap();
    let original = fs::File::open(&source).unwrap();
    assert!(same_local_file_identity(&before, &original).unwrap());

    fs::rename(&source, root.path().join("original.bin")).unwrap();
    fs::write(&source, b"other").unwrap();
    let replacement = fs::File::open(&source).unwrap();
    assert!(!same_local_file_identity(&before, &replacement).unwrap());
    assert_eq!(before.len(), replacement.metadata().unwrap().len());
}

#[test]
fn transfer_file_identity_rejects_hard_linked_source() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source.bin");
    fs::write(&source, b"protected").unwrap();
    fs::hard_link(&source, root.path().join("alias.bin")).unwrap();

    let error = open_local_transfer_source(&source, "source").err().unwrap();
    assert!(error.contains("硬链接"), "{error}");
    assert_eq!(fs::read(&source).unwrap(), b"protected");
}
