#[cfg(test)]
pub(super) fn commit_sysmon_snapshot(
    state: &AppState,
    session_id: &str,
    snapshot: SysmonSnapshot,
) -> Result<SysmonSnapshot, String> {
    commit_sysmon_snapshot_for_target(state, session_id, snapshot, &SysmonCollectionTarget::Local)
}
