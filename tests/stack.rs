mod common;
use common::*;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[test]
fn create_refresh_pop_and_push_patch() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "old\n").unwrap();

    q(dir.path())
        .args(["new", "change.diff"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Patch change.diff is now on top"));
    q(dir.path())
        .args(["add", "hello.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("File hello.txt added"));

    fs::write(dir.path().join("hello.txt"), "new\n").unwrap();
    q(dir.path()).args(["refresh"]).assert().success();

    let patch = fs::read_to_string(dir.path().join("patches/change.diff")).unwrap();
    assert!(patch.contains("-old"));
    assert!(patch.contains("+new"));

    q(dir.path()).args(["pop"]).assert().success();
    assert_eq!(
        fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
        "old\n"
    );

    q(dir.path()).args(["push"]).assert().success();
    assert_eq!(
        fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
        "new\n"
    );
}

#[test]
fn add_new_records_existing_file_as_created() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("created.txt"), "created\n").unwrap();

    q(dir.path()).args(["new", "add.diff"]).assert().success();
    q(dir.path())
        .args(["add", "--new", "created.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("File created.txt added"));

    q(dir.path())
        .args(["diff", "--no-timestamps", "-p", "ab"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--- /dev/null\n+++ b/created.txt"))
        .stdout(predicate::str::contains("+created"));
}

#[test]
fn late_plain_add_uses_edited_file_as_backup() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "old\n").unwrap();

    q(dir.path()).args(["new", "late.diff"]).assert().success();
    fs::write(dir.path().join("hello.txt"), "new\n").unwrap();
    q(dir.path()).args(["add", "hello.txt"]).assert().success();

    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Nothing in patch late.diff"));
    assert_eq!(
        fs::read_to_string(dir.path().join(".pc/late.diff/hello.txt")).unwrap(),
        "new\n"
    );
}

#[test]
fn push_applies_missing_patch_file_as_empty_patch() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("patches")).unwrap();
    fs::write(dir.path().join("patches/series"), "missing.diff\n").unwrap();

    q(dir.path())
        .args(["push"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Patch missing.diff does not exist; applied empty patch",
        ));
    assert_eq!(
        fs::read_to_string(dir.path().join(".pc/applied-patches")).unwrap(),
        "missing.diff\n"
    );
}

#[test]
fn push_and_pop_empty_patch_file() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("patches")).unwrap();
    fs::write(dir.path().join("patches/series"), "empty.diff\n").unwrap();
    fs::write(dir.path().join("patches/empty.diff"), "Header only\n").unwrap();

    q(dir.path())
        .args(["push"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Patch empty.diff appears to be empty; applied",
        ));
    q(dir.path())
        .args(["pop"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Patch empty.diff appears to be empty, removing",
        ));
}

#[test]
fn pop_refuses_to_discard_pending_edits() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "old\n").unwrap();

    q(dir.path())
        .args(["new", "change.diff"])
        .assert()
        .success();
    q(dir.path()).args(["add", "hello.txt"]).assert().success();
    fs::write(dir.path().join("hello.txt"), "new\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();

    fs::write(dir.path().join("hello.txt"), "local edit\n").unwrap();

    q(dir.path())
        .args(["pop"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Patch change.diff does not remove cleanly",
        ));
    assert_eq!(
        fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
        "local edit\n"
    );
    q(dir.path())
        .args(["top"])
        .assert()
        .success()
        .stdout("change.diff\n");
}

#[test]
fn pop_force_discards_pending_edits() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "old\n").unwrap();

    q(dir.path())
        .args(["new", "change.diff"])
        .assert()
        .success();
    q(dir.path()).args(["add", "hello.txt"]).assert().success();
    fs::write(dir.path().join("hello.txt"), "new\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();

    fs::write(dir.path().join("hello.txt"), "local edit\n").unwrap();

    q(dir.path()).args(["pop", "-f"]).assert().success();
    assert_eq!(
        fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
        "old\n"
    );
}

