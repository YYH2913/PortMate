use super::*;

#[test]
fn read_dir_skips_many_dot_entries_without_recursion() {
    let mut entries = VecDeque::new();
    for index in 0..50_000 {
        let name = if index % 2 == 0 { "." } else { ".." };
        entries.push_back((name.to_string(), Metadata::default()));
    }
    entries.push_back(("payload.bin".to_string(), Metadata::default()));
    let mut directory = ReadDir { entries };

    assert_eq!(directory.next().unwrap().file_name(), "payload.bin");
    assert!(directory.next().is_none());
}
