use super::*;

#[cfg(unix)]
#[test]
fn sqlite_rejects_a_symlink_substituted_after_the_guard_open() {
    let root = tempfile::tempdir().unwrap();
    let selected = root.path().join("store.sqlite3");
    let outside = root.path().join("outside.sqlite3");
    SqliteConnection::open(&outside)
        .unwrap()
        .execute_batch("CREATE TABLE victim(value); INSERT INTO victim VALUES ('original');")
        .unwrap();
    fs::write(&selected, []).unwrap();
    let result = open_store_sqlite_with_hook(&selected, || {
        fs::remove_file(&selected).unwrap();
        std::os::unix::fs::symlink(&outside, &selected).unwrap();
    });
    assert!(result.is_err());
    let db = SqliteConnection::open(outside).unwrap();
    let value: String = db
        .query_row("SELECT value FROM victim", [], |row| row.get(0))
        .unwrap();
    assert_eq!(value, "original");
}

#[test]
fn store_lock_and_sqlite_reject_preexisting_hard_links() {
    let root = tempfile::tempdir().unwrap();
    let outside = root.path().join("outside.sqlite3");
    fs::write(&outside, b"unchanged").unwrap();
    let selected = root.path().join("store.sqlite3");
    fs::hard_link(&outside, &selected).unwrap();
    assert!(open_store_file(&selected, true).is_err());
    assert!(open_store_sqlite(&selected).is_err());
    assert_eq!(fs::read(outside).unwrap(), b"unchanged");
}