#[test]
fn refresh_force_keeps_preserved_file_in_original_patch_order() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("file_a"), "base a\n").unwrap();
    fs::write(dir.path().join("file_b"), "base b\n").unwrap();
    fs::write(dir.path().join("file_c"), "base c\n").unwrap();

    q(dir.path()).args(["new", "p1.diff"]).assert().success();
    q(dir.path())
        .args(["add", "file_a", "file_b", "file_c"])
        .assert()
        .success();
    fs::write(dir.path().join("file_a"), "p1 a\n").unwrap();
    fs::write(dir.path().join("file_b"), "p1 b\n").unwrap();
    fs::write(dir.path().join("file_c"), "p1 c\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();

    q(dir.path()).args(["new", "p2.diff"]).assert().success();
    q(dir.path()).args(["add", "file_b"]).assert().success();
    fs::write(dir.path().join("file_b"), "p2 b\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();

    fs::write(dir.path().join("file_a"), "updated p1 a\n").unwrap();
    fs::write(dir.path().join("file_c"), "updated p1 c\n").unwrap();
    q(dir.path())
        .args(["refresh", "-f", "--no-timestamps", "p1.diff"])
        .assert()
        .success();

    let patch = fs::read_to_string(dir.path().join("patches/p1.diff")).unwrap();
    let order: Vec<_> = patch
        .lines()
        .filter_map(|line| line.strip_prefix("Index: "))
        .map(|path| {
            std::path::Path::new(path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(order, ["file_a", "file_b", "file_c"]);
    assert!(patch.contains("+p1 b"));
    assert!(!patch.contains("+p2 b"));
}

#[test]
fn push_force_conflict_refresh_keeps_fixed_file_in_original_patch_order() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("file_a"), "base a\n").unwrap();
    fs::write(dir.path().join("file_b"), "base b\n").unwrap();
    fs::write(dir.path().join("file_c"), "base c\n").unwrap();

    q(dir.path()).args(["new", "p1.diff"]).assert().success();
    q(dir.path())
        .args(["add", "file_a", "file_b", "file_c"])
        .assert()
        .success();
    fs::write(dir.path().join("file_a"), "p1 a\n").unwrap();
    fs::write(dir.path().join("file_b"), "p1 b\n").unwrap();
    fs::write(dir.path().join("file_c"), "p1 c\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();

    q(dir.path()).args(["new", "p2.diff"]).assert().success();
    q(dir.path()).args(["add", "file_b"]).assert().success();
    fs::write(dir.path().join("file_b"), "p2 b\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();

    q(dir.path()).args(["pop", "-a"]).assert().success();
    fs::write(dir.path().join("file_b"), "upstream b\n").unwrap();
    q(dir.path())
        .args(["push", "-f", "p1.diff"])
        .assert()
        .failure();
    fs::write(dir.path().join("file_b"), "p1 b\n").unwrap();
    q(dir.path())
        .args(["push", "p2.diff"])
        .assert()
        .failure()
        .stdout(predicate::str::contains(
            "The topmost patch p1.diff needs to be refreshed first.",
        ));
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();
    q(dir.path()).args(["push", "p2.diff"]).assert().success();

    let patch = fs::read_to_string(dir.path().join("patches/p1.diff")).unwrap();
    let order: Vec<_> = patch
        .lines()
        .filter_map(|line| line.strip_prefix("Index: "))
        .map(|path| {
            std::path::Path::new(path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(order, ["file_a", "file_b", "file_c"]);
}

#[test]
fn pop_refuses_after_patch_file_changes() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "old value\n").unwrap();

    q(dir.path())
        .args(["new", "change.diff"])
        .assert()
        .success();
    q(dir.path()).args(["add", "hello.txt"]).assert().success();
    fs::write(dir.path().join("hello.txt"), "new value\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();

    let patch = dir.path().join("patches/change.diff");
    let text = fs::read_to_string(&patch).unwrap().replace(' ', "_");
    fs::write(patch, text).unwrap();

    q(dir.path())
        .args(["pop"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Patch change.diff does not remove cleanly",
        ));
    assert_eq!(
        fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
        "new value\n"
    );
}

#[test]
fn pop_accepts_unique_patch_basename() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "old\n").unwrap();

    q(dir.path())
        .args(["new", "a/b/c/dddd.patch"])
        .assert()
        .success();
    q(dir.path()).args(["add", "hello.txt"]).assert().success();
    fs::write(dir.path().join("hello.txt"), "new\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();
    q(dir.path())
        .args(["new", "later.patch"])
        .assert()
        .success();
    q(dir.path()).args(["add", "hello.txt"]).assert().success();
    fs::write(dir.path().join("hello.txt"), "later\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();

    q(dir.path()).args(["pop", "dddd"]).assert().success();
    assert_eq!(
        fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
        "new\n"
    );
    q(dir.path())
        .args(["top"])
        .assert()
        .success()
        .stdout("a/b/c/dddd.patch\n");

    q(dir.path()).args(["push"]).assert().success();
    q(dir.path()).args(["pop", "dddd.patch"]).assert().success();
    q(dir.path())
        .args(["top"])
        .assert()
        .success()
        .stdout("a/b/c/dddd.patch\n");
}

#[test]
fn patch_basename_match_must_be_unambiguous() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "old\n").unwrap();

    q(dir.path())
        .args(["new", "a/dddd.patch"])
        .assert()
        .success();
    q(dir.path()).args(["add", "hello.txt"]).assert().success();
    fs::write(dir.path().join("hello.txt"), "one\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();

    q(dir.path())
        .args(["new", "b/dddd.patch"])
        .assert()
        .success();
    q(dir.path()).args(["add", "hello.txt"]).assert().success();
    fs::write(dir.path().join("hello.txt"), "two\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();
    q(dir.path())
        .args(["new", "later.patch"])
        .assert()
        .success();

    q(dir.path())
        .args(["pop", "dddd"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Patch dddd is ambiguous"));
}

#[test]
fn remove_drops_file_from_patch_tracking() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("keep.txt"), "old keep\n").unwrap();
    fs::write(dir.path().join("drop.txt"), "old drop\n").unwrap();

    q(dir.path())
        .args(["new", "change.diff"])
        .assert()
        .success();
    q(dir.path())
        .args(["add", "keep.txt", "drop.txt"])
        .assert()
        .success();
    fs::write(dir.path().join("keep.txt"), "new keep\n").unwrap();
    fs::write(dir.path().join("drop.txt"), "new drop\n").unwrap();
    q(dir.path())
        .args(["remove", "drop.txt"])
        .assert()
        .success();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();

    let patch = fs::read_to_string(dir.path().join("patches/change.diff")).unwrap();
    assert!(patch.contains("keep.txt"));
    assert!(!patch.contains("drop.txt"));
    assert!(!dir.path().join(".pc/change.diff/drop.txt").exists());
}

#[test]
fn rename_moves_patch_file_backup_and_applied_state() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "old\n").unwrap();

    q(dir.path()).args(["new", "old.diff"]).assert().success();
    q(dir.path()).args(["add", "hello.txt"]).assert().success();
    fs::write(dir.path().join("hello.txt"), "new\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();

    q(dir.path())
        .args(["rename", "nested/new.diff"])
        .assert()
        .success();

    assert!(!dir.path().join("patches/old.diff").exists());
    assert!(dir.path().join("patches/nested/new.diff").exists());
    assert!(!dir.path().join(".pc/old.diff").exists());
    assert!(dir.path().join(".pc/nested/new.diff/hello.txt").exists());
    assert_eq!(
        fs::read_to_string(dir.path().join(".pc/applied-patches")).unwrap(),
        "nested/new.diff\n"
    );
    assert_eq!(
        fs::read_to_string(dir.path().join("patches/series")).unwrap(),
        "nested/new.diff\n"
    );
}

#[test]
fn delete_removes_top_patch_and_restores_files() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "old\n").unwrap();

    q(dir.path())
        .args(["new", "delete.diff"])
        .assert()
        .success();
    q(dir.path()).args(["add", "hello.txt"]).assert().success();
    fs::write(dir.path().join("hello.txt"), "new\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();

    q(dir.path())
        .args(["delete", "-r"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed patch delete.diff"));

    assert_eq!(
        fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
        "old\n"
    );
    assert!(!dir.path().join("patches/delete.diff").exists());
    assert_eq!(
        fs::read_to_string(dir.path().join("patches/series")).unwrap(),
        ""
    );
    assert!(!dir.path().join(".pc/applied-patches").exists());
}

#[test]
fn push_forwards_fuzz_to_patch() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("patches")).unwrap();
    fs::write(dir.path().join("file"), "1\nx\n3\n4\n5\n").unwrap();
    fs::write(dir.path().join("patches/series"), "p.diff -p0\n").unwrap();
    fs::write(
        dir.path().join("patches/p.diff"),
        "--- file\n+++ file\n@@ -1,5 +1,5 @@\n 1\n 2\n-3\n+THREE\n 4\n 5\n",
    )
    .unwrap();

    q(dir.path()).args(["push", "--fuzz=0"]).assert().failure();
    let _ = fs::remove_dir_all(dir.path().join(".pc"));

    q(dir.path()).args(["push", "--fuzz=2"]).assert().success();
    assert_eq!(
        fs::read_to_string(dir.path().join("file")).unwrap(),
        "1\nx\nTHREE\n4\n5\n"
    );
}

#[test]
fn push_color_always_colors_patch_diagnostics() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("patches")).unwrap();
    fs::write(dir.path().join("file"), "1\nx\n3\n4\n5\n").unwrap();
    fs::write(dir.path().join("patches/series"), "p.diff -p0\n").unwrap();
    fs::write(
        dir.path().join("patches/p.diff"),
        "--- file\n+++ file\n@@ -1,5 +1,5 @@\n 1\n 2\n-3\n+THREE\n 4\n 5\n",
    )
    .unwrap();

    q(dir.path())
        .env("QUILT_COLORS", "patch_fail=41:clear=0")
        .env("QUILT_PUSH_ARGS", "--color=always")
        .args(["push", "--fuzz=0"])
        .assert()
        .failure()
        .stdout(predicate::str::contains(
            "\u{1b}[41mHunk #1 FAILED at 1.\u{1b}[0m",
        ));
    let _ = fs::remove_dir_all(dir.path().join(".pc"));

    q(dir.path())
        .env("QUILT_COLORS", "patch_fuzz=44:clear=0")
        .args(["push", "--color=always", "--fuzz=2"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\u{1b}[44mHunk #1 succeeded"));
}

#[test]
fn push_refresh_honors_refresh_environment_args() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("a"), "a\n").unwrap();
    q(dir.path()).args(["new", "a.patch"]).assert().success();
    q(dir.path()).args(["add", "a"]).assert().success();
    fs::write(dir.path().join("a"), "A\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();
    q(dir.path()).args(["pop"]).assert().success();

    q(dir.path())
        .env("QUILT_REFRESH_ARGS", "--no-timestamps")
        .args(["push", "--refresh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Patch a.patch is unchanged"));
}

#[test]
fn p0_deleted_file_diff_uses_file_label() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("file"), "file\n").unwrap();
    q(dir.path())
        .args(["new", "remove.diff"])
        .assert()
        .success();
    q(dir.path()).args(["add", "file"]).assert().success();
    fs::remove_file(dir.path().join("file")).unwrap();

    q(dir.path())
        .env("QUILT_NO_DIFF_TIMESTAMPS", "1")
        .args(["diff", "-p0"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--- file\n+++ /dev/null"));
}

#[cfg(unix)]
#[test]
fn patch_environment_options_are_honored() {
    let dir = tempdir().unwrap();
    let (tools, path) = poisoned_patch_path();
    let quiltrc = tools.path().join("quiltrc");
    fs::write(&quiltrc, "").unwrap();
    fs::create_dir(dir.path().join("patches")).unwrap();
    fs::write(dir.path().join("f"), "a b\n").unwrap();
    fs::write(dir.path().join("patches/series"), "p.diff\n").unwrap();
    fs::write(
        dir.path().join("patches/p.diff"),
        "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a  b\n+c d\n",
    )
    .unwrap();
    q(dir.path())
        .env("PATH", path)
        .env("QUILTRC", quiltrc)
        .env("QUILT_PATCH_OPTS", "--ignore-whitespace")
        .args(["push"])
        .assert()
        .success();
    assert_eq!(fs::read_to_string(dir.path().join("f")).unwrap(), "c d\n");
}

#[cfg(unix)]
#[test]
fn fold_uses_fast_path_for_patch_environment_options() {
    let dir = tempdir().unwrap();
    let (tools, path) = poisoned_patch_path();
    let quiltrc = tools.path().join("quiltrc");
    fs::write(&quiltrc, "").unwrap();
    fs::write(dir.path().join("f"), "a b\n").unwrap();
    q(dir.path()).args(["new", "p.diff"]).assert().success();
    q(dir.path()).args(["add", "f"]).assert().success();

    q(dir.path())
        .env("PATH", path)
        .env("QUILTRC", quiltrc)
        .env("QUILT_PATCH_OPTS", "--ignore-whitespace")
        .write_stdin("--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a  b\n+c d\n")
        .args(["fold"])
        .assert()
        .success();
    assert_eq!(fs::read_to_string(dir.path().join("f")).unwrap(), "c d\n");
}

#[test]
fn failed_fold_keeps_top_patch_changes() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("f"), "one\ntwo\n").unwrap();
    fs::write(dir.path().join("g"), "x\n").unwrap();
    q(dir.path()).args(["new", "top.diff"]).assert().success();
    q(dir.path()).args(["add", "f"]).assert().success();
    fs::write(dir.path().join("f"), "ONE\ntwo\n").unwrap();
    q(dir.path()).args(["refresh"]).assert().success();

    // The fold patches f, then fails on g. Like quilt, it must roll back to
    // the state before the fold, not before the top patch, and must not add
    // g to the top patch.
    q(dir.path())
        .write_stdin(
            "--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@\n ONE\n-two\n+TWO\n\
             --- a/g\n+++ b/g\n@@ -1 +1 @@\n-nomatch\n+y\n",
        )
        .args(["fold"])
        .assert()
        .failure();
    assert_eq!(
        fs::read_to_string(dir.path().join("f")).unwrap(),
        "ONE\ntwo\n"
    );
    assert_eq!(fs::read_to_string(dir.path().join("g")).unwrap(), "x\n");
    q(dir.path())
        .args(["files"])
        .assert()
        .success()
        .stdout("f\n");
}

#[cfg(unix)]
fn poisoned_patch_path() -> (tempfile::TempDir, std::ffi::OsString) {
    let tools = tempdir().unwrap();
    for program in ["patch", "gpatch"] {
        let patch = tools.path().join(program);
        fs::write(
            &patch,
            "#!/bin/sh\necho subprocess patch path was used >&2\nexit 99\n",
        )
        .unwrap();
        let mut permissions = fs::metadata(&patch).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&patch, permissions).unwrap();
    }
    let path = std::env::var_os("PATH")
        .map(|current| {
            let mut paths = vec![tools.path().to_path_buf()];
            paths.extend(std::env::split_paths(&current));
            std::env::join_paths(paths).unwrap()
        })
        .unwrap_or_else(|| tools.path().as_os_str().to_owned());
    (tools, path)
}

#[test]
fn series_edits_keep_comments_and_blank_lines() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("patches")).unwrap();
    fs::write(
        dir.path().join("patches/series"),
        "# upstream\na.diff\n\n# local\nb.diff -p0 # keep\n",
    )
    .unwrap();
    q(dir.path()).args(["new", "c.diff"]).assert().success();
    q(dir.path()).args(["rename", "d.diff"]).assert().success();
    assert_eq!(
        fs::read_to_string(dir.path().join("patches/series")).unwrap(),
        "# upstream\nd.diff\na.diff\n\n# local\nb.diff -p0 # keep\n"
    );
    q(dir.path()).args(["delete", "d.diff"]).assert().success();
    assert_eq!(
        fs::read_to_string(dir.path().join("patches/series")).unwrap(),
        "# upstream\na.diff\n\n# local\nb.diff -p0 # keep\n"
    );
}

