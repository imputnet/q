//! Embed the git commit q is built from as its version (`Q_VERSION`), since q
//! is used from git rather than released.

use std::process::Command;

fn main() {
    // Builds without a git checkout (e.g. Nix) pass the version in instead.
    println!("cargo:rerun-if-env-changed=Q_VERSION");
    let version = std::env::var("Q_VERSION")
        .ok()
        .filter(|version| !version.is_empty())
        .unwrap_or_else(git_version);
    println!("cargo:rustc-env=Q_VERSION={version}");
    for path in [
        ".git/HEAD",
        ".git/refs/heads",
        ".git/index",
        "src",
        "p2/src",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
}

fn git_version() -> String {
    Command::new("git")
        .args(["describe", "--always", "--dirty", "--exclude=*"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}
