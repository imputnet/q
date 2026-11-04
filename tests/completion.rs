mod common;
use common::*;

#[test]
fn hidden_completion_suggests_commands() {
    let dir = tempdir().unwrap();

    q(dir.path())
        .args(["__complete", "pu", "--", "pu"])
        .assert()
        .success()
        .stdout("push\n");
}

#[test]
fn completion_glue_falls_back_to_files() {
    let dir = tempdir().unwrap();

    q(dir.path())
        .args(["completion", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("complete -o default"))
        .stdout(predicate::str::contains("compopt -o nospace"));
    q(dir.path())
        .args(["completion", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("_files"))
        .stdout(predicate::str::contains("compadd -S ''"));
    q(dir.path())
        .args(["completion", "fish"])
        .assert()
        .success()
        .stdout(predicate::str::contains("complete -c q -f").not())
        .stdout(predicate::str::contains("complete -c quilt -f").not());
}

#[test]
fn zsh_completion_initializes_completion_system() {
    let dir = tempdir().unwrap();

    q(dir.path())
        .args(["completion", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("autoload -Uz compinit"))
        .stdout(predicate::str::contains("compinit"))
        .stdout(predicate::str::contains("compdef _q_complete q quilt"));
}

#[test]
fn zsh_completion_avoids_nested_command_substitution_expansion() {
    let dir = tempdir().unwrap();

    q(dir.path())
        .args(["completion", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains("(@").not())
        .stdout(predicate::str::contains("done < <(q __complete"));
    q(dir.path())
        .args(["completion", "zsh"])
        .assert()
        .success()
        .stdout(predicate::str::contains(r#"${words[@]:1}"#).not())
        .stdout(predicate::str::contains(r#"${words[@]}"#));
}

#[test]
fn hidden_completion_suggests_unique_patch_basenames() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("patches/a/b/c")).unwrap();
    fs::write(dir.path().join("patches/series"), "a/b/c/dddd.patch\n").unwrap();
    fs::write(dir.path().join("patches/a/b/c/dddd.patch"), "").unwrap();

    q(dir.path())
        .args(["__complete", "pop", "d", "--", "d"])
        .assert()
        .success()
        .stdout(predicate::str::contains("dddd\n"))
        .stdout(predicate::str::contains("dddd.patch\n"));

    q(dir.path())
        .args(["__complete", "pop", "a/", "--", "a/"])
        .assert()
        .success()
        .stdout("a/b/c/dddd.patch\n");
}

#[test]
fn hidden_completion_suggests_patches_after_patch_option() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("patches/a/b/c")).unwrap();
    fs::write(dir.path().join("patches/series"), "a/b/c/dddd.patch\n").unwrap();
    fs::write(dir.path().join("patches/a/b/c/dddd.patch"), "").unwrap();

    q(dir.path())
        .args(["__complete", "q", "add", "-P", "--", ""])
        .assert()
        .success()
        .stdout(predicate::str::contains("dddd\n"))
        .stdout(predicate::str::contains("dddd.patch\n"));
    q(dir.path())
        .args(["__complete", "quilt", "add", "-P", "--", "-P"])
        .assert()
        .success()
        .stdout(predicate::str::contains("dddd\n"))
        .stdout(predicate::str::contains("dddd.patch\n"));
    q(dir.path())
        .args(["__complete", "quilt", "add", "-P", "", "--", ""])
        .assert()
        .success()
        .stdout(predicate::str::contains("dddd\n"))
        .stdout(predicate::str::contains("dddd.patch\n"));
    q(dir.path())
        .args(["__complete", "-P", "--", "-P"])
        .assert()
        .success()
        .stdout(predicate::str::contains("dddd\n"))
        .stdout(predicate::str::contains("dddd.patch\n"));
    q(dir.path())
        .args(["__complete", "noise", "quilt", "add", "-P", "--", "-P"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Commands are:").not())
        .stdout(predicate::str::contains("Usage:").not())
        .stdout(predicate::str::contains("dddd\n"))
        .stdout(predicate::str::contains("dddd.patch\n"));
    q(dir.path())
        .args(["__complete", "quilt", "add", "-P", "d", "--", "d"])
        .assert()
        .success()
        .stdout(predicate::str::contains("dddd\n"))
        .stdout(predicate::str::contains("dddd.patch\n"));
    q(dir.path())
        .args(["__complete", "quilt", "add", "-P", "dddd", "--", ""])
        .assert()
        .success()
        .stdout("");
}

#[test]
fn hidden_completion_suggests_patch_directories_for_new() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("patches/a/b")).unwrap();
    fs::create_dir_all(dir.path().join("patches/other")).unwrap();
    fs::write(dir.path().join("patches/a.patch"), "").unwrap();
    fs::write(dir.path().join("patches/series"), "").unwrap();

    q(dir.path())
        .args(["__complete", "new", "--", ""])
        .assert()
        .success()
        .stdout(predicate::str::contains("a/\n"))
        .stdout(predicate::str::contains("other/\n"))
        .stdout(predicate::str::contains("a.patch").not());
    q(dir.path())
        .args(["__complete", "new", "a/", "--", "a/"])
        .assert()
        .success()
        .stdout("a/b/\n");
    q(dir.path())
        .args(["__complete", "quilt", "new", "o", "--", "o"])
        .assert()
        .success()
        .stdout("other/\n");
    q(dir.path())
        .args(["__complete", "new", "../", "--", "../"])
        .assert()
        .success()
        .stdout("");
    q(dir.path())
        .args(["__complete", "add", "h", "--", "h"])
        .assert()
        .success()
        .stdout("");
}

#[test]
fn hidden_completion_omits_ambiguous_patch_basenames() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join("patches/a")).unwrap();
    fs::create_dir_all(dir.path().join("patches/b")).unwrap();
    fs::write(
        dir.path().join("patches/series"),
        "a/dddd.patch\nb/dddd.patch\n",
    )
    .unwrap();
    fs::write(dir.path().join("patches/a/dddd.patch"), "").unwrap();
    fs::write(dir.path().join("patches/b/dddd.patch"), "").unwrap();

    q(dir.path())
        .args(["__complete", "pop", "d", "--", "d"])
        .assert()
        .success()
        .stdout(predicate::str::contains("dddd").not());
    q(dir.path())
        .args(["__complete", "pop", "a/", "--", "a/"])
        .assert()
        .success()
        .stdout("a/dddd.patch\n");
}