#[test]
fn fork_of_unrefreshed_patch() {
    let dir = tempdir().unwrap();
    q(dir.path()).args(["new", "a.diff"]).assert().success();
    q(dir.path())
        .args(["fork"])
        .assert()
        .success()
        .stdout("Fork of patch a.diff created as a-2.diff\n");
    q(dir.path())
        .args(["series"])
        .assert()
        .success()
        .stdout("a-2.diff\n");
}

#[test]
fn diff_rejects_unsupported_strip_level() {
    let dir = tempdir().unwrap();
    q(dir.path()).args(["new", "a.diff"]).assert().success();
    q(dir.path())
        .args(["diff", "-p", "2"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Cannot diff patches with -p2, please specify -p0, -p1, or -pab instead",
        ));
}

#[test]
fn refresh_records_strip_level_in_series() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("f"), "a\n").unwrap();
    q(dir.path())
        .args(["new", "-p0", "a.diff"])
        .assert()
        .success();
    q(dir.path()).args(["add", "f"]).assert().success();
    fs::write(dir.path().join("f"), "b\n").unwrap();
    q(dir.path())
        .args(["refresh", "-p", "ab"])
        .assert()
        .success();
    assert_eq!(
        fs::read_to_string(dir.path().join("patches/series")).unwrap(),
        "a.diff\n"
    );
    q(dir.path()).args(["pop"]).assert().success();
    q(dir.path()).args(["push"]).assert().success();
}

