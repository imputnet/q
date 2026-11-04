use crate::commands::*;
use crate::{completion, terminal};
use anyhow::{Context, Result, bail};
use std::env;
use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;
use std::process::Command;

pub(crate) struct CommandSpec {
    pub(crate) name: &'static str,
    usage: &'static str,
    run: fn(Vec<OsString>) -> Result<()>,
    /// Output goes through the pager when stdout is a terminal.
    pages: bool,
    /// Shell completion offers patch names for this command's arguments.
    pub(crate) completes_patches: bool,
}

const fn spec(
    name: &'static str,
    usage: &'static str,
    run: fn(Vec<OsString>) -> Result<()>,
    pages: bool,
    completes_patches: bool,
) -> CommandSpec {
    CommandSpec {
        name,
        usage,
        run,
        pages,
        completes_patches,
    }
}

/// Every command, in the order `quilt` lists them in its usage text.
pub(crate) const COMMANDS: &[CommandSpec] = &[
    spec(
        "add",
        "quilt add [--new] [-P patch] {file} ...",
        cmd_add,
        false,
        false,
    ),
    spec(
        "annotate",
        "quilt annotate [-P patch] {file}",
        cmd_annotate,
        true,
        false,
    ),
    spec(
        "absorb",
        "quilt absorb [--staged] [--[no-]sync-git]",
        cmd_absorb,
        false,
        false,
    ),
    spec("applied", "quilt applied [patch]", cmd_applied, true, true),
    spec(
        "delete",
        "quilt delete [-r] [--backup] [patch|-n]",
        cmd_delete,
        false,
        true,
    ),
    spec(
        "diff",
        "quilt diff [-p n|-p ab] [-u|-U num|-c|-C num] [--combine patch|-z] [-R] [-P patch] [--snapshot] [--diff=utility] [--no-timestamps] [--no-index] [--sort] [--color[=always|auto|never]] [file ...]",
        cmd_diff,
        true,
        true,
    ),
    spec("edit", "quilt edit file ...", cmd_edit, false, false),
    spec(
        "files",
        "quilt files [-v] [-a] [-l] [--combine patch] [patch]",
        cmd_files,
        true,
        true,
    ),
    spec(
        "fold",
        "quilt fold [-R] [-q] [-f] [-p strip-level]",
        cmd_fold,
        false,
        false,
    ),
    spec("fork", "quilt fork [new_name]", cmd_fork, false, true),
    spec(
        "graph",
        "quilt graph [--all] [--reduce] [--lines[=num]] [--edge-labels=files] [-T ps] [patch]",
        cmd_graph,
        false,
        true,
    ),
    spec(
        "grep",
        "quilt grep [-h|options] {pattern}",
        cmd_grep,
        false,
        false,
    ),
    spec(
        "header",
        "quilt header [-a|-r|-e] [--backup] [--strip-diffstat] [--strip-trailing-whitespace] [patch]",
        cmd_header,
        true,
        true,
    ),
    spec(
        "import",
        "quilt import [-p num] [-R] [-P patch] [-f] [-d {o|a|n}] patchfile ...",
        cmd_import,
        false,
        false,
    ),
    spec(
        "mail",
        "quilt mail {--mbox file|--send} [-m text] [-M file] [--prefix prefix] [--sender ...] [--from ...] [--to ...] [--cc ...] [--bcc ...] [--subject ...] [--reply-to message] [--charset ...] [--signature file] [first_patch [last_patch]]",
        cmd_mail,
        false,
        true,
    ),
    spec("new", "quilt new [-p n] {patchname}", cmd_new, false, false),
    spec("next", "quilt next [patch]", cmd_next, false, true),
    spec(
        "patches",
        "quilt patches [-v] [--color[=always|auto|never]] {file} [files...]",
        cmd_patches,
        true,
        false,
    ),
    spec(
        "pop",
        "quilt pop [-afRqv] [--refresh] [num|patch]",
        cmd_pop,
        false,
        true,
    ),
    spec(
        "previous",
        "quilt previous [patch]",
        cmd_previous,
        false,
        true,
    ),
    spec(
        "push",
        "quilt push [-afqvm] [--fuzz=N] [--merge[=merge|diff3]] [--leave-rejects] [--color[=always|auto|never]] [--refresh] [num|patch]",
        cmd_push,
        false,
        true,
    ),
    spec(
        "refresh",
        "quilt refresh [-p n|-p ab] [-u|-U num|-c|-C num] [-z[new_name]] [-f] [--no-timestamps] [--no-index] [--diffstat] [--sort] [--backup] [--strip-trailing-whitespace] [patch]",
        cmd_refresh,
        false,
        true,
    ),
    spec(
        "remove",
        "quilt remove [-P patch] {file} ...",
        cmd_remove,
        false,
        false,
    ),
    spec(
        "rename",
        "quilt rename [-P patch] new_name",
        cmd_rename,
        false,
        true,
    ),
    spec(
        "revert",
        "quilt revert [-P patch] {file} ...",
        cmd_revert,
        false,
        false,
    ),
    spec(
        "series",
        "quilt series [--color[=always|auto|never]] [-v]",
        cmd_series,
        true,
        true,
    ),
    spec(
        "setup",
        "quilt setup [-d path-prefix] [--sourcedir dir] {seriesfile}",
        cmd_setup,
        false,
        false,
    ),
    spec(
        "snapshot",
        "quilt snapshot [-d]",
        cmd_snapshot,
        false,
        false,
    ),
    spec(
        "scratch",
        "quilt scratch init [--force]",
        cmd_scratch,
        false,
        false,
    ),
    spec("top", "quilt top", cmd_top, false, false),
    spec(
        "unapplied",
        "quilt unapplied [patch]",
        cmd_unapplied,
        true,
        true,
    ),
    spec("upgrade", "quilt upgrade", cmd_upgrade, false, false),
];

