mod common;
use common::*;

#[test]
fn quiltrc_command_args_are_prepended() {
    let dir = tempdir().unwrap();
    let quiltrc = dir.path().join("quiltrc");
    fs::write(&quiltrc, "QUILT_REFRESH_ARGS=\"-p ab --backup\"\n").unwrap();
    fs::write(dir.path().join("foo"), "old\n").unwrap();

    q(dir.path())
        .args(["--quiltrc", quiltrc.to_str().unwrap(), "new", "p.diff"])
        .assert()
        .success();
    q(dir.path()).args(["add", "foo"]).assert().success();
    fs::write(dir.path().join("foo"), "new\n").unwrap();
    q(dir.path())
        .args(["--quiltrc", quiltrc.to_str().unwrap(), "refresh"])
        .assert()
        .success();
    fs::write(dir.path().join("foo"), "newer\n").unwrap();
    q(dir.path())
        .args(["--quiltrc", quiltrc.to_str().unwrap(), "refresh"])
        .assert()
        .success();

    let patch = fs::read_to_string(dir.path().join("patches/p.diff")).unwrap();
    assert!(patch.contains("Index: b/foo"));
    assert!(dir.path().join("patches/p.diff~").exists());
}

#[test]
fn quiltrc_is_sourced_as_bash() {
    let dir = tempdir().unwrap();
    let quiltrc = dir.path().join("quiltrc");
    fs::write(
        &quiltrc,
        "case yes in yes) QUILT_DIFF_ARGS=--no-index ;; esac\n",
    )
    .unwrap();
    fs::write(dir.path().join("foo"), "old\n").unwrap();
    q(dir.path())
        .args(["--quiltrc", quiltrc.to_str().unwrap(), "new", "p.diff"])
        .assert()
        .success();
    q(dir.path())
        .args(["--quiltrc", quiltrc.to_str().unwrap(), "add", "foo"])
        .assert()
        .success();
    fs::write(dir.path().join("foo"), "new\n").unwrap();

    q(dir.path())
        .args(["--quiltrc", quiltrc.to_str().unwrap(), "diff"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Index:").not());
}

#[test]
fn quiltrc_dash_suppresses_default_rc_loading() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    fs::create_dir(&home).unwrap();
    fs::write(home.join(".quiltrc"), "QUILT_DIFF_ARGS=--no-index\n").unwrap();
    fs::write(dir.path().join("foo"), "old\n").unwrap();

    q(dir.path())
        .env("HOME", &home)
        .args(["--quiltrc", "-", "new", "p.diff"])
        .assert()
        .success();
    q(dir.path())
        .env("HOME", &home)
        .args(["--quiltrc", "-", "add", "foo"])
        .assert()
        .success();
    fs::write(dir.path().join("foo"), "new\n").unwrap();

    q(dir.path())
        .env("HOME", &home)
        .args(["--quiltrc", "-", "diff"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Index:"));
}

#[test]
fn quiltrc_precedence_prefers_explicit_then_env_then_home() {
    let dir = tempdir().unwrap();
    let home = dir.path().join("home");
    fs::create_dir(&home).unwrap();
    let env_rc = dir.path().join("env.quiltrc");
    let explicit_rc = dir.path().join("explicit.quiltrc");
    fs::write(home.join(".quiltrc"), "QUILT_DIFF_ARGS=--no-index\n").unwrap();
    fs::write(&env_rc, "QUILT_DIFF_ARGS=--no-timestamps\n").unwrap();
    fs::write(
        &explicit_rc,
        "QUILT_DIFF_ARGS='--no-index --no-timestamps'\n",
    )
    .unwrap();
    fs::write(dir.path().join("foo"), "old\n").unwrap();

    q(dir.path())
        .env("HOME", &home)
        .env("QUILTRC", &env_rc)
        .args(["new", "p.diff"])
        .assert()
        .success();
    q(dir.path()).args(["add", "foo"]).assert().success();
    fs::write(dir.path().join("foo"), "new\n").unwrap();

    q(dir.path())
        .env("HOME", &home)
        .env("QUILTRC", &env_rc)
        .args(["diff"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Index:"))
        .stdout(predicate::str::contains("\t20").not());

    q(dir.path())
        .env("HOME", &home)
        .env("QUILTRC", &env_rc)
        .args(["--quiltrc", explicit_rc.to_str().unwrap(), "diff"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Index:").not())
        .stdout(predicate::str::contains("\t20").not());
}

#[test]
fn environment_backup_enables_header_backup() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("foo"), "old\n").unwrap();
    q(dir.path()).args(["new", "p.diff"]).assert().success();
    q(dir.path()).args(["add", "foo"]).assert().success();
    fs::write(dir.path().join("foo"), "new\n").unwrap();
    q(dir.path()).args(["refresh"]).assert().success();

    q(dir.path())
        .env("QUILT_BACKUP", "1")
        .args(["header", "-r"])
        .write_stdin("Header\n")
        .assert()
        .success();

    assert!(dir.path().join("patches/p.diff~").exists());
}

#[test]
fn diff_environment_options_are_honored() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("foo"), "a b\n").unwrap();
    q(dir.path()).args(["new", "p.diff"]).assert().success();
    q(dir.path()).args(["add", "foo"]).assert().success();
    fs::write(dir.path().join("foo"), "ab\n").unwrap();

    q(dir.path())
        .env("QUILT_NO_DIFF_INDEX", "1")
        .args(["diff"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Index:").not());

    q(dir.path())
        .env("QUILT_DIFF_OPTS", "-w")
        .args(["refresh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Nothing in patch p.diff"));
}

#[test]
fn malformed_command_args_environment_is_reported() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("foo"), "old\n").unwrap();
    q(dir.path()).args(["new", "p.diff"]).assert().success();
    q(dir.path()).args(["add", "foo"]).assert().success();
    fs::write(dir.path().join("foo"), "new\n").unwrap();

    q(dir.path())
        .env("QUILT_REFRESH_ARGS", "\"unterminated")
        .args(["refresh"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "failed to parse QUILT_REFRESH_ARGS",
        ));
}

#[test]
fn diff_color_always_honors_color_environment() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("foo"), "old\n").unwrap();
    q(dir.path()).args(["new", "p.diff"]).assert().success();
    q(dir.path()).args(["add", "foo"]).assert().success();
    fs::write(dir.path().join("foo"), "new\n").unwrap();

    q(dir.path())
        .env("QUILT_NO_DIFF_TIMESTAMPS", "1")
        .env(
            "QUILT_COLORS",
            "diff_hdr=31:diff_add=32:diff_rem=33:diff_hunk=34:clear=0",
        )
        .args(["diff", "--color=always"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\u{1b}[31mIndex:"))
        .stdout(predicate::str::contains("\u{1b}[33m-old\n\u{1b}[0m"))
        .stdout(predicate::str::contains("\u{1b}[32m+new\n\u{1b}[0m"));
}

#[test]
fn combined_diff_color_always_honors_color_environment() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("f"), "old\n").unwrap();
    q(dir.path()).args(["new", "one.diff"]).assert().success();
    q(dir.path()).args(["add", "f"]).assert().success();
    fs::write(dir.path().join("f"), "mid\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();
    q(dir.path()).args(["new", "two.diff"]).assert().success();
    q(dir.path()).args(["add", "f"]).assert().success();
    fs::write(dir.path().join("f"), "new\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();

    q(dir.path())
        .env("QUILT_NO_DIFF_TIMESTAMPS", "1")
        .env(
            "QUILT_COLORS",
            "diff_hdr=31:diff_add=32:diff_rem=33:diff_hunk=34:clear=0",
        )
        .args(["diff", "--combine", "-", "--color=always"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\u{1b}[31mIndex:"))
        .stdout(predicate::str::contains("\u{1b}[33m-old\n\u{1b}[0m"))
        .stdout(predicate::str::contains("\u{1b}[32m+new\n\u{1b}[0m"));
}

#[test]
fn diff_timestamps_are_controlled_by_environment() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("foo"), "one\n").unwrap();
    q(dir.path()).args(["new", "p.diff"]).assert().success();
    q(dir.path()).args(["add", "foo"]).assert().success();
    fs::write(dir.path().join("foo"), "two\n").unwrap();

    q(dir.path())
        .args(["diff"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--- ").and(predicate::str::contains("\t20")));

    q(dir.path())
        .env("QUILT_NO_DIFF_TIMESTAMPS", "1")
        .args(["diff"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\t20").not());
}

#[test]
fn diff_external_utility_receives_old_and_new_files() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().unwrap();
    fs::write(dir.path().join("foo"), "old\n").unwrap();
    q(dir.path()).args(["new", "p.diff"]).assert().success();
    q(dir.path()).args(["add", "foo"]).assert().success();
    fs::write(dir.path().join("foo"), "new\n").unwrap();
    let utility = dir.path().join("show-diff-args");
    fs::write(
        &utility,
        "#!/bin/sh\nprintf '%s\\n%s\\n' \"$(cat \"$1\")\" \"$(cat \"$2\")\"\n",
    )
    .unwrap();
    let mut perms = fs::metadata(&utility).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&utility, perms).unwrap();

    q(dir.path())
        .args(["diff", "--diff", utility.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("old\nnew\n"))
        .stdout(predicate::str::contains("Index:").not());
}

#[test]
fn version_works_after_global_options() {
    let dir = tempdir().unwrap();
    q(dir.path())
        .args(["--quiltrc", "-", "--version"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty().not());
}
