use super::refresh::refresh_patch_for_files;
use crate::cli::usage;
use crate::expected::*;
use crate::fsutil::{copy_creating_dirs, create_parent_dir};
use crate::state::State;
use anyhow::{Context, Result, bail};
use lexopt::prelude::*;
use std::collections::HashMap;
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::path::Component;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

pub(crate) fn cmd_absorb(args: Vec<OsString>) -> Result<()> {
    let mut sync_git = false;
    let mut no_sync_git = false;
    let mut parser = lexopt::Parser::from_args(args);
    while let Some(arg) = parser.next()? {
        match arg {
            Long("staged") => {}
            Long("sync-git") => sync_git = true,
            Long("no-sync-git") => no_sync_git = true,
            _ => bail!("{}", usage("absorb")),
        }
    }
    if sync_git && no_sync_git {
        bail!("Options --sync-git and --no-sync-git are mutually exclusive");
    }

    let state = State::discover(false)?;
    state.require_series()?;
    let patch = state.top()?;
    ensure_git_root_matches_state(&state)?;
    let scratch = git_scratch_enabled(&state)?;
    if sync_git && !scratch {
        bail!("--sync-git requires local git config q.scratch=true");
    }
    let sync_git = sync_git || scratch && !no_sync_git;

    let staged = git_staged_paths(&state)?;
    if staged.is_empty() {
        bail!("No staged changes");
    }

    for file in staged.keys() {
        ensure_absorb_backup(&state, &patch, file)?;
    }

    let tracked = state.tracked_files(&patch)?;
    let overlay = tempdir()?;
    seed_absorb_overlay(&state, &patch, &tracked, &staged, overlay.path())?;
    refresh_patch_for_files(&state, &patch, &tracked, Some(overlay.path()))?;

    if sync_git {
        git_sync_scratch(&state)?;
    } else {
        git_reset_paths(&state, staged.keys())?;
    }
    Ok(())
}

pub(crate) fn cmd_scratch(args: Vec<OsString>) -> Result<()> {
    let mut force = false;
    let mut subcommand = None::<String>;
    let mut parser = lexopt::Parser::from_args(args);
    while let Some(arg) = parser.next()? {
        match arg {
            Long("force") => force = true,
            Value(v) => subcommand = Some(v.to_string_lossy().into_owned()),
            _ => bail!("{}", usage("scratch")),
        }
    }
    if subcommand.as_deref() != Some("init") {
        bail!("{}", usage("scratch"));
    }

    let cwd = env::current_dir()?;
    let git = Git(&cwd);
    if git.toplevel()?.is_some() {
        if !force {
            bail!("Git repository already exists; rerun with --force to mark it as q scratch");
        }
        git.run(["config", "q.scratch", "true"])?;
        println!("Marked existing Git repository as q scratch");
        return Ok(());
    }

    append_gitignore_line(&cwd.join(".gitignore"), ".pc")?;
    git.run(["init"])?;
    ensure_git_identity(&git)?;
    git.run(["config", "q.scratch", "true"])?;
    git.run(["add", "."])?;
    git.run(["commit", "-m", "base"])?;
    println!("Initialized q scratch Git repository");
    Ok(())
}

enum StagedContent {
    Present(Vec<u8>),
    Deleted,
}

fn ensure_git_root_matches_state(state: &State) -> Result<()> {
    let git_root = Git(&state.root)
        .toplevel()?
        .context("not in a git repository")?;
    let git_root = git_root.canonicalize().unwrap_or(git_root);
    let state_root = state
        .root
        .canonicalize()
        .unwrap_or_else(|_| state.root.clone());
    if git_root != state_root {
        bail!(
            "Git root {} does not match quilt root {}",
            git_root.display(),
            state.root.display()
        );
    }
    Ok(())
}

fn git_staged_paths(state: &State) -> Result<HashMap<PathBuf, StagedContent>> {
    let output = Git(&state.root).output([
        "diff",
        "--cached",
        "--name-status",
        "-z",
        "--find-renames",
        "--find-copies",
        "--find-copies-harder",
        "--diff-filter=ACMRTD",
    ])?;
    let parts: Vec<&[u8]> = output
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .collect();
    let mut idx = 0;
    let mut out = HashMap::new();
    while idx < parts.len() {
        let status = String::from_utf8_lossy(parts[idx]).to_string();
        idx += 1;
        if status.starts_with('R') || status.starts_with('C') {
            bail!("q absorb does not support staged renames or copies yet");
        }
        let path = parts
            .get(idx)
            .with_context(|| "unexpected git staged path output")?;
        idx += 1;
        let path = git_path(path)?;
        ensure_absorbable_git_path(state, &path)?;
        let content = if status.starts_with('D') {
            StagedContent::Deleted
        } else {
            StagedContent::Present(
                Git(&state.root).output(["show", &format!(":{}", path.display())])?,
            )
        };
        out.insert(path, content);
    }
    Ok(out)
}

fn git_path(bytes: &[u8]) -> Result<PathBuf> {
    let path = PathBuf::from(String::from_utf8_lossy(bytes).into_owned());
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        bail!("refusing unsafe git path {}", path.display());
    }
    Ok(path)
}

fn ensure_absorbable_git_path(state: &State, path: &Path) -> Result<()> {
    let abs = state.root.join(path);
    if abs.starts_with(&state.pc_dir) || abs.starts_with(&state.patches_dir) {
        bail!("Cannot absorb file {} below quilt metadata", path.display());
    }
    Ok(())
}

