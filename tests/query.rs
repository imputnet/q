mod common;
use common::*;

#[test]
fn series_and_stack_queries_use_quilt_metadata() {
    let dir = tempdir().unwrap();
    q(dir.path()).args(["new", "one.diff"]).assert().success();
    q(dir.path()).args(["new", "two.diff"]).assert().success();

    q(dir.path())
        .args(["top"])
        .assert()
        .success()
        .stdout("two.diff\n");
    q(dir.path())
        .args(["applied"])
        .assert()
        .success()
        .stdout("one.diff\ntwo.diff\n");
    q(dir.path())
        .args(["series", "-v"])
        .assert()
        .success()
        .stdout(predicate::str::contains("+ one.diff\n= two.diff\n"));
}

#[test]
fn commands_work_from_subdirectories() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src/nested")).unwrap();
    fs::write(dir.path().join("src/file.txt"), "old\n").unwrap();

    q(dir.path())
        .args(["new", "subdir.diff"])
        .assert()
        .success();
    q(&dir.path().join("src"))
        .args(["add", "file.txt"])
        .assert()
        .success()
        .stdout(predicate::str::contains("File src/file.txt added"));
    fs::write(dir.path().join("src/file.txt"), "new\n").unwrap();
    q(&dir.path().join("src"))
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();

    q(&dir.path().join("src/nested"))
        .args(["files"])
        .assert()
        .success()
        .stdout("src/file.txt\n");
    let patch = fs::read_to_string(dir.path().join("patches/subdir.diff")).unwrap();
    assert!(patch.contains("src/file.txt"));
}

#[test]
fn display_patch_honors_patches_prefix_environment() {
    let dir = tempdir().unwrap();
    q(dir.path())
        .env("QUILT_PATCHES_PREFIX", "yes")
        .args(["new", "prefix.diff"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Patch patches/prefix.diff is now on top",
        ));
    q(dir.path())
        .env("QUILT_PATCHES_PREFIX", "yes")
        .args(["top"])
        .assert()
        .success()
        .stdout("patches/prefix.diff\n");
}

#[test]
fn patches_prefix_is_controlled_by_quilt_patches_prefix() {
    let dir = tempfile::Builder::new()
        .prefix("q space ")
        .tempdir()
        .unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    q(dir.path())
        .args(["new", "space.diff"])
        .assert()
        .success()
        .stdout("Patch space.diff is now on top\n");

    q(&dir.path().join("src"))
        .args(["top"])
        .assert()
        .success()
        .stdout("space.diff\n");
    q(&dir.path().join("src"))
        .env("QUILT_PATCHES_PREFIX", "yes")
        .args(["top"])
        .assert()
        .success()
        .stdout("../patches/space.diff\n");
}

#[test]
fn series_color_always_honors_color_environment() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("foo"), "one\n").unwrap();
    q(dir.path()).args(["new", "one.diff"]).assert().success();
    q(dir.path()).args(["add", "foo"]).assert().success();
    fs::write(dir.path().join("foo"), "two\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();
    q(dir.path()).args(["new", "two.diff"]).assert().success();
    q(dir.path()).args(["add", "foo"]).assert().success();
    fs::write(dir.path().join("foo"), "three\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();
    fs::write(
        dir.path().join("patches/three.diff"),
        "--- a/foo\n+++ b/foo\n@@ -1 +1 @@\n-three\n+four\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("patches/series"),
        "one.diff\ntwo.diff\nthree.diff\n",
    )
    .unwrap();

    q(dir.path())
        .env(
            "QUILT_COLORS",
            "series_app=31:series_top=32:series_una=33:clear=0",
        )
        .args(["series", "--color=always", "-v"])
        .assert()
        .success()
        .stdout("\u{1b}[31m+ one.diff\u{1b}[0m\n\u{1b}[32m= two.diff\u{1b}[0m\n\u{1b}[33m  three.diff\u{1b}[0m\n");
}

#[cfg(target_os = "linux")]
#[test]
fn series_color_auto_survives_pager_pipe() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("foo"), "one\n").unwrap();
    q(dir.path()).args(["new", "one.diff"]).assert().success();
    q(dir.path()).args(["add", "foo"]).assert().success();
    fs::write(dir.path().join("foo"), "two\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();
    q(dir.path()).args(["new", "two.diff"]).assert().success();

    let q_bin = assert_cmd::cargo::cargo_bin("q");
    let mut cmd = std::process::Command::new("script");
    isolate_quilt_env_std(&mut cmd);
    let output = cmd
        .args([
            "-q",
            "-c",
            &format!(
                "env QUILT_SERIES_ARGS=--color=auto PAGER='cat -' '{}' series -v",
                q_bin.display()
            ),
            "/dev/null",
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\u{1b}[32m+ one.diff\u{1b}[00m"));
    assert!(stdout.contains("\u{1b}[33m= two.diff\u{1b}[00m"));
}

#[test]
fn patches_color_always_honors_color_environment() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("foo"), "one\n").unwrap();
    q(dir.path()).args(["new", "one.diff"]).assert().success();
    q(dir.path()).args(["add", "foo"]).assert().success();
    fs::write(dir.path().join("foo"), "two\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();
    q(dir.path()).args(["new", "two.diff"]).assert().success();
    q(dir.path()).args(["add", "foo"]).assert().success();
    fs::write(dir.path().join("foo"), "three\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();
    fs::write(
        dir.path().join("patches/three.diff"),
        "--- a/foo\n+++ b/foo\n@@ -1 +1 @@\n-three\n+four\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("patches/series"),
        "one.diff\ntwo.diff\nthree.diff\n",
    )
    .unwrap();

    q(dir.path())
        .env(
            "QUILT_COLORS",
            "series_app=31:series_top=32:series_una=33:clear=0",
        )
        .args(["patches", "--color=always", "-v", "foo"])
        .assert()
        .success()
        .stdout("\u{1b}[31m+ one.diff\u{1b}[0m\n\u{1b}[32m= two.diff\u{1b}[0m\n\u{1b}[33m  three.diff\u{1b}[0m\n");
}

#[test]
fn patches_finds_unapplied_patch_files() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("patches")).unwrap();
    fs::write(dir.path().join("file"), "old\n").unwrap();
    fs::write(dir.path().join("patches/series"), "change.diff\n").unwrap();
    fs::write(
        dir.path().join("patches/change.diff"),
        "--- a/file\n+++ b/file\n@@ -1 +1 @@\n-old\n+new\n",
    )
    .unwrap();

    q(dir.path())
        .args(["patches", "-v", "file"])
        .assert()
        .success()
        .stdout("  change.diff\n");
}

#[test]
fn files_combine_lists_applied_patch_range() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("f"), "one\n").unwrap();
    q(dir.path()).args(["new", "a.diff"]).assert().success();
    q(dir.path()).args(["add", "f"]).assert().success();
    fs::write(dir.path().join("f"), "two\n").unwrap();
    q(dir.path()).args(["refresh"]).assert().success();
    q(dir.path()).args(["new", "b.diff"]).assert().success();
    q(dir.path()).args(["add", "g"]).assert().success();
    fs::write(dir.path().join("g"), "g\n").unwrap();
    q(dir.path()).args(["refresh"]).assert().success();
    q(dir.path()).args(["new", "c.diff"]).assert().success();
    q(dir.path()).args(["add", "h"]).assert().success();
    fs::write(dir.path().join("h"), "h\n").unwrap();
    q(dir.path()).args(["refresh"]).assert().success();

    q(dir.path())
        .args(["files", "--combine", "a.diff", "b.diff"])
        .assert()
        .success()
        .stdout("f\ng\n");
    q(dir.path())
        .args(["files", "--combine", "-", "-l", "c.diff"])
        .assert()
        .success()
        .stdout("a.diff f\nb.diff g\nc.diff h\n");
}

