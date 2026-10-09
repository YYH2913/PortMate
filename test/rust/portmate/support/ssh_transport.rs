#[cfg(all(test, unix))]
pub(super) async fn establish_ssh_runtime_with_timeout(
    state: &AppState,
    profile: &SessionProfile,
    password: Option<String>,
    passphrase: Option<String>,
    connect_timeout: Duration,
    agent_socket_path: Option<PathBuf>,
) -> Result<EstablishedSshRuntime, String> {
    establish_ssh_runtime_with_timeout_mode(
        state,
        profile,
        password,
        passphrase,
        connect_timeout,
        agent_socket_path,
        SshEstablishmentOptions {
            enforce_profile_snapshot: false,
            automatic_reconnect: false,
        },
    )
    .await
}