/// Once a patch names a file, refresh and `files` take its name from the
/// patch, which must keep `/` on Windows too.
#[test]
fn refresh_keeps_slashes_in_names_from_the_old_patch() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("dir/sub")).unwrap();
    fs::write(dir.path().join("dir/sub/file"), "one\n").unwrap();
    q(dir.path()).args(["new", "p.diff"]).assert().success();
    q(dir.path())
        .args(["add", "dir/sub/file"])
        .assert()
        .success();
    for text in ["two\n", "three\n"] {
        fs::write(dir.path().join("dir/sub/file"), text).unwrap();
        q(dir.path())
            .args(["refresh", "-p", "ab", "--no-timestamps"])
            .assert()
            .success();
    }
    let patch = fs::read_to_string(dir.path().join("patches/p.diff")).unwrap();
    assert!(patch.starts_with("Index: b/dir/sub/file\n"), "{patch}");
    assert!(
        patch.contains("\n--- a/dir/sub/file\n+++ b/dir/sub/file\n"),
        "{patch}"
    );

    q(dir.path()).args(["pop"]).assert().success();
    q(dir.path())
        .args(["files", "p.diff"])
        .assert()
        .success()
        .stdout("dir/sub/file\n");
}

#[test]
fn import_rejects_paths_without_a_file_name() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("patches")).unwrap();
    fs::create_dir_all(dir.path().join("sub")).unwrap();
    q(&dir.path().join("sub"))
        .args(["import", ".."])
        .assert()
        .failure()
        .stderr(predicate::str::contains("panicked").not());
}

