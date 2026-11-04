//! `--color` options and the `QUILT_COLORS` palette.

use crate::{cli, terminal};
use anyhow::{Result, bail};
use std::env;
use std::ffi::OsString;

/// Parse a `--color[=always|auto|tty|never]` value into whether to color.
pub(crate) fn color_arg(value: Option<OsString>, command: &str) -> Result<bool> {
    let value = value.map(|value| value.to_string_lossy().into_owned());
    match value.as_deref() {
        None | Some("" | "always") => Ok(true),
        Some("auto" | "tty") => Ok(terminal::stdout_is_terminal()),
        Some("never") => Ok(false),
        Some(_) => bail!("{}", cli::usage(command)),
    }
}

/// The escape sequence for `name`: its `QUILT_COLORS` entry, else `default`.
pub(crate) fn escape(name: &str, default: &str) -> String {
    let configured = env::var("QUILT_COLORS").ok().and_then(|colors| {
        // Later entries win, as when the shell version assigns them in turn.
        colors
            .rsplit(':')
            .filter_map(|item| item.split_once('='))
            .find(|(key, _)| *key == name)
            .map(|(_, code)| code.to_string())
    });
    format!("\x1b[{}m", configured.as_deref().unwrap_or(default))
}
