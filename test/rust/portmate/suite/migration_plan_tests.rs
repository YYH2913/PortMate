#[test]
fn profile_secret_migration_preserves_sharing_scope_and_reserved_tokens() {
    let mut first = test_ssh_profile();
    if let ConnectionConfig::Ssh(ssh) = &mut first.connection {
        ssh.password_secret_ref = Some(" keychain:shared ".to_string());
        ssh.passphrase_secret_ref = Some("keychain:target-passphrase".to_string());
        ssh.identity_refs = vec![
            vault_identity("shared-key", "keychain:shared"),
            vault_identity("portable-key", "stronghold:already-portable"),
        ];
        ssh.jumps = vec![
            portmate_core::JumpHop {
                host: "bastion.example".to_string(),
                port: 22,
                username: "root".to_string(),
                password_secret_ref: Some("keychain:jump-password".to_string()),
                passphrase_secret_ref: Some("keychain:jump-passphrase".to_string()),
                identity_ref: None,
                host_key_policy: None,
            },
            portmate_core::JumpHop {
                host: "reserved.example".to_string(),
                port: 22,
                username: "root".to_string(),
                password_secret_ref: Some(MCP_HTTP_TOKEN_REF.to_string()),
                passphrase_secret_ref: None,
                identity_ref: None,
                host_key_policy: None,
            },
        ];
    }
    let mut unselected = test_ssh_profile();
    unselected.id = "ssh-session-2".to_string();
    unselected.name = "Unselected SSH".to_string();
    if let ConnectionConfig::Ssh(ssh) = &mut unselected.connection {
        ssh.password_secret_ref = Some("shared".to_string());
    }
    let mut tmux = test_ssh_profile();
    tmux.id = "tmux-session-1".to_string();
    tmux.name = "Selected Tmux".to_string();
    tmux.kind = SessionKind::Tmux;
    let mut tmux_ssh = match tmux.connection {
        ConnectionConfig::Ssh(ssh) => ssh,
        _ => panic!("expected SSH profile"),
    };
    tmux_ssh.password_secret_ref = Some("keychain:tmux-password".to_string());
    tmux.connection = ConnectionConfig::Tmux(tmux_ssh);

    let mut store = SessionStore::default();
    store.upsert_profile(first);
    store.upsert_profile(unselected.clone());
    store.upsert_profile(tmux);
    let request = ProfileSecretMigrationRequest {
        target_storage: SecretStorage::Portable,
        profile_ids: vec!["ssh-session-1".to_string(), "tmux-session-1".to_string()],
        cleanup_source: true,
    };
    let plan = build_profile_secret_migration_plan(&store, &request).unwrap();
    assert_eq!(plan.preview.selected_profile_count, 2);
    assert_eq!(plan.preview.affected_profile_count, 2);
    assert_eq!(plan.preview.eligible_reference_count, 6);
    assert_eq!(plan.preview.eligible_secret_count, 5);
    assert_eq!(plan.preview.retained_shared_secret_count, 1);
    assert_eq!(plan.preview.already_target_reference_count, 1);
    assert_eq!(plan.preview.excluded_reserved_reference_count, 1);
    let plan_token = profile_secret_migration_plan_token(&plan, &request);
    assert_eq!(plan_token.len(), 64);
    let mut changed_store = store.clone();
    if let ConnectionConfig::Ssh(ssh) = &mut changed_store.profiles[0].connection {
        ssh.password_secret_ref = Some("keychain:changed".to_string());
    }
    let changed_plan = build_profile_secret_migration_plan(&changed_store, &request).unwrap();
    assert_ne!(
        profile_secret_migration_plan_token(&changed_plan, &request),
        plan_token
    );
    let mut unrelated_store = store.clone();
    unrelated_store.profiles[0].name = "renamed only".to_string();
    let unrelated_plan = build_profile_secret_migration_plan(&unrelated_store, &request).unwrap();
    assert_eq!(
        profile_secret_migration_plan_token(&unrelated_plan, &request),
        plan_token
    );

    let source_values = HashMap::from([
        ("keychain:shared".to_string(), "secret-shared".to_string()),
        (
            "keychain:target-passphrase".to_string(),
            "secret-target-passphrase".to_string(),
        ),
        (
            "keychain:jump-password".to_string(),
            "secret-jump-password".to_string(),
        ),
        (
            "keychain:jump-passphrase".to_string(),
            "secret-jump-passphrase".to_string(),
        ),
        (
            "keychain:tmux-password".to_string(),
            "secret-tmux-password".to_string(),
        ),
    ]);
    let written = std::rc::Rc::new(std::cell::RefCell::new(HashMap::new()));
    let deleted = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let persisted = std::rc::Rc::new(std::cell::RefCell::new(None));
    let source_values_for_write = source_values.clone();
    let response = migrate_profile_secrets_with_io(
        &mut store,
        &request,
        |secret_ref| {
            source_values
                .get(secret_ref)
                .cloned()
                .ok_or_else(|| format!("missing test secret: {secret_ref}"))
        },
        {
            let written = std::rc::Rc::clone(&written);
            move |storage, entries| {
                assert_eq!(storage, SecretStorage::Portable);
                let mut written = written.borrow_mut();
                for entry in entries {
                    assert_eq!(
                        entry.secret.as_str(),
                        source_values_for_write[&entry.source_ref]
                    );
                    written.insert(entry.target_ref.clone(), entry.secret.to_string());
                }
                Ok(false)
            }
        },
        {
            let deleted = std::rc::Rc::clone(&deleted);
            move |storage, secret_refs| {
                assert_eq!(storage, SecretStorage::Native);
                deleted.borrow_mut().extend(secret_refs.iter().cloned());
                SecretBatchDeleteOutcome {
                    results: secret_refs
                        .iter()
                        .map(|secret_ref| (secret_ref.clone(), Ok(())))
                        .collect(),
                    portable_vault_requires_reunlock: false,
                }
            }
        },
        {
            let persisted = std::rc::Rc::clone(&persisted);
            move |next_store, _, _| {
                *persisted.borrow_mut() = Some(next_store.clone());
                ProfileSecretStoreCommit::Committed { warning: None }
            }
        },
    )
    .unwrap();

    assert_eq!(response.selected_profile_count, 2);
    assert_eq!(response.migrated_profile_count, 2);
    assert_eq!(response.migrated_reference_count, 6);
    assert_eq!(response.migrated_secret_count, 5);
    assert_eq!(response.summaries.len(), 2);
    assert_eq!(written.borrow().len(), 5);
    assert_eq!(deleted.borrow().len(), 4);
    assert!(!deleted.borrow().contains(&"keychain:shared".to_string()));
    assert!(persisted.borrow().is_some());

    let migrated = store.profile("ssh-session-1").unwrap();
    let migrated_ssh = ssh_connection(&migrated).unwrap();
    let shared_target = migrated_ssh.password_secret_ref.as_deref().unwrap();
    assert!(shared_target.starts_with("stronghold:"));
    assert_eq!(
        migrated_ssh.identity_refs[0].secret_ref.as_deref(),
        Some(shared_target)
    );
    assert_eq!(
        migrated_ssh.identity_refs[1].secret_ref.as_deref(),
        Some("stronghold:already-portable")
    );
    assert_eq!(
        migrated_ssh.jumps[1].password_secret_ref.as_deref(),
        Some(MCP_HTTP_TOKEN_REF)
    );
    assert_eq!(store.profile("ssh-session-2").unwrap(), unselected);
    let shared_item = response
        .items
        .iter()
        .find(|item| item.source_ref == "keychain:shared")
        .unwrap();
    assert_eq!(shared_item.reference_count, 2);
    assert_eq!(shared_item.remaining_source_references, 1);
    assert_eq!(
        shared_item.cleanup_status,
        ProfileSecretCleanupStatus::RetainedShared
    );
    let encoded = serde_json::to_string(&response).unwrap();
    for secret in source_values.values() {
        assert!(!encoded.contains(secret));
    }
}
