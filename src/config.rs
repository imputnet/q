//! Settings q takes from the environment, and the external tools it runs.

use anyhow::{Context, Result};
use std::env;
use std::process::Command;
use std::sync::OnceLock;

/// Whether a boolean setting like `QUILT_BACKUP` is set to anything.
pub(crate) fn env_flag(name: &str) -> bool {
    env::var_os(name).is_some_and(|value| !value.is_empty())
}

/// A shell-quoted argument list setting like `QUILT_DIFF_OPTS`.
pub(crate) fn env_args(name: &str) -> Result<Vec<String>> {
    let Some(value) = env::var_os(name) else {
        return Ok(Vec::new());
    };
    let value = value.to_string_lossy();
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    shlex::split(&value).with_context(|| format!("failed to parse {name}"))
}

/// GNU patch, preferring `gpatch` where `patch` may be a BSD one.
pub(crate) fn patch_command() -> Command {
    static PROGRAM: OnceLock<&str> = OnceLock::new();
    Command::new(PROGRAM.get_or_init(|| gnu_program("gpatch", "patch")))
}

/// GNU diff, preferring `gdiff` where `diff` may be a BSD one.
pub(crate) fn diff_command() -> Command {
    static PROGRAM: OnceLock<&str> = OnceLock::new();
    Command::new(PROGRAM.get_or_init(|| gnu_program("gdiff", "diff")))
}

fn gnu_program(gnu: &'static str, fallback: &'static str) -> &'static str {
    let on_path = env::var_os("PATH")
        .is_some_and(|path| env::split_paths(&path).any(|dir| dir.join(gnu).is_file()));
    if on_path { gnu } else { fallback }
}
