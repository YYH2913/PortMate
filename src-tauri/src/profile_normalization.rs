use super::*;

pub(super) const MAX_SESSION_PROFILE_NAME_CHARACTERS: usize = 128;
pub(super) const MAX_SESSION_PROFILE_ID_CHARACTERS: usize = 256;
pub(super) const MAX_SESSION_PROFILE_GROUP_CHARACTERS: usize = 256;
pub(super) const MAX_SESSION_PROFILE_TAGS: usize = 32;
pub(super) const MAX_SESSION_PROFILE_TAG_CHARACTERS: usize = 64;
pub(super) const MAX_LOG_RETENTION_DAYS: u32 = 3_650;
pub(super) const DEFAULT_TERMINAL_THEME: &str = "portmate-dark";
pub(super) const DEFAULT_TERMINAL_NAME: &str = "xterm-256color";
pub(super) const DEFAULT_TERMINAL_FONT_FAMILY: &str =
    "\"JetBrains Mono\", \"Noto Sans Mono CJK SC\", \"Sarasa Mono SC\", \"Microsoft YaHei UI\", monospace";
const LEGACY_DEFAULT_TERMINAL_FONT_FAMILIES: [&str; 2] = [
    "Roboto Mono, JetBrains Mono, monospace",
    "JetBrains Mono, monospace",
];
pub(super) const MIN_TERMINAL_ROWS: u16 = 1;
pub(super) const MAX_TERMINAL_ROWS: u16 = 512;
pub(super) const MIN_TERMINAL_COLS: u16 = 1;
pub(super) const MAX_TERMINAL_COLS: u16 = 1024;
pub(super) const MAX_TERMINAL_SCROLLBACK: u32 = 10_000_000;
pub(super) const MIN_TERMINAL_FONT_SIZE: u8 = 6;
pub(super) const MAX_TERMINAL_FONT_SIZE: u8 = 72;
pub(super) const MIN_TERMINAL_BACKGROUND_OPACITY: u8 = 20;
pub(super) const MAX_TERMINAL_BACKGROUND_OPACITY: u8 = 100;
pub(super) const MAX_TERMINAL_NAME_BYTES: usize = 64;
pub(super) const MAX_TERMINAL_FONT_FAMILY_CHARACTERS: usize = 256;
pub(super) const SUPPORTED_TERMINAL_THEMES: [&str; 4] = [
    DEFAULT_TERMINAL_THEME,
    "graphite",
    "solarized-dark",
    "portmate-light",
];

pub(super) fn normalized_terminal_name(term: &str) -> &str {
    let term = term.trim();
    if term.len() <= MAX_TERMINAL_NAME_BYTES
        && term
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && term
            .bytes()
            .skip(1)
            .all(|byte| byte.is_ascii_alphanumeric() || b"._+-".contains(&byte))
    {
        term
    } else {
        DEFAULT_TERMINAL_NAME
    }
}

pub(super) fn normalized_terminal_font_family(font_family: &str) -> String {
    let font_family = font_family.trim();
    if !font_family.is_empty()
        && font_family.chars().count() <= MAX_TERMINAL_FONT_FAMILY_CHARACTERS
        && !font_family.chars().any(char::is_control)
        && !LEGACY_DEFAULT_TERMINAL_FONT_FAMILIES.contains(&font_family)
    {
        font_family.to_string()
    } else {
        DEFAULT_TERMINAL_FONT_FAMILY.to_string()
    }
}

pub(super) fn normalized_terminal_theme(theme: &str) -> &str {
    let theme = theme.trim();
    if SUPPORTED_TERMINAL_THEMES.contains(&theme) {
        theme
    } else {
        DEFAULT_TERMINAL_THEME
    }
}

pub(super) fn normalized_profile_metadata_text(value: &str, max_characters: usize) -> String {
    let clean = value
        .chars()
        .filter(|character| !character.is_control())
        .collect::<String>();
    clean
        .trim()
        .chars()
        .take(max_characters)
        .collect::<String>()
        .trim()
        .to_string()
}

pub(super) fn normalized_session_profile_id(value: &str) -> String {
    normalized_profile_metadata_text(value, MAX_SESSION_PROFILE_ID_CHARACTERS)
}

