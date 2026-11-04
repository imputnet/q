mod common;
use common::*;

#[test]
fn graph_outputs_dot_for_shared_files() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("foo"), "one\n").unwrap();
    q(dir.path()).args(["new", "one.diff"]).assert().success();
    q(dir.path()).args(["add", "foo"]).assert().success();
    fs::write(dir.path().join("foo"), "two\n").unwrap();
    q(dir.path()).args(["refresh"]).assert().success();
    q(dir.path()).args(["new", "two.diff"]).assert().success();
    q(dir.path()).args(["add", "foo"]).assert().success();

    q(dir.path())
        .args(["graph", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"two.diff\" -> \"one.diff\""));
}

#[test]
fn graph_accepts_upstream_option_forms() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("foo"), "one\n").unwrap();
    q(dir.path()).args(["new", "one.diff"]).assert().success();
    q(dir.path()).args(["add", "foo"]).assert().success();
    fs::write(dir.path().join("foo"), "two\n").unwrap();
    q(dir.path()).args(["refresh"]).assert().success();
    q(dir.path()).args(["new", "two.diff"]).assert().success();
    q(dir.path()).args(["add", "foo"]).assert().success();
    fs::write(dir.path().join("foo"), "three\n").unwrap();
    q(dir.path()).args(["refresh"]).assert().success();

    q(dir.path())
        .args(["graph", "--all", "--edge-labels", "files", "--lines", "4"])
        .assert()
        .success()
        .stdout(predicate::str::contains("label=\"foo\""));
}

#[test]
fn graph_patch_argument_includes_related_patches() {
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
    q(dir.path())
        .args(["new", "isolated.diff"])
        .assert()
        .success();
    fs::write(dir.path().join("bar"), "x\n").unwrap();
    q(dir.path()).args(["add", "bar"]).assert().success();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();

    q(dir.path())
        .args(["graph", "one.diff"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"one.diff\""))
        .stdout(predicate::str::contains("\"two.diff\""))
        .stdout(predicate::str::contains("isolated.diff").not());
}

#[test]
fn graph_lines_filters_non_overlapping_file_changes() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("f"), "a\nb\nc\nd\ne\n").unwrap();
    q(dir.path()).args(["new", "one.diff"]).assert().success();
    q(dir.path()).args(["add", "f"]).assert().success();
    fs::write(dir.path().join("f"), "A\nb\nc\nd\ne\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();
    q(dir.path()).args(["new", "two.diff"]).assert().success();
    q(dir.path()).args(["add", "f"]).assert().success();
    fs::write(dir.path().join("f"), "A\nb\nc\nd\nE\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();

    q(dir.path())
        .args(["graph", "--all"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"two.diff\" -> \"one.diff\""));

    q(dir.path())
        .args(["graph", "--all", "--lines=0"])
        .assert()
        .success()
        .stdout(predicate::str::contains("->").not());
}
