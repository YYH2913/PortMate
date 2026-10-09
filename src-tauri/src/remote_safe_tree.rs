use super::*;

const REMOTE_SAFE_TREE_SCRIPT: &str = include_str!("remote_safe_tree.py");

pub(super) async fn remote_safe_tree(
    handle: Arc<tokio::sync::Mutex<SshBackendSession>>,
    action: &str,
    paths: &[String],
) -> Result<String, String> {
    if paths.is_empty() || paths.len() > MAX_EXTERNAL_DROP_ROOTS {
        return Err("invalid remote tree root count".into());
    }
    for path in paths {
        validate_remote_mutating_path(path)?;
    }
    let args = serde_json::to_string(&(action, paths)).map_err(|error| error.to_string())?;
    let command = format!(
        "python3 -I -c {} {}",
        shell_quote(REMOTE_SAFE_TREE_SCRIPT),
        shell_quote(&args)
    );
    let output = exec_ssh_command_capture(handle, &command, Duration::from_secs(30)).await?;
    if action == "delete" {
        let result: serde_json::Value = serde_json::from_str(&output)
            .map_err(|error| format!("remote deletion did not confirm completion: {error}"))?;
        if result != serde_json::json!({"directories": [], "files": [], "skipped": []}) {
            return Err("remote deletion returned an invalid completion response".into());
        }
    }
    Ok(output)
}

pub(super) async fn delete_remote_tree(
    handle: Arc<tokio::sync::Mutex<SshBackendSession>>,
    path: &str,
) -> Result<(), String> {
    remote_safe_tree(handle, "delete", &[path.to_string()])
        .await
        .map(|_| ())
}

#[cfg(all(test, unix))]
#[path = "../../test/rust/portmate/unit/remote_safe_tree.rs"]
mod tests;
