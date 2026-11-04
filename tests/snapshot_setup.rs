mod common;
use common::*;

#[test]
fn snapshot_diff_requires_existing_snapshot() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("tracked"), "one\n").unwrap();
    q(dir.path()).args(["new", "p.diff"]).assert().success();
    q(dir.path()).args(["add", "tracked"]).assert().success();

    q(dir.path())
        .args(["diff", "--snapshot"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("No snapshot to diff against"));
}

#[test]
fn snapshot_diff_only_reports_files_in_applied_patches() {
    let dir = tempdir().unwrap();
    fs::write(dir.path().join("tracked"), "tracked\n").unwrap();
    fs::write(dir.path().join("untracked"), "untracked\n").unwrap();
    q(dir.path()).args(["new", "p.diff"]).assert().success();
    q(dir.path()).args(["add", "tracked"]).assert().success();
    fs::write(dir.path().join("tracked"), "tracked2\n").unwrap();
    q(dir.path())
        .args(["refresh", "--no-timestamps"])
        .assert()
        .success();

    q(dir.path()).args(["snapshot"]).assert().success();
    assert!(dir.path().join(".pc/.snap/tracked").exists());

    fs::write(dir.path().join("tracked"), "tracked3\n").unwrap();
    fs::write(dir.path().join("untracked"), "untracked2\n").unwrap();
    q(dir.path()).args(["new", "after.diff"]).assert().success();
    q(dir.path())
        .args(["add", "added-after"])
        .assert()
        .success();
    fs::write(dir.path().join("added-after"), "added\n").unwrap();
    let output = q(dir.path())
        .args(["diff", "--snapshot", "--no-timestamps"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output).unwrap();
    assert!(stdout.contains("Index:"));
    assert!(stdout.contains("tracked"));
    assert!(stdout.contains("-tracked2"));
    assert!(stdout.contains("+tracked3"));
    assert!(stdout.contains("added-after"));
    assert!(stdout.contains("+added"));
    assert!(!stdout.contains("untracked"));
}

#[test]
fn setup_honors_series_sourcedir_for_plain_tar() {
    let dir = tempdir().unwrap();
    fs::create_dir(dir.path().join("srcpkg")).unwrap();
    fs::write(dir.path().join("srcpkg/f"), "one\n").unwrap();
    let status = std::process::Command::new("tar")
        .args(["cf", "srcpkg.tar", "srcpkg"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    assert!(status.success());
    fs::remove_dir_all(dir.path().join("srcpkg")).unwrap();
    fs::write(
        dir.path().join("series"),
        "# Sourcedir: unpacked\n# Source: srcpkg.tar\n# Patchdir: unpacked/srcpkg\n#\np.diff\n",
    )
    .unwrap();

    q(dir.path()).args(["setup", "series"]).assert().success();

    assert!(dir.path().join("unpacked/srcpkg/f").exists());
    let patches = fs::read_link(dir.path().join("unpacked/srcpkg/patches")).unwrap();
    let series = fs::read_link(dir.path().join("unpacked/srcpkg/series")).unwrap();
    assert_eq!(patches, std::path::PathBuf::from("../.."));
    assert_eq!(series, std::path::PathBuf::from("../../series"));
}
