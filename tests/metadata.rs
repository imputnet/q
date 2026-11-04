mod common;
use common::*;

#[test]
fn q_metadata_is_kept_outside_patch_backup_dirs() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("f"), "old\n").unwrap();
    q(dir.path()).args(["new", "p.diff"]).assert().success();
    q(dir.path()).args(["add", "f"]).assert().success();
    fs::write(dir.path().join("f"), "new\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();

    assert!(dir.path().join(".pc/.q/p.diff/expected/f").exists());
    assert!(dir.path().join(".pc/.q/p.diff/deleted").exists());
    assert!(!dir.path().join(".pc/p.diff/.q").exists());
}

#[test]
fn refresh_records_expected_files_deleted_list_and_patch_text() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("kept"), "old\n").unwrap();
    fs::write(dir.path().join("gone"), "old\n").unwrap();
    q(dir.path()).args(["new", "p.diff"]).assert().success();
    q(dir.path())
        .args(["add", "kept", "gone"])
        .assert()
        .success();
    fs::write(dir.path().join("kept"), "new\n").unwrap();
    fs::remove_file(dir.path().join("gone")).unwrap();

    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();

    assert_eq!(
        fs::read_to_string(dir.path().join(".pc/.q/p.diff/expected/kept")).unwrap(),
        "new\n"
    );
    assert!(!dir.path().join(".pc/.q/p.diff/expected/gone").exists());
    assert_eq!(
        fs::read_to_string(dir.path().join(".pc/.q/p.diff/deleted")).unwrap(),
        "gone\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join(".pc/.q/p.diff/files")).unwrap(),
        "gone\nkept\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join(".pc/.q/p.diff/patch")).unwrap(),
        fs::read_to_string(dir.path().join("patches/p.diff")).unwrap()
    );
}

#[test]
fn changed_patch_file_invalidates_expected_state_on_pop() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("f"), "old\n").unwrap();
    q(dir.path()).args(["new", "p.diff"]).assert().success();
    q(dir.path()).args(["add", "f"]).assert().success();
    fs::write(dir.path().join("f"), "new\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();

    let patch = dir.path().join("patches/p.diff");
    let edited = fs::read_to_string(&patch)
        .unwrap()
        .replace("+new", "+other");
    fs::write(&patch, edited).unwrap();

    q(dir.path())
        .args(["pop"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Patch p.diff does not remove cleanly",
        ));
    assert_eq!(fs::read_to_string(dir.path().join("f")).unwrap(), "new\n");
}

#[test]
fn refresh_fork_skips_expected_deleted_files_with_spaces() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("gone file"), "old\n").unwrap();
    fs::write(dir.path().join("kept file"), "old\n").unwrap();

    q(dir.path()).args(["new", "p.diff"]).assert().success();
    q(dir.path())
        .args(["add", "gone file", "kept file"])
        .assert()
        .success();
    fs::remove_file(dir.path().join("gone file")).unwrap();
    fs::write(dir.path().join("kept file"), "mid\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();

    let patch_text = fs::read_to_string(dir.path().join("patches/p.diff")).unwrap();
    let recorded_text = fs::read_to_string(dir.path().join(".pc/.q/p.diff/patch")).unwrap();
    assert_eq!(recorded_text, patch_text);
    assert_eq!(
        fs::read_to_string(dir.path().join(".pc/.q/p.diff/deleted")).unwrap(),
        "gone file\n"
    );

    fs::write(dir.path().join("kept file"), "new\n").unwrap();
    q(dir.path())
        .args(["refresh", "-zfork.diff", "--no-timestamps"])
        .assert()
        .success();

    let fork = fs::read_to_string(dir.path().join("patches/fork.diff")).unwrap();
    assert!(fork.contains("kept file"));
    assert!(!fork.contains("gone file"));
}
