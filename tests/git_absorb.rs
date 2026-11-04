mod common;
use common::*;

fn stage_blob(dir: &std::path::Path, mode: &str, path: &str, content: &[u8]) {
    let mut child = StdCommand::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args(["hash-object", "-w", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        child.stdin.as_mut().unwrap().write_all(content).unwrap();
    }
    drop(child.stdin.take());
    let blob = child.wait_with_output().unwrap();
    assert!(blob.status.success());
    let blob = String::from_utf8(blob.stdout).unwrap();
    assert!(
        git(
            dir,
            &["update-index", "--cacheinfo", mode, blob.trim(), path]
        )
        .status
        .success()
    );
}

#[test]
fn absorb_staged_adds_late_file_to_top_patch() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "old\n").unwrap();
    init_git_baseline(dir.path());

    q(dir.path())
        .args(["new", "change.diff"])
        .assert()
        .success();
    fs::write(dir.path().join("hello.txt"), "new\n").unwrap();
    assert!(git(dir.path(), &["add", "hello.txt"]).status.success());

    q(dir.path()).args(["absorb"]).assert().success();

    let patch = fs::read_to_string(dir.path().join("patches/change.diff")).unwrap();
    assert!(patch.contains("-old"));
    assert!(patch.contains("+new"));
    assert_eq!(
        String::from_utf8(git(dir.path(), &["diff", "--cached", "--name-only"]).stdout).unwrap(),
        ""
    );
}

#[test]
fn absorb_staged_flag_is_accepted_as_alias() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "old\n").unwrap();
    init_git_baseline(dir.path());

    q(dir.path())
        .args(["new", "change.diff"])
        .assert()
        .success();
    fs::write(dir.path().join("hello.txt"), "new\n").unwrap();
    assert!(git(dir.path(), &["add", "hello.txt"]).status.success());

    q(dir.path())
        .args(["absorb", "--staged"])
        .assert()
        .success();

    let patch = fs::read_to_string(dir.path().join("patches/change.diff")).unwrap();
    assert!(patch.contains("+new"));
}

#[test]
fn absorb_staged_uses_index_blob_not_unstaged_worktree() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "a\nb\nc\n").unwrap();
    init_git_baseline(dir.path());

    q(dir.path())
        .args(["new", "change.diff"])
        .assert()
        .success();
    fs::write(dir.path().join("hello.txt"), "A\nB\nc\n").unwrap();
    stage_blob(dir.path(), "100644", "hello.txt", b"A\nb\nc\n");

    q(dir.path()).args(["absorb"]).assert().success();

    let patch = fs::read_to_string(dir.path().join("patches/change.diff")).unwrap();
    assert!(patch.contains("+A"));
    assert!(!patch.contains("+B"));
    assert_eq!(
        fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
        "A\nB\nc\n"
    );
}

#[test]
fn absorb_staged_records_new_file() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("base.txt"), "base\n").unwrap();
    init_git_baseline(dir.path());

    q(dir.path()).args(["new", "add.diff"]).assert().success();
    fs::write(dir.path().join("created.txt"), "created\n").unwrap();
    assert!(git(dir.path(), &["add", "created.txt"]).status.success());

    q(dir.path()).args(["absorb"]).assert().success();

    let patch = fs::read_to_string(dir.path().join("patches/add.diff")).unwrap();
    assert!(patch.contains("--- /dev/null"));
    assert!(patch.contains("+created"));
    q(dir.path()).args(["pop"]).assert().success();
    assert!(!dir.path().join("created.txt").exists());
}

#[test]
fn absorb_staged_records_deleted_file() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("gone.txt"), "old\n").unwrap();
    init_git_baseline(dir.path());

    q(dir.path())
        .args(["new", "delete.diff"])
        .assert()
        .success();
    fs::remove_file(dir.path().join("gone.txt")).unwrap();
    assert!(git(dir.path(), &["add", "-u", "gone.txt"]).status.success());

    q(dir.path()).args(["absorb"]).assert().success();

    let patch = fs::read_to_string(dir.path().join("patches/delete.diff")).unwrap();
    assert!(patch.contains("-old"));
    assert!(patch.contains("+++ /dev/null"));
    q(dir.path()).args(["pop"]).assert().success();
    assert_eq!(
        fs::read_to_string(dir.path().join("gone.txt")).unwrap(),
        "old\n"
    );
}

