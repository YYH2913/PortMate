use super::*;
use config::AddressMode;
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};

struct TestApp(tauri::App<MockRuntime>);

impl std::ops::Deref for TestApp {
    type Target = tauri::App<MockRuntime>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for TestApp {
    fn drop(&mut self) {
        shutdown_for_app(self.0.state::<AppState>().inner());
    }
}

fn app(root: &Path) -> (TestApp, WebviewWindow) {
    let app = mock_builder()
        .manage(EasyTierRuntime::default())
        .build(mock_context(noop_assets()))
        .unwrap();
    let mut state = crate::tests::test_app_state(
        crate::tests::test_shell_profile(),
        root.join("store.sqlite3"),
    );
    state.app_handle = Some(app.handle().clone());
    app.manage(state);
    let window = tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::default())
        .build()
        .unwrap();
    (TestApp(app), window)
}

#[test]
fn real_two_node_commands_and_lifecycle() {
    tauri::async_runtime::block_on(async {
        let root = tempfile::tempdir().unwrap();
        let (first, first_window) = app(&root.path().join("first"));
        let (second, second_window) = app(&root.path().join("second"));
        // EasyTier binds outgoing connectors to physical interface addresses;
        // connecting to loopback from that source can time out on Linux.
        let route = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
        route.connect("192.0.2.1:9").unwrap(); // Select a route; sends no packet.
        let local_ip = route.local_addr().unwrap().ip();
        let listener = std::net::TcpListener::bind((local_ip, 0)).unwrap();
        let address = format!("tcp://{}", listener.local_addr().unwrap());
        drop(listener);
        let config = EasyTierConfig {
            network_name: Uuid::new_v4().to_string(),
            hostname: "PortMate-node-A".into(),
            address_mode: AddressMode::Relay,
            listeners: vec![address.clone()],
            ..EasyTierConfig::default()
        };
        let other_config = EasyTierConfig {
            hostname: "PortMate-node-B".into(),
            listeners: vec![],
            peers: vec![address],
            ..config.clone()
        };
        let password = "real-test-only-quote\"-中文\\secret";
        start_easy_tier(
            first_window.clone(),
            first.state(),
            first.state(),
            config.clone(),
            password.into(),
        )
        .await
        .unwrap();
        start_easy_tier(
            second_window.clone(),
            second.state(),
            second.state(),
            other_config.clone(),
            password.into(),
        )
        .await
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let a = easy_tier_status(first_window.clone(), first.state(), first.state()).unwrap();
            let b =
                easy_tier_status(second_window.clone(), second.state(), second.state()).unwrap();
            assert_ne!(
                a.status.phase, "failed",
                "node A failed: {:?}",
                a.status.error
            );
            assert_ne!(
                b.status.phase, "failed",
                "node B failed: {:?}",
                b.status.error
            );
            if a.status.peers.len() == 1 && b.status.peers.len() == 1 {
                assert_eq!(a.status.peers[0].hostname, "PortMate-node-B");
                assert_eq!(b.status.peers[0].hostname, "PortMate-node-A");
                let public = serde_json::to_string(&a).unwrap();
                assert!(!public.contains(password));
                assert!(
                    !fs::read_to_string(root.path().join("first/easytier/settings.json"))
                        .unwrap()
                        .contains("secret")
                );
                break;
            }
            assert!(Instant::now() < deadline, "nodes failed to connect");
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        assert_eq!(
            save_easy_tier_config(first_window.clone(), first.state(), first.state(), config)
                .await
                .err()
                .unwrap()
                .code,
            "already-running"
        );
        stop_before_update(first.state::<AppState>().inner())
            .await
            .unwrap();
        assert!(inner(first.state::<EasyTierRuntime>().inner())
            .running
            .is_none());
        stop_easy_tier(second_window.clone(), second.state(), second.state())
            .await
            .unwrap();
        start_easy_tier(
            second_window,
            second.state(),
            second.state(),
            other_config,
            password.into(),
        )
        .await
        .unwrap();
        shutdown_for_app(second.state::<AppState>().inner());
        assert!(inner(second.state::<EasyTierRuntime>().inner())
            .running
            .is_none());
        for node in ["first", "second"] {
            assert!(
                fs::read_dir(root.path().join(node).join("easytier"))
                    .unwrap()
                    .all(|entry| !entry
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .starts_with("run-")),
                "temporary configuration survived cleanup"
            );
        }
    });
}
