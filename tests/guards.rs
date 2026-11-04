mod common;
use common::*;

#[test]
fn also_builds_drop_in_quilt_binary_name() {
    let dir = tempdir().unwrap();
    quilt(dir.path())
        .args(["new", "named.diff"])
        .assert()
        .success();
    quilt(dir.path())
        .args(["top"])
        .assert()
        .success()
        .stdout("named.diff\n");
}

#[test]
fn guards_selects_lists_and_checks_entries() {
    let dir = tempdir().unwrap();
    fs::write(
        dir.path().join("guards.conf"),
        "base.patch\n+foo foo.patch\n-foo nofoo.patch\n+!bar notbar.patch\n",
    )
    .unwrap();
    fs::write(dir.path().join("base.patch"), "").unwrap();
    fs::write(dir.path().join("foo.patch"), "").unwrap();
    fs::write(dir.path().join("nofoo.patch"), "").unwrap();
    fs::write(dir.path().join("notbar.patch"), "").unwrap();
    fs::write(
        dir.path().join("files"),
        "base.patch\nfoo.patch\nnofoo.patch\nnotbar.patch\n",
    )
    .unwrap();

    guards(dir.path())
        .args(["--config", "guards.conf", "foo"])
        .assert()
        .success()
        .stdout("base.patch\nfoo.patch\nnotbar.patch\n");

    guards(dir.path())
        .args(["--list", "--with-guards", "--config", "guards.conf"])
        .assert()
        .success()
        .stdout("base.patch\n+foo foo.patch\n-foo nofoo.patch\n+!bar notbar.patch\n");

    guards(dir.path())
        .args(["--check", "--config", "guards.conf", "--path", "@files"])
        .assert()
        .success();
}
