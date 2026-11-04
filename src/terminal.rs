use std::io::{self, IsTerminal};
use std::sync::OnceLock;

static STDOUT_IS_TERMINAL: OnceLock<bool> = OnceLock::new();

/// Record whether stdout is a terminal before the pager replaces it with a
/// pipe, so `--color=auto` still sees the real terminal.
pub(crate) fn remember_stdout() -> bool {
    stdout_is_terminal()
}

pub(crate) fn stdout_is_terminal() -> bool {
    *STDOUT_IS_TERMINAL.get_or_init(|| io::stdout().is_terminal())
}
