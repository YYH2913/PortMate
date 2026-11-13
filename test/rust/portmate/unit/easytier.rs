use super::*;
#[cfg(all(target_os = "linux", feature = "bundled-easytier"))]
use config::AddressMode;

#[test]
fn rejects_config_injection_and_unsafe_endpoints() {
    let mut config = EasyTierConfig::default();
    assert!(config.validate().is_ok());
    for endpoint in [
        "http://host:11010",
        "udp://0.0.0.0:11010",
        "tcp://user:pass@host:22",
        "udp://host:0",
        "tcp://host:22?x=y",
    ] {
        config.peers = vec![endpoint.into()];
        assert!(config.validate().is_err(), "{endpoint}");
    }
    config.peers = vec![
        "udp://[::1]:11010".into(),
        "tcp://peer.example:11010".into(),
    ];
    assert!(config.validate().is_ok());
    config.network_name = "${HOME}".into();
    assert!(config.validate().is_err());
    for secret in ["", " \t", "value\n[flags]", "${ENV_SECRET}"] {
        assert!(config::validate_secret(secret).is_err());
    }
    assert!(config::validate_secret("中文-quote\"-backslash\\").is_ok());
}

#[test]
fn saves_only_nonsecret_settings_and_redacts_runtime_output() {
    let root = tempfile::tempdir().unwrap();
    let config = EasyTierConfig::default();
    config::save_config(root.path(), &config).unwrap();
    assert_eq!(config::load_config(root.path()).unwrap(), config);
    let json = fs::read_to_string(root.path().join("settings.json")).unwrap();
    assert!(!json.contains("secret"));
    let secret = "quote\"and\\slash";
    let text = config::render_config(&config, secret, "a-fixed-instance");
    assert!(text.contains("network_secret = \"quote\\\"and\\\\slash\""));
    assert!(!sanitize_log(format!("password: {secret}").as_bytes(), secret).contains(secret));
    assert_eq!(
        sanitize_log(b"network_secret_digest = [1, 2]", secret),
        "[network credentials omitted]"
    );
}

#[test]
fn prepares_embedded_files_offline_and_repairs_corruption() {
    const CORE: &[u8] = b"core";
    let hash: String = Sha256::digest(CORE)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let files = [bundled::EmbeddedFile {
        name: "easytier-core",
        bytes: CORE,
        sha256: Box::leak(hash.into_boxed_str()),
    }];
    let root = tempfile::tempdir().unwrap();
    let directory = bundled::prepare_files(root.path(), &files, "fake-payload-1").unwrap();
    let binary = directory.join("easytier-core");
    assert_eq!(fs::read(&binary).unwrap(), CORE);
    assert!(directory.join("LICENSE").exists());
    assert!(directory.join("COPYING").exists());
    let modified = fs::metadata(&binary).unwrap().modified().unwrap();
    assert_eq!(
        bundled::prepare_files(root.path(), &files, "fake-payload-1").unwrap(),
        directory
    );
    assert_eq!(
        fs::metadata(&binary).unwrap().modified().unwrap(),
        modified,
        "an intact core should not be rewritten"
    );
    fs::write(&binary, b"evil").unwrap(); // Same length; existence/size are insufficient.
    bundled::prepare_files(root.path(), &files, "fake-payload-1").unwrap();
    assert_eq!(fs::read(&binary).unwrap(), CORE);
    fs::remove_file(&binary).unwrap();
    bundled::prepare_files(root.path(), &files, "fake-payload-1").unwrap();
    assert_eq!(fs::read(&binary).unwrap(), CORE);
    #[cfg(unix)]
    {
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o600)).unwrap();
        bundled::prepare_files(root.path(), &files, "fake-payload-1").unwrap();
        assert_ne!(
            fs::metadata(&binary).unwrap().permissions().mode() & 0o100,
            0
        );
    }
    assert!(bundled::prepare_files(root.path(), &[], "missing").is_err());
}

#[cfg(feature = "bundled-easytier")]
#[tokio::test]
async fn bundled_core_and_cli_run_without_an_external_installation() {
    let root = tempfile::tempdir().unwrap();
    let directory = bundled::prepare(root.path()).unwrap();
    for name in ["easytier-core", "easytier-cli"] {
        let binary = bundled::executable(&directory, name).unwrap();
        let version = command_output(&binary, &["--version"]).await.unwrap();
        assert!(String::from_utf8_lossy(&version).contains(VERSION));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn kills_command_descendants_when_the_output_deadline_expires() {
    let root = tempfile::tempdir().unwrap();
    let script = root.path().join("slow-cli");
    fs::write(
        &script,
        "#!/bin/sh\nsleep 30 &\necho $! > child.pid\nwait\n",
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(command_output(&script, &[]).await.is_err());
    let pid: i32 = fs::read_to_string(root.path().join("child.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // A killed descendant may briefly remain a zombie until its OS parent reaps it.
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let proc_state = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default();
        if unsafe { libc::kill(pid, 0) } != 0 || proc_state.contains(") Z ") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "CLI descendant survived its command deadline"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[cfg(all(target_os = "linux", feature = "bundled-easytier"))]
#[tokio::test]
async fn real_easytier_configuration_rpc_and_cleanup() {
    let bundled_root = tempfile::tempdir().unwrap();
    let binaries = bundled::prepare(bundled_root.path()).unwrap();
    let core = bundled::executable(&binaries, "easytier-core").unwrap();
    let cli = bundled::executable(&binaries, "easytier-cli").unwrap();
    let files = tempfile::tempdir().unwrap();
    let id = Uuid::new_v4().to_string();
    let config = EasyTierConfig {
        address_mode: AddressMode::Relay,
        listeners: vec![],
        ..EasyTierConfig::default()
    };
    let path = files.path().join("network.toml");
    fs::write(
        &path,
        config::render_config(&config, "quote\"-unicode中文-backslash\\", &id),
    )
    .unwrap();
    command_output(
        &core,
        &["--config-file", path.to_str().unwrap(), "--check-config"],
    )
    .await
    .unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let rpc = listener.local_addr().unwrap().to_string();
    drop(listener);
    let mut command = tokio::process::Command::new(&core);
    clean_environment(&mut command);
    command.args([
        "--config-file",
        path.to_str().unwrap(),
        "--rpc-portal",
        &rpc,
        "--console-log-level",
        "error",
    ]);
    let (child, tree) = host_script_process::spawn(&mut command).unwrap();
    let runtime = EasyTierRuntime::default();
    inner(&runtime).running = Some(Running {
        id: id.clone(),
        child,
        tree,
        _files: files,
        cli: cli.clone(),
        rpc: rpc.clone(),
        started: Instant::now(),
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Ok((_, _, peers)) = query_network(&cli, &rpc, &id).await {
            assert!(
                peers.is_empty(),
                "the local node must not be shown as a connected peer"
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "EasyTier RPC did not become ready"
        );
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    assert!(query_network(&cli, &rpc, &Uuid::new_v4().to_string())
        .await
        .is_err());
    let directory = path.parent().unwrap().to_path_buf();
    stop(&runtime).await.unwrap();
    assert!(
        !directory.exists(),
        "temporary secret configuration must be removed"
    );
    assert!(
        std::net::TcpListener::bind(&rpc).is_ok(),
        "control port must be released"
    );
}