/// Two patches over `file_a`..`file_c` where p2 also changes `file_b`, built
/// with `-p ab --no-index` like a stack that keeps no `Index:` lines.
fn shadowed_stack_without_index() -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    for name in ["file_a", "file_b", "file_c"] {
        fs::write(dir.path().join(name), format!("base {name}\n")).unwrap();
    }
    q(dir.path()).args(["new", "p1.diff"]).assert().success();
    q(dir.path())
        .args(["add", "file_a", "file_b", "file_c"])
        .assert()
        .success();
    for name in ["file_a", "file_b", "file_c"] {
        fs::write(dir.path().join(name), format!("p1 {name}\n")).unwrap();
    }
    q(dir.path())
        .args(["refresh", "-p", "ab", "--no-index", "--no-timestamps"])
        .assert()
        .success();
    q(dir.path()).args(["new", "p2.diff"]).assert().success();
    q(dir.path()).args(["add", "file_b"]).assert().success();
    fs::write(dir.path().join("file_b"), "p2 file_b\n").unwrap();
    q(dir.path())
        .args(["refresh", "-p", "ab", "--no-index", "--no-timestamps"])
        .assert()
        .success();
    dir
}

#[test]
fn refresh_force_without_index_lines_keeps_each_file_once() {
    let dir = shadowed_stack_without_index();
    fs::write(dir.path().join("file_a"), "updated p1 file_a\n").unwrap();
    q(dir.path())
        .args([
            "refresh",
            "-f",
            "-p",
            "ab",
            "--no-index",
            "--no-timestamps",
            "p1.diff",
        ])
        .assert()
        .success();

    let patch = fs::read_to_string(dir.path().join("patches/p1.diff")).unwrap();
    let files: Vec<_> = patch
        .lines()
        .filter_map(|line| line.strip_prefix("+++ b/"))
        .collect();
    assert_eq!(files, ["file_a", "file_b", "file_c"], "{patch}");
    assert!(patch.contains("+updated p1 file_a"), "{patch}");
    assert!(patch.contains("+p1 file_b"), "{patch}");
    assert!(!patch.contains("+p2 file_b"), "{patch}");
}