/// The `Usage:` line for a command, for argument errors.
pub(crate) fn usage(command: &str) -> String {
    let spec = COMMANDS
        .iter()
        .find(|spec| spec.name == command)
        .expect("usage() is only called with known command names");
    format!("Usage: {}", spec.usage)
}

/// An error that exits with a specific status instead of 1. Like any other
/// error, an empty message means the details were already printed.
#[derive(Debug)]
pub(crate) struct ExitStatus {
    pub(crate) code: i32,
    pub(crate) message: String,
}

impl fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ExitStatus {}

/// Entry point shared by the `q` and `quilt` binaries.
pub fn main() -> ! {
    let code = match run() {
        Ok(()) => 0,
        Err(err) => {
            if !err.to_string().is_empty() {
                eprintln!("{err:#}");
            }
            err.downcast_ref::<ExitStatus>()
                .map_or(1, |status| status.code)
        }
    };
    std::process::exit(code)
}

fn run() -> Result<()> {
    let mut args: Vec<OsString> = env::args_os().skip(1).collect();
    match args.first().and_then(|arg| arg.to_str()) {
        Some("completion") => return completion::cmd_completion(args.split_off(1)),
        Some("__complete") => return completion::cmd_complete(args.split_off(1)),
        _ => {}
    }
    let (quiltrc, mut args) = strip_global_options(args);
    if args.is_empty() {
        return print_usage();
    }
    if args.len() == 1 && args[0] == "--version" {
        println!("{}", env!("Q_VERSION"));
        return Ok(());
    }

    let name = args.remove(0).to_string_lossy().into_owned();
    let command = match resolve_command(&name) {
        Ok(Some(command)) => command,
        Ok(None) => return print_usage(),
        Err(err) => return Err(err),
    };
    load_quiltrc(quiltrc)?;
    let args = prepend_command_args(command.name, args)?;
    if args.iter().take_while(|a| *a != "--").any(|a| a == "-h") {
        println!("{}", usage(command.name));
        return Ok(());
    }
    let stdout_is_terminal = terminal::remember_stdout();
    if command.pages
        && stdout_is_terminal
        && let Some(pager) = pager_command()
    {
        return run_with_pager(&pager, || (command.run)(args));
    }
    (command.run)(args)
}

/// Resolve a possibly abbreviated command name. `Ok(None)` means no command
/// matches; an ambiguous abbreviation is an error listing the candidates.
pub(crate) fn resolve_command(name: &str) -> Result<Option<&'static CommandSpec>> {
    if let Some(exact) = COMMANDS.iter().find(|spec| spec.name == name) {
        return Ok(Some(exact));
    }
    let matches: Vec<_> = COMMANDS
        .iter()
        .filter(|spec| spec.name.starts_with(name))
        .collect();
    match matches.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some(one)),
        many => {
            let names: Vec<_> = many.iter().map(|spec| spec.name).collect();
            bail!("{name} : {}", names.join(" "))
        }
    }
}

fn pager_command() -> Option<String> {
    let pager = env::var("QUILT_PAGER")
        .or_else(|_| env::var("GIT_PAGER"))
        .or_else(|_| env::var("PAGER"))
        .unwrap_or_else(|_| "less -R".to_string());
    let pager = pager.trim();
    (!pager.is_empty() && pager != "cat").then(|| pager.to_string())
}

/// Run `run` with stdout redirected into `pager`. If the pipe can't be set
/// up, run unpaged.
#[cfg(unix)]
fn run_with_pager(pager: &str, run: impl FnOnce() -> Result<()>) -> Result<()> {
    use std::os::fd::AsRawFd;
    use std::process::Stdio;

    let Ok((reader, writer)) = std::io::pipe() else {
        return run();
    };
    let less = env::var("LESS")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "FRSX".to_string());
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(pager)
        .stdin(Stdio::from(reader))
        .env("LESS", less)
        .spawn()
        .with_context(|| format!("failed to start pager {pager}"))?;

    // Point our stdout at the pipe, keeping the original to restore later.
    let stdout_fd = libc::STDOUT_FILENO;
    // SAFETY: dup(2) on stdout, which is open; the result is checked.
    let saved = unsafe { libc::dup(stdout_fd) };
    // SAFETY: both descriptors are open; the result is checked.
    let redirected = saved >= 0 && unsafe { libc::dup2(writer.as_raw_fd(), stdout_fd) } >= 0;
    drop(writer);
    if !redirected {
        if saved >= 0 {
            // SAFETY: `saved` is our own duplicate and is not used again.
            unsafe { libc::close(saved) };
        }
        let _ = child.wait();
        return run();
    }

    let result = run();
    let _ = std::io::Write::flush(&mut std::io::stdout());
    // SAFETY: restores the stdout saved above; closing it hands EOF to the pager.
    unsafe {
        libc::dup2(saved, stdout_fd);
        libc::close(saved);
    }
    let status = child.wait()?;
    if !status.success() && result.is_ok() {
        bail!("pager failed");
    }
    result
}

