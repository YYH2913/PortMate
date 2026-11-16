use super::*;
use tauri::test::{mock_builder, mock_context, noop_assets};

#[test]
fn non_workspace_windows_cannot_download_launch_cancel_or_recheck_dependencies() {
    tauri::async_runtime::block_on(async {
        let root = tempfile::tempdir().unwrap();
        let app = mock_builder()
            .manage(EasyTierRuntime::default())
            .build(mock_context(noop_assets()))
            .unwrap();
        let state = crate::tests::test_app_state(
            crate::tests::test_shell_profile(),
            root.path().join("store.sqlite3"),
        );
        app.manage(state);
        let window = tauri::WebviewWindowBuilder::new(
            &app,
            "serial-analyzer-test",
            tauri::WebviewUrl::default(),
        )
        .build()
        .unwrap();
        for result in [
            download_npcap_installer(window.clone(), app.state(), app.state()).await,
            launch_npcap_installer(window.clone(), app.state(), app.state()).await,
            recheck_npcap(window.clone(), app.state()).await,
        ] {
            assert!(matches!(result, Err(error) if error.code == "unavailable"));
        }
        assert!(
            matches!(cancel_npcap_download(window, app.state()), Err(error) if error.code == "unavailable")
        );
        assert!(
            fs::read_dir(root.path()).unwrap().next().is_none(),
            "authorization failure created setup files"
        );
    });
}