#[test]
fn absorb_staged_preserves_existing_top_patch_backup() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "a\nb\n").unwrap();
    init_git_baseline(dir.path());

    q(dir.path())
        .args(["new", "change.diff"])
        .assert()
        .success();
    q(dir.path()).args(["add", "hello.txt"]).assert().success();
    fs::write(dir.path().join("hello.txt"), "A\nb\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();

    fs::write(dir.path().join("hello.txt"), "AA\nBB\n").unwrap();
    stage_blob(dir.path(), "100644", "hello.txt", b"AA\nb\n");

    q(dir.path()).args(["absorb"]).assert().success();

    let patch = fs::read_to_string(dir.path().join("patches/change.diff")).unwrap();
    assert!(patch.contains("-a"));
    assert!(patch.contains("+AA"));
    assert!(!patch.contains("+BB"));
    assert_eq!(
        fs::read_to_string(dir.path().join(".pc/change.diff/hello.txt")).unwrap(),
        "a\nb\n"
    );
}

#[test]
fn absorb_sync_git_requires_scratch_config() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "old\n").unwrap();
    init_git_baseline(dir.path());

    q(dir.path())
        .args(["new", "change.diff"])
        .assert()
        .success();
    fs::write(dir.path().join("hello.txt"), "new\n").unwrap();
    assert!(git(dir.path(), &["add", "hello.txt"]).status.success());

    q(dir.path())
        .args(["absorb", "--sync-git"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("q.scratch=true"));
}

#[test]
fn absorb_sync_git_rejects_no_sync_git_too() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "old\n").unwrap();
    init_git_baseline(dir.path());
    assert!(
        git(dir.path(), &["config", "q.scratch", "true"])
            .status
            .success()
    );
    q(dir.path())
        .args(["new", "change.diff"])
        .assert()
        .success();
    fs::write(dir.path().join("hello.txt"), "new\n").unwrap();
    assert!(git(dir.path(), &["add", "hello.txt"]).status.success());

    q(dir.path())
        .args(["absorb", "--sync-git", "--no-sync-git"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("mutually exclusive"));
}