fn ensure_absorb_backup(state: &State, patch: &str, file: &Path) -> Result<()> {
    let backup = state.backup_dir(patch).join(file);
    if backup.exists() {
        return Ok(());
    }
    create_parent_dir(&backup)?;
    if let Some(src) = expected_before_top_patch(state, patch, file)? {
        if src.exists() {
            fs::copy(src, backup)?;
        } else {
            File::create(backup)?;
        }
    } else if let Some(content) =
        Git(&state.root).try_output(["show", &format!("HEAD:{}", file.display())])?
    {
        fs::write(backup, content)?;
    } else {
        File::create(backup)?;
    }
    Ok(())
}

fn expected_before_top_patch(state: &State, patch: &str, file: &Path) -> Result<Option<PathBuf>> {
    let applied = state.applied()?;
    let pos = applied
        .iter()
        .position(|candidate| candidate == patch)
        .unwrap_or(applied.len());
    for earlier in applied[..pos].iter().rev() {
        if !state
            .tracked_files(earlier)?
            .iter()
            .any(|tracked| tracked == file)
        {
            continue;
        }
        if expected_deleted(state, earlier)?.contains(file) {
            return Ok(Some(PathBuf::new()));
        }
        let expected = expected_file(state, earlier, file);
        if expected.exists() {
            return Ok(Some(expected));
        }
        let backup = state.backup_dir(earlier).join(file);
        if backup.exists() {
            return Ok(Some(backup));
        }
    }
    Ok(None)
}

fn seed_absorb_overlay(
    state: &State,
    patch: &str,
    tracked: &[PathBuf],
    staged: &HashMap<PathBuf, StagedContent>,
    overlay: &Path,
) -> Result<()> {
    let deleted = expected_deleted(state, patch)?;
    for file in tracked {
        if let Some(content) = staged.get(file) {
            write_overlay_content(overlay, file, content)?;
            continue;
        }
        if deleted.contains(file) {
            continue;
        }
        let expected = expected_file(state, patch, file);
        let src = if expected.exists() {
            expected
        } else {
            state.root.join(file)
        };
        if src.exists() {
            let dst = overlay.join(file);
            copy_creating_dirs(&src, &dst)?;
        }
    }
    Ok(())
}

fn write_overlay_content(overlay: &Path, file: &Path, content: &StagedContent) -> Result<()> {
    match content {
        StagedContent::Present(bytes) => {
            let dst = overlay.join(file);
            create_parent_dir(&dst)?;
            fs::write(dst, bytes)?;
        }
        StagedContent::Deleted => {}
    }
    Ok(())
}

fn git_reset_paths<'a>(state: &State, paths: impl Iterator<Item = &'a PathBuf>) -> Result<()> {
    let mut args = vec![OsString::from("reset"), "-q".into(), "--".into()];
    args.extend(paths.map(OsString::from));
    Git(&state.root).run(args)
}

fn git_scratch_enabled(state: &State) -> Result<bool> {
    let value = Git(&state.root).try_output(["config", "--bool", "q.scratch"])?;
    Ok(value.is_some_and(|bytes| String::from_utf8_lossy(&bytes).trim() == "true"))
}

fn git_sync_scratch(state: &State) -> Result<()> {
    let git = Git(&state.root);
    ensure_git_identity(&git)?;
    git.run(["add", "-A"])?;
    git.run(["commit", "--amend", "--no-edit"])
}

/// git in one directory, ignoring system and global config so user settings
/// (hooks, signing, aliases) can't change what q's git calls do.
struct Git<'a>(&'a Path);

impl Git<'_> {
    fn command<I, S>(&self, args: I) -> Command
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut cmd = Command::new("git");
        cmd.current_dir(self.0)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args(args);
        cmd
    }

    /// stdout, or `None` if git exits unsuccessfully.
    fn try_output<I, S>(&self, args: I) -> Result<Option<Vec<u8>>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let output = self.command(args).output().context("failed to run git")?;
        Ok(output.status.success().then_some(output.stdout))
    }

    fn output<I, S>(&self, args: I) -> Result<Vec<u8>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args: Vec<OsString> = args.into_iter().map(|a| a.as_ref().to_owned()).collect();
        self.try_output(&args)?
            .with_context(|| format!("git {} failed", args.join(OsStr::new(" ")).display()))
    }

    fn run<I, S>(&self, args: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let args: Vec<OsString> = args.into_iter().map(|a| a.as_ref().to_owned()).collect();
        let status = self.command(&args).status().context("failed to run git")?;
        if !status.success() {
            bail!("git {} failed", args.join(OsStr::new(" ")).display());
        }
        Ok(())
    }

    fn toplevel(&self) -> Result<Option<PathBuf>> {
        let output = self.try_output(["rev-parse", "--show-toplevel"])?;
        Ok(output.map(|out| PathBuf::from(String::from_utf8_lossy(&out).trim())))
    }
}

/// Commits need an identity; scratch repos may have none.
fn ensure_git_identity(git: &Git) -> Result<()> {
    if git.try_output(["config", "user.email"])?.is_none() {
        git.run(["config", "user.email", "q@example.invalid"])?;
    }
    if git.try_output(["config", "user.name"])?.is_none() {
        git.run(["config", "user.name", "q"])?;
    }
    Ok(())
}

fn append_gitignore_line(path: &Path, line: &str) -> Result<()> {
    let existing = fs::read_to_string(path).unwrap_or_default();
    if existing.lines().any(|existing| existing == line) {
        return Ok(());
    }
    let mut out = existing;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(line);
    out.push('\n');
    fs::write(path, out)?;
    Ok(())
}
