mod common;
use common::*;

#[test]
fn header_replace_and_append_preserve_patch_body() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("foo"), "foo\n").unwrap();
    q(dir.path()).args(["new", "patch"]).assert().success();
    q(dir.path()).args(["add", "foo"]).assert().success();
    fs::write(dir.path().join("foo"), "bar\n").unwrap();
    q(dir.path())
        .args(["refresh", "-p", "ab"])
        .assert()
        .success();

    q(dir.path())
        .args(["header", "-r"])
        .write_stdin("Header\n")
        .assert()
        .success();
    q(dir.path())
        .args(["header", "-a"])
        .write_stdin("Appended")
        .assert()
        .success();

    q(dir.path())
        .args(["header"])
        .assert()
        .success()
        .stdout("Header\nAppended\n");
    let patch = fs::read_to_string(dir.path().join("patches/patch")).unwrap();
    assert!(patch.contains("Index: b/foo"));
    assert!(patch.contains("-foo"));
    assert!(patch.contains("+bar"));
}

#[test]
fn header_edit_and_strip_diffstat_match_quilt_behavior() {
    let dir = tempdir().unwrap();
    let editor = dir.path().join("editor");
    fs::write(&editor, "#!/bin/sh\nprintf 'Edited header\\n' > \"$1\"\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&editor, fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(dir.path().join("foo"), "foo\n").unwrap();
    q(dir.path()).args(["new", "patch"]).assert().success();
    q(dir.path()).args(["add", "foo"]).assert().success();
    fs::write(dir.path().join("foo"), "bar\n").unwrap();
    q(dir.path())
        .args(["refresh", "--diffstat", "-p", "ab"])
        .assert()
        .success();

    q(dir.path())
        .args(["header", "--strip-diffstat"])
        .assert()
        .success()
        .stdout("---\n\n");
    q(dir.path())
        .env("EDITOR", &editor)
        .args(["header", "-e"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Replaced header of patch patch"));

    let patch = fs::read_to_string(dir.path().join("patches/patch")).unwrap();
    assert!(patch.starts_with("Edited header\nIndex: b/foo\n"));
    assert!(!patch.contains("1 file changed"));
}

#[test]
fn refresh_diffstat_adds_header_section() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("f"), "old\n").unwrap();
    q(dir.path()).args(["new", "p.diff"]).assert().success();
    q(dir.path()).args(["add", "f"]).assert().success();
    fs::write(dir.path().join("f"), "new\n").unwrap();

    q(dir.path())
        .args(["refresh", "--diffstat"])
        .assert()
        .success();

    let patch = fs::read_to_string(dir.path().join("patches/p.diff")).unwrap();
    assert!(patch.starts_with("---\n f |    2 +-\n"));
    assert!(patch.contains("1 file changed, 1 insertion(+), 1 deletion(-)\n\nIndex:"));
}
