#[cfg(test)]
pub(super) fn save_store_with_expected_snapshot_version(
    path: &Path,
    store: &SessionStore,
    expected: &mut StoreSnapshotVersion,
) -> Result<(), String> {
    let snapshot_lock = lock_store_snapshot(path)?;
    let current = store_snapshot_version(path)?;
    let result = save_store_checked_locked(path, store, expected, current);
    drop(snapshot_lock);
    result
}
