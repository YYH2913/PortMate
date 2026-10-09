#[cfg(test)]
pub(super) fn open_shell_session(
    state: &AppState,
    profile: SessionProfile,
) -> Result<SessionSummary, String> {
    install_shell_session(state, prepare_shell_session(profile)?)
}