#[test]
fn annotate_reports_patch_numbers_for_changed_lines() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("foo"), "foo\nbar\nbaz\n").unwrap();
    q(dir.path()).args(["new", "patch"]).assert().success();
    q(dir.path()).args(["add", "foo"]).assert().success();
    fs::write(dir.path().join("foo"), "foo\nBar\nBaz\n").unwrap();
    q(dir.path()).args(["refresh"]).assert().success();

    q(dir.path())
        .args(["annotate", "foo"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\tfoo\n1\tBar\n1\tBaz\n\n1\tpatch\n",
        ));
}

#[test]
fn grep_matches_upstream_filename_prefixing_and_help_delimiter() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("patches")).unwrap();
    fs::write(dir.path().join("patches/series"), "").unwrap();
    fs::write(dir.path().join("file"), "needle\n").unwrap();

    q(dir.path())
        .args(["grep", "needle", "file"])
        .assert()
        .success()
        .stdout("file:needle\n");
    q(dir.path())
        .args(["grep", "--", "-h", "needle", "file"])
        .assert()
        .success()
        .stdout("needle\n");
}

#[test]
fn grep_skips_configured_metadata_directories() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("qp")).unwrap();
    fs::create_dir(dir.path().join("meta")).unwrap();
    fs::write(dir.path().join("qp/series"), "").unwrap();
    fs::write(dir.path().join("qp/hidden"), "needle\n").unwrap();
    fs::write(dir.path().join("meta/hidden"), "needle\n").unwrap();
    fs::write(dir.path().join("visible"), "needle\n").unwrap();

    q(dir.path())
        .env("QUILT_PATCHES", "qp")
        .env("QUILT_PC", "meta")
        .args(["grep", "needle"])
        .assert()
        .success()
        .stdout("visible:needle\n");
}

#[cfg(unix)]
#[test]
fn paged_commands_honor_quilt_pager_on_tty() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().unwrap();
    q(dir.path()).args(["new", "p.diff"]).assert().success();
    let capture = dir.path().join("pager.out");
    let env_capture = dir.path().join("pager.env");
    let pager = dir.path().join("pager");
    fs::write(
        &pager,
        format!(
            "#!/bin/sh\ncat > '{}'\nprintf 'LESS=%s\\n' \"$LESS\" > '{}'\n",
            capture.display(),
            env_capture.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&pager, fs::Permissions::from_mode(0o755)).unwrap();

    let q_bin = assert_cmd::cargo::cargo_bin("q");
    let mut cmd = std::process::Command::new("script");
    isolate_quilt_env_std(&mut cmd);
    let status = cmd
        .arg("-q")
        .arg("-c")
        .arg(format!("{} series", q_bin.display()))
        .arg("/dev/null")
        .current_dir(dir.path())
        .env("QUILT_PAGER", &pager)
        .status()
        .unwrap();

    assert!(status.success());
    assert_eq!(fs::read_to_string(capture).unwrap(), "p.diff\n");
    assert_eq!(fs::read_to_string(env_capture).unwrap(), "LESS=FRSX\n");
}

#[test]
fn patches_reads_unapplied_patches_with_their_strip_level() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("patches")).unwrap();
    fs::write(dir.path().join("f"), "a\n").unwrap();
    fs::write(dir.path().join("patches/series"), "p0.diff -p0\n").unwrap();
    fs::write(
        dir.path().join("patches/p0.diff"),
        "--- f.orig\n+++ f\n@@ -1 +1 @@\n-a\n+b\n",
    )
    .unwrap();
    q(dir.path())
        .args(["patches", "f"])
        .assert()
        .success()
        .stdout("p0.diff\n");
}