#[test]
fn absorb_staged_refuses_renames_and_copies() {
    let rename = tempdir().unwrap();
    fs::write(rename.path().join("old.txt"), "old\n").unwrap();
    init_git_baseline(rename.path());
    q(rename.path())
        .args(["new", "rename.diff"])
        .assert()
        .success();
    assert!(
        git(rename.path(), &["mv", "old.txt", "new.txt"])
            .status
            .success()
    );

    q(rename.path())
        .args(["absorb"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("renames or copies"));

    let copy = tempdir().unwrap();
    fs::write(copy.path().join("old.txt"), "old\n").unwrap();
    init_git_baseline(copy.path());
    q(copy.path()).args(["new", "copy.diff"]).assert().success();
    fs::copy(copy.path().join("old.txt"), copy.path().join("copy.txt")).unwrap();
    assert!(git(copy.path(), &["add", "copy.txt"]).status.success());

    q(copy.path())
        .args(["absorb"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("renames or copies"));
}

#[test]
fn absorb_staged_reports_no_staged_changes() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "old\n").unwrap();
    init_git_baseline(dir.path());
    q(dir.path())
        .args(["new", "change.diff"])
        .assert()
        .success();

    q(dir.path())
        .args(["absorb"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No staged changes"));
}

#[test]
fn absorb_staged_requires_top_patch() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "old\n").unwrap();
    init_git_baseline(dir.path());
    q(dir.path())
        .args(["new", "change.diff"])
        .assert()
        .success();
    q(dir.path()).args(["pop"]).assert().success();
    fs::write(dir.path().join("hello.txt"), "new\n").unwrap();
    assert!(git(dir.path(), &["add", "hello.txt"]).status.success());

    q(dir.path())
        .args(["absorb"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No patches applied"));
}

#[test]
fn absorb_staged_requires_matching_git_and_quilt_roots() {
    let dir = tempdir().unwrap();
    let project = dir.path().join("project");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("hello.txt"), "old\n").unwrap();
    assert!(git(dir.path(), &["init"]).status.success());
    assert!(
        git(dir.path(), &["config", "user.email", "q@example.invalid"])
            .status
            .success()
    );
    assert!(
        git(dir.path(), &["config", "user.name", "q"])
            .status
            .success()
    );
    assert!(git(dir.path(), &["add", "."]).status.success());
    assert!(git(dir.path(), &["commit", "-m", "init"]).status.success());

    q(&project).args(["new", "change.diff"]).assert().success();
    fs::write(project.join("hello.txt"), "new\n").unwrap();
    assert!(
        git(dir.path(), &["add", "project/hello.txt"])
            .status
            .success()
    );

    q(&project)
        .args(["absorb"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not match quilt root"));
}

#[test]
fn absorb_staged_refuses_binary_diff() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("blob.bin"), b"old\0value\n").unwrap();
    init_git_baseline(dir.path());
    q(dir.path())
        .args(["new", "binary.diff"])
        .assert()
        .success();
    fs::write(dir.path().join("blob.bin"), b"new\0value\n").unwrap();
    assert!(git(dir.path(), &["add", "blob.bin"]).status.success());

    q(dir.path())
        .args(["absorb"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Diff failed on file 'blob.bin'"));
}

#[test]
fn scratch_init_creates_repo_ignores_pc_and_commits_base() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "base\n").unwrap();
    fs::create_dir(dir.path().join(".pc")).unwrap();
    fs::write(dir.path().join(".pc/cache"), "ignored\n").unwrap();

    q(dir.path()).args(["scratch", "init"]).assert().success();

    let gitignore = fs::read_to_string(dir.path().join(".gitignore")).unwrap();
    assert!(gitignore.lines().any(|line| line == ".pc"));
    assert_eq!(
        String::from_utf8(git(dir.path(), &["config", "--bool", "q.scratch"]).stdout).unwrap(),
        "true\n"
    );
    assert_eq!(
        String::from_utf8(git(dir.path(), &["status", "--porcelain"]).stdout).unwrap(),
        ""
    );
    assert_eq!(
        String::from_utf8(git(dir.path(), &["ls-files", ".pc/cache"]).stdout).unwrap(),
        ""
    );
    assert_eq!(
        String::from_utf8(git(dir.path(), &["show", "--format=%s", "--no-patch", "HEAD"]).stdout)
            .unwrap(),
        "base\n"
    );
}

#[test]
fn scratch_init_does_not_read_global_git_config() {
    let dir = tempdir().unwrap();
    let home = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "base\n").unwrap();
    fs::write(
        home.path().join(".gitconfig"),
        "[commit]\n\tgpgSign = true\n[user]\n\temail = global@example.invalid\n\tname = global\n",
    )
    .unwrap();

    q(dir.path())
        .env("HOME", home.path())
        .args(["scratch", "init"])
        .assert()
        .success();

    assert_eq!(
        String::from_utf8(
            git(
                dir.path(),
                &["show", "--format=%ae %an", "--no-patch", "HEAD"]
            )
            .stdout
        )
        .unwrap(),
        "q@example.invalid q\n"
    );
}

#[test]
fn scratch_init_existing_repo_requires_force() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "base\n").unwrap();
    init_git_baseline(dir.path());

    q(dir.path())
        .args(["scratch", "init"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("rerun with --force"));

    q(dir.path())
        .args(["scratch", "init", "--force"])
        .assert()
        .success();
    assert_eq!(
        String::from_utf8(git(dir.path(), &["config", "--bool", "q.scratch"]).stdout).unwrap(),
        "true\n"
    );
}

#[test]
fn absorb_staged_unstages_chmod_only_change() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().unwrap();
    fs::write(dir.path().join("script.sh"), "echo hi\n").unwrap();
    init_git_baseline(dir.path());
    q(dir.path()).args(["new", "mode.diff"]).assert().success();
    let mut perms = fs::metadata(dir.path().join("script.sh"))
        .unwrap()
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(dir.path().join("script.sh"), perms).unwrap();
    assert!(git(dir.path(), &["add", "script.sh"]).status.success());

    q(dir.path()).args(["absorb"]).assert().success();

    assert_eq!(
        String::from_utf8(git(dir.path(), &["diff", "--cached", "--name-only"]).stdout).unwrap(),
        ""
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("patches/mode.diff")).unwrap(),
        ""
    );
}

