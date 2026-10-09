use super::*;

#[test]
fn generated_secrets_are_retained_when_store_commit_cannot_be_verified() {
    for caller in ["profile_proxy", "identity_rotation", "one_key"] {
        let mut store = SessionStore::default();
        let durable = Mutex::new(None);
        let new_secret_ref = format!("secret:{caller}");
        let mut secrets = HashSet::from([new_secret_ref.clone()]);
        let error = commit_store_mutation_with_state(
            &mut store,
            |next_store| {
                next_store.one_keys.push(OneKeyCredential {
                    id: caller.to_string(),
                    label: caller.to_string(),
                    kind: OneKeyKind::Account,
                    username: "user".to_string(),
                    password_secret_ref: Some(new_secret_ref.clone()),
                    passphrase_secret_ref: None,
                    identity: None,
                    session_ids: Vec::new(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                });
                Ok(())
            },
            |next_store| {
                *durable.lock().unwrap() = Some(next_store.clone());
                Err("post-commit read failed".to_string())
            },
            |_| Err("verification unavailable".to_string()),
        )
        .unwrap_err();
        if error.state == StoreCommitState::NotCommitted {
            secrets.remove(&new_secret_ref);
        }
        assert_eq!(error.state, StoreCommitState::Unknown);
        let durable = durable.lock().unwrap();
        let persisted_ref = durable.as_ref().unwrap().one_keys[0]
            .password_secret_ref
            .as_ref()
            .unwrap();
        assert!(secrets.contains(persisted_ref));
        assert!(store.one_keys.is_empty());
    }
}

#[test]
fn definite_store_failures_allow_generated_secret_cleanup() {
    let mut store = SessionStore::default();
    let error = commit_store_mutation_with_state(
        &mut store,
        |_| Ok(()),
        |_| Err("write rejected".to_string()),
        |_| Ok(false),
    )
    .unwrap_err();
    assert_eq!(error.state, StoreCommitState::NotCommitted);
    assert_eq!(error.message, "write rejected");

    let error = commit_store_mutation_with_state(
        &mut store,
        |_| Err::<(), _>("mutation rejected".to_string()),
        |_| panic!("invalid mutations cannot persist"),
        |_| panic!("invalid mutations cannot need verification"),
    )
    .unwrap_err();
    assert_eq!(error.state, StoreCommitState::NotCommitted);
}
