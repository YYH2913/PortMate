#[cfg(test)]
pub(super) fn flush_json_compatibility_snapshot(
    store_path: &Path,
    timeout: Duration,
) -> Result<(), String> {
    let snapshot_path = compatibility_snapshot_path(store_path);
    wait_for_compatibility_snapshots(timeout, |state| {
        !state.pending.contains_key(&snapshot_path) && !state.writing.contains(&snapshot_path)
    })
}
fn assert_test_json_snapshot_flushed(store_path: &Path) {
    if let Err(error) = flush_json_compatibility_snapshot(store_path, Duration::from_secs(5)) {
        panic!("test JSON compatibility snapshot did not flush: {error}");
    }
}