#[cfg(not(unix))]
fn run_with_pager(_pager: &str, run: impl FnOnce() -> Result<()>) -> Result<()> {
    run()
}

/// Which quiltrc to load, from `--quiltrc`.
enum Quiltrc {
    /// `$QUILTRC`, `~/.quiltrc` or `/etc/quilt.quiltrc`.
    Default,
    /// `--quiltrc -`
    Skip,
    File(PathBuf),
}

impl Quiltrc {
    fn from_arg(value: Option<PathBuf>) -> Self {
        match value {
            Some(path) if path.as_os_str() != "-" => Self::File(path),
            _ => Self::Skip,
        }
    }
}

fn strip_global_options(args: Vec<OsString>) -> (Quiltrc, Vec<OsString>) {
    let mut quiltrc = Quiltrc::Default;
    let mut out = Vec::new();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        let text = arg.to_string_lossy();
        if text == "--quiltrc" {
            quiltrc = Quiltrc::from_arg(iter.next().map(PathBuf::from));
        } else if let Some(value) = text.strip_prefix("--quiltrc=") {
            quiltrc = Quiltrc::from_arg(Some(PathBuf::from(value)));
        } else if text != "--trace" && text != "--trace=verbose" {
            out.push(arg);
        }
    }
    (quiltrc, out)
}

/// Source the quiltrc in bash and import the variables it exports.
fn load_quiltrc(quiltrc: Quiltrc) -> Result<()> {
    let path = match quiltrc {
        Quiltrc::Default => default_quiltrc(),
        Quiltrc::Skip => None,
        Quiltrc::File(path) => Some(path),
    };
    let Some(path) = path else {
        return Ok(());
    };
    if !path.exists() {
        return Ok(());
    }
    let output = Command::new("bash")
        .arg("-c")
        .arg("set -a; . \"$1\"; env -0")
        .arg("quilt-rc")
        .arg(&path)
        .output()
        .with_context(|| format!("failed to read quiltrc {}", path.display()))?;
    if !output.status.success() {
        bail!("failed to read quiltrc {}", path.display());
    }
    for item in output.stdout.split(|b| *b == 0) {
        let Some(pos) = item.iter().position(|b| *b == b'=') else {
            continue;
        };
        let name = String::from_utf8_lossy(&item[..pos]);
        let value = String::from_utf8_lossy(&item[pos + 1..]);
        // SAFETY: q is single-threaded at this point; nothing else reads the
        // environment concurrently.
        unsafe { env::set_var(name.as_ref(), value.as_ref()) };
    }
    Ok(())
}

fn default_quiltrc() -> Option<PathBuf> {
    if let Some(path) = env::var_os("QUILTRC") {
        return Some(PathBuf::from(path));
    }
    let home = env::var_os("HOME").map(|home| PathBuf::from(home).join(".quiltrc"));
    home.into_iter()
        .chain([PathBuf::from("/etc/quilt.quiltrc")])
        .find(|path| path.exists())
}

/// Prepend `QUILT_<COMMAND>_ARGS` to the command line.
fn prepend_command_args(command: &str, args: Vec<OsString>) -> Result<Vec<OsString>> {
    let var = format!("QUILT_{}_ARGS", command.to_ascii_uppercase());
    let mut out: Vec<OsString> = crate::config::env_args(&var)?
        .into_iter()
        .map(OsString::from)
        .collect();
    out.extend(args);
    Ok(out)
}

/// Print the command list. Exits non-zero, like quilt.
fn print_usage() -> Result<()> {
    println!("Usage: quilt [--trace[=verbose]] [--quiltrc=XX] command [-h] ...");
    println!("       quilt --version");
    println!("\nCommands are:");
    let names: Vec<_> = COMMANDS.iter().map(|spec| spec.name).collect();
    for chunk in names.chunks(6) {
        println!("\t{}", chunk.join("\t"));
    }
    bail!("")
}

#[cfg(test)]
mod tests {
    use super::resolve_command;

    #[test]
    fn resolves_unambiguous_abbreviation() {
        assert_eq!(resolve_command("ref").unwrap().unwrap().name, "refresh");
    }

    #[test]
    fn rejects_ambiguous_abbreviation() {
        assert!(resolve_command("p").is_err());
    }

    #[test]
    fn unknown_command_is_not_an_error() {
        assert!(resolve_command("frobnicate").unwrap().is_none());
    }
}