#[test]
fn diff_of_lower_patch_takes_shadowed_files_from_next_patch_backup() {
    let dir = shadowed_stack_without_index();
    // A stack pushed without q's metadata, as by GNU quilt.
    let _ = fs::remove_dir_all(dir.path().join(".pc/.q"));

    q(dir.path())
        .args([
            "diff",
            "-p",
            "ab",
            "--no-index",
            "--no-timestamps",
            "-P",
            "p1.diff",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("+p1 file_b"))
        .stdout(predicate::str::contains("p2 file_b").not());
    q(dir.path())
        .args([
            "diff",
            "-p",
            "ab",
            "--no-index",
            "--no-timestamps",
            "--combine",
            "-",
            "-P",
            "p1.diff",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("+p1 file_b"))
        .stdout(predicate::str::contains("p2 file_b").not());
}

#[test]
fn diff_z_ignores_expected_state_recorded_for_other_patch_text() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("file"), "base\n").unwrap();
    q(dir.path()).args(["new", "p1.diff"]).assert().success();
    q(dir.path()).args(["add", "file"]).assert().success();
    fs::write(dir.path().join("file"), "p1\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();
    let committed = fs::read_to_string(dir.path().join("patches/p1.diff")).unwrap();

    // Refresh work in progress, then put the committed patch text back, as
    // checking out the patch file from version control does.
    fs::write(dir.path().join("file"), "wip\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();
    fs::write(dir.path().join("patches/p1.diff"), &committed).unwrap();

    q(dir.path())
        .args(["diff", "-z", "--no-timestamps"])
        .assert()
        .success()
        .stdout(predicate::str::contains("-p1\n+wip\n"));
    q(dir.path())
        .args(["pop"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not remove cleanly"));
}

/// One patch changing `hello.txt` from "old" to "new", refreshed and applied.
fn applied_hello_patch() -> tempfile::TempDir {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "old\n").unwrap();
    q(dir.path())
        .args(["new", "change.diff"])
        .assert()
        .success();
    q(dir.path()).args(["add", "hello.txt"]).assert().success();
    fs::write(dir.path().join("hello.txt"), "new\n").unwrap();
    q(dir.path()).args(["refresh"]).assert().success();
    dir
}

#[test]
fn pop_notices_same_size_edits_made_right_after_recording() {
    // Right after refresh and after push: an edit within the same timestamp
    // tick that keeps the size must not pass for the recorded state.
    let dir = applied_hello_patch();
    fs::write(dir.path().join("hello.txt"), "NEW\n").unwrap();
    q(dir.path())
        .args(["pop"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not remove cleanly"));

    fs::write(dir.path().join("hello.txt"), "new\n").unwrap();
    q(dir.path()).args(["pop"]).assert().success();
    q(dir.path()).args(["push"]).assert().success();
    fs::write(dir.path().join("hello.txt"), "NEW\n").unwrap();
    q(dir.path())
        .args(["pop"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not remove cleanly"));
}

#[test]
fn pop_discards_backups() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("a"), "a\n").unwrap();
    fs::write(dir.path().join("b"), "b\n").unwrap();
    q(dir.path()).args(["new", "p.diff"]).assert().success();
    q(dir.path()).args(["add", "a", "b"]).assert().success();
    fs::write(dir.path().join("a"), "A\n").unwrap();
    fs::write(dir.path().join("b"), "B\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();
    q(dir.path()).args(["pop"]).assert().success();
    assert!(!dir.path().join(".pc/p.diff").exists());
    assert!(!dir.path().join(".pc/.q/p.diff").exists());

    // Drop b from the patch, and let b change upstream meanwhile.
    let patch = fs::read_to_string(dir.path().join("patches/p.diff")).unwrap();
    let b_section = patch.match_indices("Index: ").nth(1).unwrap().0;
    let a_only = &patch[..b_section];
    assert!(a_only.contains("+A") && !a_only.contains("+B"));
    fs::write(dir.path().join("patches/p.diff"), a_only).unwrap();
    fs::write(dir.path().join("b"), "upstream\n").unwrap();

    q(dir.path()).args(["push"]).assert().success();
    q(dir.path())
        .args(["files"])
        .assert()
        .success()
        .stdout("a\n");
    q(dir.path()).args(["pop"]).assert().success();
    assert_eq!(fs::read_to_string(dir.path().join("a")).unwrap(), "a\n");
    assert_eq!(
        fs::read_to_string(dir.path().join("b")).unwrap(),
        "upstream\n"
    );
}

#[test]
fn pop_gives_restored_files_a_fresh_mtime() {
    let dir = applied_hello_patch();
    let hello = dir.path().join("hello.txt");
    let old = filetime::FileTime::from_unix_time(1_000_000_000, 0);
    // Age the backup, as a clone or a long-lived backup would be.
    filetime::set_file_mtime(dir.path().join(".pc/change.diff/hello.txt"), old).unwrap();
    q(dir.path()).args(["pop"]).assert().success();
    assert_eq!(fs::read_to_string(&hello).unwrap(), "old\n");
    let mtime = filetime::FileTime::from_last_modification_time(&fs::metadata(&hello).unwrap());
    assert!(mtime > old, "restored file kept the backup's old mtime");
}

#[test]
fn push_refresh_rewrites_patch_and_still_pops_cleanly() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("hello.txt"), "old\n").unwrap();
    fs::create_dir(dir.path().join("patches")).unwrap();
    // No Index: line, so refreshing rewrites the patch text.
    fs::write(
        dir.path().join("patches/change.diff"),
        "--- a/hello.txt\n+++ b/hello.txt\n@@ -1 +1 @@\n-old\n+new\n",
    )
    .unwrap();
    fs::write(dir.path().join("patches/series"), "change.diff\n").unwrap();

    q(dir.path())
        .args(["push", "--refresh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Refreshed patch"));
    let patch = fs::read_to_string(dir.path().join("patches/change.diff")).unwrap();
    assert!(patch.starts_with("Index: "));

    fs::write(dir.path().join("hello.txt"), "NEW\n").unwrap();
    q(dir.path())
        .args(["pop"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not remove cleanly"));
    fs::write(dir.path().join("hello.txt"), "new\n").unwrap();
    q(dir.path()).args(["pop"]).assert().success();
    assert_eq!(
        fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
        "old\n"
    );
}

#[test]
fn pop_removes_backup_directories_of_nested_patch_names() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("a"), "a\n").unwrap();
    q(dir.path())
        .args(["new", "sub/dir/s.diff"])
        .assert()
        .success();
    q(dir.path()).args(["add", "a"]).assert().success();
    fs::write(dir.path().join("a"), "A\n").unwrap();
    q(dir.path()).args(["refresh"]).assert().success();
    assert!(dir.path().join(".pc/sub/dir/s.diff").exists());
    q(dir.path()).args(["pop"]).assert().success();
    assert!(!dir.path().join(".pc/sub").exists());
    assert!(!dir.path().join(".pc/.q/sub").exists());
    assert!(dir.path().join(".pc/.version").exists());
}
