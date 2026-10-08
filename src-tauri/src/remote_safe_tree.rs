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
mod tests {
    use super::*;

    #[test]
    fn remote_tree_parent_replacement_does_not_delete_or_plan_outside_files() {
        let harness = r#"
import json, os, pathlib, sys, tempfile
namespace = {'__name__': 'tree_regression'}
exec(sys.argv[1], namespace)
for action in ('plan', 'delete'):
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp)
        selected, saved, outside = root/'selected', root/'saved', root/'outside'
        selected.mkdir(); outside.mkdir()
        (selected/'victim').write_text('selected')
        (outside/'victim').write_text('outside')
        def race(path):
            if path == str(selected):
                selected.rename(saved)
                selected.symlink_to(outside, target_is_directory=True)
        try:
            namespace['run'](action, [str(selected)], race)
            raise AssertionError('changed directory was accepted')
        except RuntimeError:
            pass
        assert (outside/'victim').read_text() == 'outside'
with tempfile.TemporaryDirectory() as tmp:
    selected = pathlib.Path(tmp)/'normal'; selected.mkdir()
    (selected/'payload').write_text('data')
    assert namespace['run']('plan', [str(selected)])['files'][0]['size'] == 4
    namespace['run']('delete', [str(selected)])
    assert not selected.exists()
"#;
        let result = Command::new("python3")
            .args(["-I", "-c", harness, REMOTE_SAFE_TREE_SCRIPT])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
}
