use super::*;

pub(super) fn ssh_uses_libssh_gssapi_backend(ssh: &SshConnection) -> bool {
    let uses_supported_methods = ssh
        .identity_policy
        .auth_order
        .contains(&AuthMethod::GssapiWithMic)
        && ssh.identity_policy.auth_order.iter().all(|method| {
            matches!(
                method,
                AuthMethod::GssapiWithMic
                    | AuthMethod::PublicKey
                    | AuthMethod::KeyboardInteractive
                    | AuthMethod::Password
                    | AuthMethod::None
            )
        });
    uses_supported_methods && !libssh_auth_order_requires_filtered_agent(ssh)
}

fn libssh_auth_order_requires_filtered_agent(ssh: &SshConnection) -> bool {
    if !ssh
        .identity_policy
        .auth_order
        .contains(&AuthMethod::PublicKey)
        || !ssh.agent_policy.enabled
    {
        return false;
    }
    let has_agent_identity = ssh
        .identity_refs
        .iter()
        .any(|identity| identity.source == IdentitySource::Agent);
    has_agent_identity
}

pub(super) fn authenticate_libssh_with_order(
    session: &libssh_rs::Session,
    auth_order: &[AuthMethod],
    password: Option<&str>,
    identity_refs: &[IdentityRef],
    passphrase: Option<&str>,
    offer_agent_before: bool,
    offer_agent_after: bool,
) -> Result<AuthMethod, String> {
    let none = session
        .userauth_none(None)
        .map_err(|error| format!("libssh authentication capability probe failed: {error}"))?;
    if none == libssh_rs::AuthStatus::Success {
        return auth_order
            .contains(&AuthMethod::None)
            .then_some(AuthMethod::None)
            .ok_or_else(|| {
                "SSH server accepted none authentication, but the profile does not allow it"
                    .to_string()
            });
    }

    let mut methods = session
        .userauth_list(None)
        .map_err(|error| format!("libssh auth method query failed: {error}"))?;
    let mut attempted = Vec::new();
    let mut failures = Vec::new();

    for method in auth_order {
        let status = match method {
            AuthMethod::GssapiWithMic => {
                attempted.push("gssapi-with-mic");
                if !methods.contains(libssh_rs::AuthMethods::GSSAPI_MIC) {
                    failures.push("server did not advertise gssapi-with-mic".to_string());
                    continue;
                }
                session
                    .userauth_gssapi()
                    .map_err(|error| format!("libssh GSSAPI authentication failed: {error}"))?
            }
            AuthMethod::KeyboardInteractive => {
                let Some(password) = password else {
                    continue;
                };
                attempted.push("keyboard-interactive");
                if !methods.contains(libssh_rs::AuthMethods::INTERACTIVE) {
                    failures.push("server did not advertise keyboard-interactive".to_string());
                    continue;
                }
                authenticate_libssh_keyboard_interactive(session, password)?
            }
            AuthMethod::Password => {
                let Some(password) = password else {
                    continue;
                };
                attempted.push("password");
                if !methods.contains(libssh_rs::AuthMethods::PASSWORD) {
                    failures.push("server did not advertise password authentication".to_string());
                    continue;
                }
                session
                    .userauth_password(None, Some(password))
                    .map_err(|error| format!("libssh password authentication failed: {error}"))?
            }
            AuthMethod::None => {
                attempted.push("none");
                failures.push("server rejected none authentication".to_string());
                continue;
            }
            AuthMethod::PublicKey => {
                attempted.push("publickey");
                if !methods.contains(libssh_rs::AuthMethods::PUBLIC_KEY) {
                    failures.push("server did not advertise public-key authentication".to_string());
                    continue;
                }
                if offer_agent_before
                    && authenticate_libssh_agent(session, "before profile keys", &mut failures)?
                {
                    return Ok(AuthMethod::PublicKey);
                }
                let identities = identity_refs.iter().filter(|identity| {
                    matches!(
                        identity.source,
                        IdentitySource::SystemFile | IdentitySource::ProfileVault
                    )
                });
                let mut identity_attempted = false;
                for identity in identities {
                    identity_attempted = true;
                    let key = match load_libssh_private_key(identity, passphrase) {
                        Ok(Some(key)) => key,
                        Ok(None) => continue,
                        Err(error) => {
                            failures.push(format!("{}: {error}", identity.label));
                            continue;
                        }
                    };
                    let status = session.userauth_publickey(None, &key).map_err(|error| {
                        format!(
                            "libssh public-key authentication failed for {}: {error}",
                            identity.label
                        )
                    })?;
                    match status {
                        libssh_rs::AuthStatus::Success => return Ok(AuthMethod::PublicKey),
                        libssh_rs::AuthStatus::Denied => {
                            failures.push(format!("{}: public key was denied", identity.label));
                        }
                        libssh_rs::AuthStatus::Partial => failures.push(format!(
                            "{}: public key was only partially accepted",
                            identity.label
                        )),
                        libssh_rs::AuthStatus::Info | libssh_rs::AuthStatus::Again => {
                            return Err(format!(
                                "libssh public-key authentication for {} returned {status:?}",
                                identity.label
                            ));
                        }
                    }
                }
                if !identity_attempted && !offer_agent_before && !offer_agent_after {
                    failures
                        .push("profile has no usable explicit private-key identity".to_string());
                }
                if offer_agent_after
                    && authenticate_libssh_agent(session, "after profile keys", &mut failures)?
                {
                    return Ok(AuthMethod::PublicKey);
                }
                methods = session
                    .userauth_list(None)
                    .map_err(|error| format!("libssh auth method refresh failed: {error}"))?;
                continue;
            }
        };

        match status {
            libssh_rs::AuthStatus::Success => return Ok(*method),
            libssh_rs::AuthStatus::Denied => {
                failures.push(if *method == AuthMethod::GssapiWithMic {
                    "GSSAPI authentication was denied".to_string()
                } else {
                    format!("{method:?} was denied")
                });
            }
            libssh_rs::AuthStatus::Partial => {
                failures.push(if *method == AuthMethod::GssapiWithMic {
                    "GSSAPI authentication was only partially accepted".to_string()
                } else {
                    format!("{method:?} was only partially accepted")
                });
            }
            libssh_rs::AuthStatus::Info => {
                return Err(format!(
                    "libssh {method:?} authentication returned an unexpected prompt state"
                ));
            }
            libssh_rs::AuthStatus::Again => {
                return Err(format!(
                    "libssh {method:?} authentication unexpectedly requested a retry"
                ));
            }
        }
        methods = session
            .userauth_list(None)
            .map_err(|error| format!("libssh auth method refresh failed: {error}"))?;
    }

    if auth_order == [AuthMethod::GssapiWithMic]
        && failures
            .iter()
            .any(|failure| failure == "server did not advertise gssapi-with-mic")
    {
        return Err("SSH server did not advertise gssapi-with-mic".to_string());
    }
    let attempted = if attempted.is_empty() {
        "none".to_string()
    } else {
        attempted.join(", ")
    };
    let details = if failures.is_empty() {
        String::new()
    } else {
        format!("; {}", failures.join(" | "))
    };
    Err(format!(
        "libssh SSH authentication failed; attempted: {attempted}{details}"
    ))
}

