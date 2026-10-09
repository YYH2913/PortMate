fn configure_test_libssh_trace(session: &libssh_rs::Session) -> Result<(), String> {
    if std::env::var_os("PORTMATE_COMPAT_LIBSSH_TRACE").is_some() {
        session
            .set_option(libssh_rs::SshOption::LogLevel(
                libssh_rs::LogLevel::Protocol,
            ))
            .map_err(|error| format!("libssh 设置测试日志级别失败: {error}"))?;
    }
    Ok(())
}