pub(super) fn normalize_session_profile(mut profile: SessionProfile) -> SessionProfile {
    profile.id = normalized_session_profile_id(&profile.id);
    if profile.id.is_empty() {
        profile.id = format!("session-{}", Uuid::new_v4());
    }
    profile.name =
        normalized_profile_metadata_text(&profile.name, MAX_SESSION_PROFILE_NAME_CHARACTERS);
    if profile.name.is_empty() {
        profile.name =
            normalized_profile_metadata_text(&profile.id, MAX_SESSION_PROFILE_NAME_CHARACTERS);
    }
    if profile.name.is_empty() {
        profile.name = "未命名会话".to_string();
    }
    profile.group =
        normalized_profile_metadata_text(&profile.group, MAX_SESSION_PROFILE_GROUP_CHARACTERS);
    let mut tags = Vec::new();
    let mut seen_tags = HashSet::new();
    for tag in profile.tags {
        let tag = normalized_profile_metadata_text(&tag, MAX_SESSION_PROFILE_TAG_CHARACTERS);
        if tag.is_empty() || !seen_tags.insert(tag.clone()) {
            continue;
        }
        tags.push(tag);
        if tags.len() >= MAX_SESSION_PROFILE_TAGS {
            break;
        }
    }
    profile.tags = tags;
    profile.kind = session_kind_for_connection(&profile.connection);
    profile.terminal.term = normalized_terminal_name(&profile.terminal.term).to_string();
    profile.terminal.rows = profile
        .terminal
        .rows
        .clamp(MIN_TERMINAL_ROWS, MAX_TERMINAL_ROWS);
    profile.terminal.cols = profile
        .terminal
        .cols
        .clamp(MIN_TERMINAL_COLS, MAX_TERMINAL_COLS);
    profile.terminal.scrollback = profile.terminal.scrollback.min(MAX_TERMINAL_SCROLLBACK);
    profile.terminal.font_family = normalized_terminal_font_family(&profile.terminal.font_family);
    profile.terminal.font_size = profile
        .terminal
        .font_size
        .clamp(MIN_TERMINAL_FONT_SIZE, MAX_TERMINAL_FONT_SIZE);
    profile.terminal.theme = normalized_terminal_theme(&profile.terminal.theme).to_string();
    profile.terminal.background_opacity = profile.terminal.background_opacity.clamp(
        MIN_TERMINAL_BACKGROUND_OPACITY,
        MAX_TERMINAL_BACKGROUND_OPACITY,
    );
    profile.logging.retention_days = profile.logging.retention_days.min(MAX_LOG_RETENTION_DAYS);
    profile.triggers = normalize_triggers(profile.triggers);

    match &mut profile.connection {
        ConnectionConfig::Ssh(ssh) | ConnectionConfig::Tmux(ssh) => {
            ssh.endpoint.host = ssh.endpoint.host.trim().to_string();
            if ssh.endpoint.port == 0 {
                ssh.endpoint.port = 22;
            }
            ssh.username = ssh.username.trim().to_string();
            ssh.normalize_health_settings();
            ssh.proxy.normalize();
            ssh.tunnels = normalize_tunnels(std::mem::take(&mut ssh.tunnels));
            let alias = ssh
                .host_key_policy
                .alias
                .as_deref()
                .map(str::trim)
                .filter(|alias| !alias.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| profile.id.clone());
            ssh.host_key_policy.alias = Some(alias);
            for key in &mut ssh.trusted_host_keys {
                if key.scope == HostKeyScope::Profile {
                    key.profile_id = Some(
                        key.profile_id
                            .as_deref()
                            .map(str::trim)
                            .filter(|profile_id| !profile_id.is_empty())
                            .unwrap_or(&profile.id)
                            .to_string(),
                    );
                }
                key.alias = key.alias.trim().to_string();
            }
            ssh.trusted_host_keys.retain(|key| {
                key.scope != HostKeyScope::Profile
                    || key.profile_id.as_deref() == Some(profile.id.as_str())
            });
            for jump in &mut ssh.jumps {
                jump.host = jump.host.trim().to_string();
                if jump.port == 0 {
                    jump.port = 22;
                }
                jump.username = jump.username.trim().to_string();
                if jump.username.is_empty() {
                    jump.username = ssh.username.clone();
                }
                jump.identity_ref = jump
                    .identity_ref
                    .as_deref()
                    .map(str::trim)
                    .filter(|identity_ref| !identity_ref.is_empty())
                    .map(ToOwned::to_owned);
            }
            ssh.jumps.retain(|jump| !jump.host.is_empty());
            let mut normalized_auth_order = Vec::new();
            for method in ssh.identity_policy.auth_order.drain(..) {
                if !normalized_auth_order.contains(&method) {
                    normalized_auth_order.push(method);
                }
            }
            if normalized_auth_order.is_empty() {
                normalized_auth_order = vec![
                    AuthMethod::PublicKey,
                    AuthMethod::KeyboardInteractive,
                    AuthMethod::Password,
                ];
            }
            ssh.identity_policy.auth_order = normalized_auth_order;
            if !ssh.identity_policy.record_success
                || ssh
                    .identity_policy
                    .last_successful
                    .is_some_and(|method| !ssh.identity_policy.auth_order.contains(&method))
            {
                ssh.identity_policy.last_successful = None;
            }
        }
        ConnectionConfig::Tcp(tcp) | ConnectionConfig::Telnet(tcp) => {
            tcp.host = tcp.host.trim().to_string();
            tcp.proxy.normalize();
            tcp.normalize_health_settings();
        }
        ConnectionConfig::Serial(serial) => {
            serial.port = serial.port.trim().to_string();
            serial.normalize_health_settings();
        }
        ConnectionConfig::Shell(shell) => {
            if shell.program.trim().is_empty() {
                shell.program.clear();
            }
        }
    }

    profile
}

fn session_kind_for_connection(connection: &ConnectionConfig) -> SessionKind {
    match connection {
        ConnectionConfig::Ssh(_) => SessionKind::Ssh,
        ConnectionConfig::Serial(_) => SessionKind::Serial,
        ConnectionConfig::Shell(_) => SessionKind::Shell,
        ConnectionConfig::Telnet(_) => SessionKind::Telnet,
        ConnectionConfig::Tcp(_) => SessionKind::Tcp,
        ConnectionConfig::Tmux(_) => SessionKind::Tmux,
    }
}
