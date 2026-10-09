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
