use super::*;
use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};

const PUBLIC_KEY: &str = include_str!("../../../fixtures/updater/public.key");
const SIGNATURE: &str = include_str!("../../../fixtures/updater/payload.txt.sig");
const PAYLOAD: &[u8] = include_bytes!("../../../fixtures/updater/payload.txt");

fn app() -> tauri::App<MockRuntime> {
    let mut context = mock_context(noop_assets());
    context.config_mut().plugins.0.insert(
        "updater".into(),
        serde_json::json!({"pubkey":PUBLIC_KEY.trim()}),
    );
    mock_builder()
        .plugin(
            tauri_plugin_updater::Builder::new()
                .pubkey(PUBLIC_KEY.trim())
                .build(),
        )
        .manage(UpdateRuntime::default())
        .invoke_handler(tauri::generate_handler![check_for_updates, install_update])
        .build(context)
        .unwrap()
}

fn candidate(app: &tauri::App<MockRuntime>, url: &str, version: &str) -> Update {
    app.updater_builder().target("test")
        .endpoints(vec!["https://example.invalid/latest.json".parse().unwrap()]).unwrap()
        .version_comparator(|_, _| true).build().unwrap()
        .restore_update(serde_json::json!({"version": version,"platforms":{"test":{"url":url,"signature":SIGNATURE.trim()}}})).unwrap().unwrap()
}

async fn serve(payload: &'static [u8]) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let worker = tokio::spawn(async move {
        let (mut client, _) = listener.accept().await.unwrap();
        let mut request = [0_u8; 4096];
        client.read(&mut request).await.unwrap();
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        );
        client.write_all(header.as_bytes()).await.unwrap();
        client.write_all(payload).await.unwrap();
    });
    (format!("http://127.0.0.1:{port}/package"), worker)
}

#[test]
fn update_signature_rejects_payload_or_signed_version_substitution() {
    let app = app();
    assert!(candidate(&app, "https://example.invalid/package", "0.2.0")
        .verify(PAYLOAD)
        .is_ok());
    assert!(candidate(&app, "https://example.invalid/package", "0.2.0")
        .verify(b"tampered")
        .is_err());
    assert!(candidate(&app, "https://example.invalid/package", "0.3.0")
        .verify(PAYLOAD)
        .is_err());
}

#[test]
fn update_commands_restrict_window_owners_and_expired_candidates() {
    tauri::async_runtime::block_on(async {
        let app = app();
        let main = tauri::WebviewWindowBuilder::new(&app, "main", tauri::WebviewUrl::default())
            .build()
            .unwrap();
        let detached =
            tauri::WebviewWindowBuilder::new(&app, "pane-test", tauri::WebviewUrl::default())
                .build()
                .unwrap();
        assert_eq!(authorize(&detached).unwrap_err().code, "unavailable");
        assert!(authorize(&main).is_ok());
        let root = tempfile::tempdir().unwrap();
        let state = crate::tests::test_app_state(
            crate::tests::test_shell_profile(),
            root.path().join("store.sqlite3"),
        );
        app.manage(state);
        let runtime = app.state::<UpdateRuntime>();
        *runtime.candidate.lock().await = Some(UpdateCandidate {
            id: "known".into(),
            owner: "workspace-another".into(),
            checked_at: Instant::now(),
            update: candidate(&app, "https://example.invalid/package", "0.2.0"),
        });
        let result = install_update(
            app.handle().clone(),
            main.clone(),
            app.state(),
            runtime.clone(),
            "known".into(),
            Channel::new(|_| Ok(())),
        )
        .await;
        assert_eq!(result.unwrap_err().code, "expired");
        let mut guard = runtime.candidate.lock().await;
        let cached = guard.as_mut().unwrap();
        cached.owner = "main".into();
        cached.checked_at = Instant::now() - CANDIDATE_TTL;
        drop(guard);
        let result = install_update(
            app.handle().clone(),
            main,
            app.state(),
            runtime.clone(),
            "known".into(),
            Channel::new(|_| Ok(())),
        )
        .await;
        assert_eq!(result.unwrap_err().code, "expired");
        assert!(!runtime.preparing.load(Ordering::SeqCst));
        let guard = runtime.candidate.lock().await;
        assert!(
            runtime.candidate.try_lock().is_err(),
            "concurrent operations must be rejected"
        );
        drop(guard);
    });
}

#[cfg(unix)]
#[test]
fn update_only_stops_owned_shell_after_successful_download_and_verification() {
    tauri::async_runtime::block_on(async {
        let app = app();
        let root = tempfile::tempdir().unwrap();
        let pid_file = root.path().join("shell.pid");
        let mut profile = crate::tests::test_shell_profile();
        let ConnectionConfig::Shell(shell) = &mut profile.connection else {
            unreachable!()
        };
        shell.program = "/bin/sh".into();
        shell.args = vec![
            "-c".into(),
            format!("echo $$ > '{}'; exec sleep 30", pid_file.display()),
        ];
        let mut state =
            crate::tests::test_app_state(profile.clone(), root.path().join("store.sqlite3"));
        state.app_handle = Some(app.handle().clone());
        install_shell_session(&state, prepare_shell_session(profile.clone()).unwrap()).unwrap();
        let closed = Arc::clone(&state.shell.lock().unwrap().get(&profile.id).unwrap().closed);
        tokio::time::timeout(Duration::from_secs(5), async {
            while !pid_file.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let pid: i32 = fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let runtime = app.state::<UpdateRuntime>();
        let progress = Channel::new(|_| Ok(()));
        let (url, server) = serve(b"tampered").await;
        let bad = candidate(&app, &url, "0.2.0");
        let result = download_and_prepare_update(&bad, &state, runtime.inner(), &progress).await;
        server.await.unwrap();
        assert_eq!(result.unwrap_err().code, "download-failed");
        assert!(!closed.load(Ordering::SeqCst));
        assert!(!runtime.preparing.load(Ordering::SeqCst));
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            0,
            "failed verification must leave the owned process running"
        );
        let (url, server) = serve(PAYLOAD).await;
        let good = candidate(&app, &url, "0.2.0");
        assert_eq!(
            download_and_prepare_update(&good, &state, runtime.inner(), &progress)
                .await
                .unwrap(),
            PAYLOAD
        );
        server.await.unwrap();
        assert!(closed.load(Ordering::SeqCst));
        assert!(state.shell.lock().unwrap().is_empty());
        assert!(runtime.preparing.load(Ordering::SeqCst));
        assert!(ensure_not_updating(&state).is_err());
        assert!(session_open::register_session_open_cancellation(&state, &profile.id).is_err());
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "owned shell must be terminated and reaped before installation"
        );
        assert!(state.store_path.exists());
    });
}
