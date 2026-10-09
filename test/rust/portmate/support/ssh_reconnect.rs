#[cfg(test)]
pub(super) fn take_forced_ssh_reconnect_install_error(state: &AppState) -> Option<String> {
    state
        .ssh_reconnect_install_error
        .lock()
        .ok()
        .and_then(|mut error| error.take())
}