fn authenticate_libssh_agent(
    session: &libssh_rs::Session,
    position: &str,
    failures: &mut Vec<String>,
) -> Result<bool, String> {
    let status = match session.userauth_agent(None) {
        Ok(status) => status,
        Err(error) => {
            failures.push(format!("SSH agent ({position}) failed: {error}"));
            return Ok(false);
        }
    };
    match status {
        libssh_rs::AuthStatus::Success => Ok(true),
        libssh_rs::AuthStatus::Denied => {
            failures.push(format!("SSH agent ({position}) was denied"));
            Ok(false)
        }
        libssh_rs::AuthStatus::Partial => {
            failures.push(format!(
                "SSH agent ({position}) was only partially accepted"
            ));
            Ok(false)
        }
        status @ (libssh_rs::AuthStatus::Info | libssh_rs::AuthStatus::Again) => Err(format!(
            "libssh SSH agent authentication ({position}) returned {status:?}"
        )),
    }
}

fn load_libssh_private_key(
    identity: &IdentityRef,
    passphrase: Option<&str>,
) -> Result<Option<libssh_rs::SshKey>, String> {
    load_libssh_private_key_with(identity, passphrase, read_secret_from_store)
}

pub(super) fn load_libssh_private_key_with<ReadSecret>(
    identity: &IdentityRef,
    passphrase: Option<&str>,
    read_secret: ReadSecret,
) -> Result<Option<libssh_rs::SshKey>, String>
where
    ReadSecret: FnOnce(&str) -> Result<String, String>,
{
    let private_key = match identity.source {
        IdentitySource::SystemFile => {
            let Some(path) = identity
                .path
                .as_deref()
                .map(str::trim)
                .filter(|path| !path.is_empty())
            else {
                return Ok(None);
            };
            let path = expand_identity_path(path);
            fs::read_to_string(&path)
                .map_err(|error| format!("system-file {}: {error}", path.display()))?
        }
        IdentitySource::ProfileVault => {
            let Some(secret_ref) = identity
                .secret_ref
                .as_deref()
                .map(str::trim)
                .filter(|secret_ref| !secret_ref.is_empty())
            else {
                return Err("profile-vault identity 缺少 secretRef".to_string());
            };
            read_secret(secret_ref)
                .map_err(|error| format!("profile-vault {secret_ref}: {error}"))?
        }
        IdentitySource::Agent | IdentitySource::PublicKeyOnly => return Ok(None),
    };
    libssh_rs::SshKey::from_privkey_base64(&private_key, passphrase)
        .map(Some)
        .map_err(|error| format!("private key 解析失败: {error}"))
}

fn authenticate_libssh_keyboard_interactive(
    session: &libssh_rs::Session,
    password: &str,
) -> Result<libssh_rs::AuthStatus, String> {
    for _ in 0..8 {
        let status = session
            .userauth_keyboard_interactive(None, None)
            .map_err(|error| {
                format!("libssh keyboard-interactive authentication failed: {error}")
            })?;
        if status != libssh_rs::AuthStatus::Info {
            return Ok(status);
        }
        let info = session
            .userauth_keyboard_interactive_info()
            .map_err(|error| format!("libssh keyboard-interactive prompt failed: {error}"))?;
        let answers = info
            .prompts
            .iter()
            .map(|prompt| {
                if prompt.echo {
                    String::new()
                } else {
                    password.to_string()
                }
            })
            .collect::<Vec<_>>();
        session
            .userauth_keyboard_interactive_set_answers(&answers)
            .map_err(|error| format!("libssh keyboard-interactive response failed: {error}"))?;
    }
    Err("libssh keyboard-interactive authentication exceeded 8 rounds".to_string())
}
