#[cfg(all(test, unix))]
pub(super) fn open_serial_session(
    state: &AppState,
    profile: SessionProfile,
) -> Result<SessionSummary, String> {
    install_serial_session(state, prepare_serial_session(state, profile)?)
}