#[test]
fn absorb_sync_git_amends_unstaged_changes_too() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("staged.txt"), "old\n").unwrap();
    fs::write(dir.path().join("other.txt"), "old\n").unwrap();
    init_git_baseline(dir.path());
    assert!(
        git(dir.path(), &["config", "q.scratch", "true"])
            .status
            .success()
    );
    q(dir.path())
        .args(["new", "change.diff"])
        .assert()
        .success();
    fs::write(dir.path().join("staged.txt"), "new\n").unwrap();
    fs::write(dir.path().join("other.txt"), "unstaged\n").unwrap();
    assert!(git(dir.path(), &["add", "staged.txt"]).status.success());

    q(dir.path())
        .args(["absorb", "--sync-git"])
        .assert()
        .success();

    assert_eq!(
        String::from_utf8(git(dir.path(), &["status", "--porcelain"]).stdout).unwrap(),
        ""
    );
    assert_eq!(
        String::from_utf8(git(dir.path(), &["show", "HEAD:other.txt"]).stdout).unwrap(),
        "unstaged\n"
    );
    let patch = fs::read_to_string(dir.path().join("patches/change.diff")).unwrap();
    assert!(patch.contains("+new"));
    assert!(!patch.contains("unstaged"));
}

#[test]
fn absorb_staged_auto_syncs_when_scratch_config_enabled() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "old\n").unwrap();
    init_git_baseline(dir.path());
    assert!(
        git(dir.path(), &["config", "q.scratch", "true"])
            .status
            .success()
    );

    q(dir.path())
        .args(["new", "change.diff"])
        .assert()
        .success();
    fs::write(dir.path().join("hello.txt"), "new\n").unwrap();
    assert!(git(dir.path(), &["add", "hello.txt"]).status.success());

    q(dir.path()).args(["absorb"]).assert().success();

    assert_eq!(
        String::from_utf8(git(dir.path(), &["status", "--porcelain"]).stdout).unwrap(),
        ""
    );
    let patch = fs::read_to_string(dir.path().join("patches/change.diff")).unwrap();
    assert!(patch.contains("+new"));
}

#[test]
fn absorb_staged_no_sync_git_overrides_scratch_config() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "old\n").unwrap();
    init_git_baseline(dir.path());
    assert!(
        git(dir.path(), &["config", "q.scratch", "true"])
            .status
            .success()
    );

    q(dir.path())
        .args(["new", "change.diff"])
        .assert()
        .success();
    fs::write(dir.path().join("hello.txt"), "new\n").unwrap();
    assert!(git(dir.path(), &["add", "hello.txt"]).status.success());

    q(dir.path())
        .args(["absorb", "--no-sync-git"])
        .assert()
        .success();

    assert_eq!(
        String::from_utf8(git(dir.path(), &["diff", "--cached", "--name-only"]).stdout).unwrap(),
        ""
    );
    assert_ne!(
        String::from_utf8(git(dir.path(), &["status", "--porcelain"]).stdout).unwrap(),
        ""
    );
}

#[test]
fn absorb_staged_refuses_quilt_metadata_paths() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "old\n").unwrap();
    init_git_baseline(dir.path());

    q(dir.path())
        .args(["new", "change.diff"])
        .assert()
        .success();
    assert!(git(dir.path(), &["add", "patches/series"]).status.success());

    q(dir.path())
        .args(["absorb"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("below quilt metadata"));
}

#[test]
fn absorb_sync_git_amends_scratch_commit() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "old\n").unwrap();
    init_git_baseline(dir.path());
    assert!(
        git(dir.path(), &["config", "q.scratch", "true"])
            .status
            .success()
    );

    q(dir.path())
        .args(["new", "change.diff"])
        .assert()
        .success();
    fs::write(dir.path().join("hello.txt"), "new\n").unwrap();
    assert!(git(dir.path(), &["add", "hello.txt"]).status.success());

    q(dir.path())
        .args(["absorb", "--sync-git"])
        .assert()
        .success();

    let status = String::from_utf8(git(dir.path(), &["status", "--porcelain"]).stdout).unwrap();
    assert_eq!(status, "");
    let patch = fs::read_to_string(dir.path().join("patches/change.diff")).unwrap();
    assert!(patch.contains("+new"));
}
