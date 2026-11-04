#![allow(dead_code, unused_imports)]

use assert_cmd::Command;

pub use predicates::prelude::*;
pub use std::fs;
pub use std::process::Command as StdCommand;
pub use tempfile::tempdir;

pub fn q(dir: &std::path::Path) -> Command {
    let mut cmd = match std::env::var_os("Q_TEST_BIN") {
        Some(bin) => Command::new(bin),
        None => Command::cargo_bin("q").unwrap(),
    };
    cmd.current_dir(dir);
    isolate_quilt_env(&mut cmd);
    cmd
}

pub fn quilt(dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("quilt").unwrap();
    cmd.current_dir(dir);
    isolate_quilt_env(&mut cmd);
    cmd
}

pub fn isolate_quilt_env(cmd: &mut Command) {
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("QUILT") {
            cmd.env_remove(name);
        }
    }
    cmd.env("QUILTRC", "/dev/null");
}

pub fn isolate_quilt_env_std(cmd: &mut std::process::Command) {
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("QUILT") {
            cmd.env_remove(name);
        }
    }
    cmd.env("QUILTRC", "/dev/null");
}

pub fn guards(dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("guards").unwrap();
    cmd.current_dir(dir);
    cmd
}

pub fn git(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    StdCommand::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .args(args)
        .output()
        .unwrap()
}

pub fn init_git_baseline(dir: &std::path::Path) {
    assert!(git(dir, &["init"]).status.success());
    assert!(
        git(dir, &["config", "user.email", "q@example.invalid"])
            .status
            .success()
    );
    assert!(git(dir, &["config", "user.name", "q"]).status.success());
    assert!(git(dir, &["add", "."]).status.success());
    assert!(git(dir, &["commit", "-m", "init"]).status.success());
}
