use super::*;

pub(crate) fn normalize_loaded_one_keys(store: &mut SessionStore) {
    let profiles = store
        .profiles
        .iter()
        .map(|profile| (profile.id.clone(), profile.kind))
        .collect::<HashMap<_, _>>();
    let mut used_ids = HashSet::new();
    let one_keys = std::mem::take(&mut store.one_keys);
    for (index, mut one_key) in one_keys.into_iter().enumerate() {
        if store.one_keys.len() >= MAX_ONE_KEYS {
            break;
        }
        one_key.id = one_key.id.trim().to_string();
        if one_key.id.is_empty()
            || one_key.id.len() > 128
            || !one_key
                .id
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || ":_-".contains(character))
            || !used_ids.insert(one_key.id.clone())
        {
            one_key.id = format!("onekey:loaded:{}", index + 1);
            while !used_ids.insert(one_key.id.clone()) {
                one_key.id.push('x');
            }
        }
        one_key.label = truncate_one_key_text(
            one_key.label.trim().trim_matches('\0'),
            MAX_ONE_KEY_LABEL_CHARACTERS,
        );
        if one_key.label.is_empty() {
            one_key.label = format!("OneKey {}", store.one_keys.len() + 1);
        }
        one_key.username = truncate_one_key_text(
            one_key.username.trim().trim_matches(['\0', '\r', '\n']),
            MAX_ONE_KEY_USERNAME_CHARACTERS,
        );
        if one_key.username.is_empty() {
            continue;
        }
        one_key.password_secret_ref = one_key
            .password_secret_ref
            .as_deref()
            .and_then(canonical_secret_ref);
        one_key.passphrase_secret_ref = if one_key.kind == OneKeyKind::Ssh {
            one_key
                .passphrase_secret_ref
                .as_deref()
                .and_then(canonical_secret_ref)
        } else {
            None
        };
        one_key.identity = if one_key.kind == OneKeyKind::Ssh {
            one_key.identity.take().and_then(|selected| {
                let source_profile_id = selected.source_profile_id.trim().to_string();
                if source_profile_id.is_empty() {
                    None
                } else {
                    normalize_one_key_identity(source_profile_id, selected.identity).ok()
                }
            })
        } else {
            None
        };
        let mut session_ids = Vec::new();
        for session_id in one_key.session_ids {
            let session_id = session_id.trim();
            let Some(profile_kind) = profiles.get(session_id) else {
                continue;
            };
            if one_key.kind == OneKeyKind::Ssh
                && !matches!(profile_kind, SessionKind::Ssh | SessionKind::Tmux)
            {
                continue;
            }
            if !session_id.is_empty()
                && session_ids.len() < MAX_ONE_KEY_SESSIONS
                && !session_ids.iter().any(|existing| existing == session_id)
            {
                session_ids.push(session_id.to_string());
            }
        }
        one_key.session_ids = session_ids;
        if one_key.identity.as_ref().is_some_and(|selected| {
            !one_key
                .session_ids
                .iter()
                .any(|session_id| session_id == &selected.source_profile_id)
        }) {
            one_key.identity = None;
        }
        if one_key.password_secret_ref.is_none()
            && one_key.passphrase_secret_ref.is_none()
            && one_key.identity.is_none()
        {
            continue;
        }
        store.one_keys.push(one_key);
    }
}
