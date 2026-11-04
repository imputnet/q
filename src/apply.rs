use crate::color::escape;
use crate::config::*;
use crate::expected::*;
use crate::fsutil::*;
use crate::patchfile::*;
use crate::state::{PatchEntry, State};
use anyhow::{Context, Result, bail};
use p2::HunkStatus;
use std::borrow::Cow;
use std::collections::HashSet;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::{NamedTempFile, tempdir};

pub struct ApplyResult {
    pub clean: bool,
    pub files: Vec<PathBuf>,
}

pub struct PushColors {
    fail: String,
    fuzz: String,
    offset: String,
    clear: String,
}

impl PushColors {
    pub fn from_env() -> Self {
        Self {
            fail: escape("patch_fail", "31"),
            fuzz: escape("patch_fuzz", "35"),
            offset: escape("patch_offs", "33"),
            clear: escape("clear", "00"),
        }
    }
}

pub struct PushOptions<'a> {
    pub force: bool,
    pub quiet: bool,
    pub leave_rejects: bool,
    pub patch_args: &'a [String],
    pub colors: Option<&'a PushColors>,
}

pub(crate) fn apply_patch(
    state: &State,
    entry: &PatchEntry,
    opts: &PushOptions,
) -> Result<ApplyResult> {
    let patch_path = patch_file_for_apply(&state.patch_path(&entry.name), entry)?;
    let files = filenames_in_patch(patch_path.path(), strip_level(entry));
    fs::create_dir_all(state.backup_dir(&entry.name))?;
    for file in &files {
        let src = state.root.join(file);
        let dst = state.backup_dir(&entry.name).join(file);
        if src.exists() {
            copy_creating_dirs(&src, &dst)?;
        } else {
            create_parent_dir(&dst)?;
            File::create(&dst)?;
        }
    }

    let input = fs::read(patch_path.path())?;
    let args = PatchFlags {
        strip: strip_level(entry),
        reverse: is_reversed(entry),
        force: opts.force,
        quiet: opts.quiet,
        extra: opts.patch_args,
    }
    .args()?;
    match apply_in_process(&state.root, &input, args, opts.colors) {
        InProcess::Applied => return Ok(ApplyResult { clean: true, files }),
        outcome if outcome.touched_files() => {
            let _ = restore_patch_files(state, &entry.name, &files, false);
        }
        _ => {}
    }

    // Fall back to GNU patch, invoked the way quilt does: unless rejects are
    // wanted, send them to a scratch file.
    let leave_rejects = opts.force || opts.leave_rejects;
    let rejects = if leave_rejects {
        None
    } else {
        Some(NamedTempFile::new()?)
    };
    let mut cmd = gnu_patch(&state.root, patch_path.path(), entry, is_reversed(entry))?;
    cmd.args(opts.patch_args);
    if opts.quiet {
        cmd.arg("-s");
    }
    if let Some(rejects) = &rejects {
        cmd.arg("-r").arg(rejects.path());
    }
    let output = cmd.output().context("failed to run patch")?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    if !leave_rejects {
        text = cleanup_patch_output(&text, opts.quiet);
    }
    print!("{}", colorize_patch_output(&text, opts.colors));
    let clean = output.status.success();
    if !clean && !opts.force {
        let _ = restore_patch(state, &entry.name, false);
        let verdict = if reverse_applies(state, entry) {
            "can be reverse-applied"
        } else {
            "does not apply (enforce with -f)"
        };
        bail!("Patch {} {verdict}", state.display_patch(&entry.name));
    }
    Ok(ApplyResult { clean, files })
}

/// `patch -pN -i FILE -f [$QUILT_PATCH_OPTS] [-R]` in `dir`, never prompting.
fn gnu_patch(dir: &Path, patch_file: &Path, entry: &PatchEntry, reverse: bool) -> Result<Command> {
    let mut cmd = patch_command();
    cmd.current_dir(dir)
        .arg(format!("-p{}", strip_level(entry)))
        .arg("-i")
        .arg(patch_file)
        .arg("-f")
        .args(env_args("QUILT_PATCH_OPTS")?)
        .stdin(Stdio::null());
    if reverse {
        cmd.arg("-R");
    }
    Ok(cmd)
}

/// quilt's `cleanup_patch_output`: when rejects go to a scratch file, don't
/// show its path; name the patched file instead (or nothing when quiet).
fn cleanup_patch_output(output: &str, quiet: bool) -> String {
    let mut out = String::with_capacity(output.len());
    let mut patching = "";
    for line in output.split_inclusive('\n') {
        if let Some(file) = line.strip_prefix("patching file ") {
            patching = file.trim_end_matches('\n');
        }
        match line.find(" -- saving rejects to ") {
            Some(idx) if quiet => {
                out.push_str(&line[..idx]);
                out.push('\n');
            }
            Some(idx) => {
                out.push_str(&line[..idx]);
                out.push_str(" -- rejects in file ");
                out.push_str(patching);
                out.push('\n');
            }
            None => out.push_str(line),
        }
    }
    out
}

/// The outcome of applying a patch in-process with p2.
pub(crate) enum InProcess {
    /// Every hunk applied cleanly.
    Applied,
    /// Some hunks failed or were already applied; files may be partly patched.
    Rejected,
    /// p2 can't apply this patch exactly as GNU patch would. With `touched`,
    /// it gave up after modifying files.
    Unsupported { touched: bool },
}

impl InProcess {
    /// Whether the working files need restoring before falling back.
    pub(crate) fn touched_files(&self) -> bool {
        matches!(self, Self::Rejected | Self::Unsupported { touched: true })
    }
}

/// Apply a patch with p2, which is much faster than spawning GNU patch. Only
/// a clean result counts; on anything else the caller falls back to GNU
/// patch, which then produces the real diagnostics. Prints p2's progress
/// report like GNU patch would, unless `args` has `-s`.
pub(crate) fn apply_in_process(
    root: &Path,
    input: &[u8],
    args: Vec<String>,
    colors: Option<&PushColors>,
) -> InProcess {
    let unsupported = InProcess::Unsupported { touched: false };
    let Ok(parsed) = p2::parse_apply_options(args) else {
        return unsupported;
    };
    let Ok(input) = std::str::from_utf8(input) else {
        return unsupported;
    };
    let input = if parsed.binary {
        Cow::Borrowed(input)
    } else {
        Cow::Owned(input.replace("\r\n", "\n"))
    };
    let Ok(patch) = p2::parse_patch(&input) else {
        return unsupported;
    };
    if let Some(format) = parsed.force_format
        && patch.files.iter().any(|file| file.format != format)
    {
        return unsupported;
    }
    let Ok(report) = p2::apply_patch(root, &patch, &parsed.options) else {
        return InProcess::Unsupported { touched: true };
    };
    let already_applied = report.files.iter().any(|file| {
        file.hunk_reports
            .iter()
            .any(|hunk| matches!(hunk.status, HunkStatus::AlreadyApplied { .. }))
    });
    if report.failed_hunks() > 0 || already_applied {
        return InProcess::Rejected;
    }
    if !parsed.quiet {
        print_fast_patch_report(&report, colors);
    }
    InProcess::Applied
}

/// The options of one `patch` run, as passed to p2 or GNU patch.
pub(crate) struct PatchFlags<'a> {
    pub(crate) strip: usize,
    pub(crate) reverse: bool,
    pub(crate) force: bool,
    pub(crate) quiet: bool,
    pub(crate) extra: &'a [String],
}

impl PatchFlags<'_> {
    /// The command line, including `$QUILT_PATCH_OPTS`.
    pub(crate) fn args(&self) -> Result<Vec<String>> {
        let mut args = vec![format!("-p{}", self.strip)];
        args.extend(env_args("QUILT_PATCH_OPTS")?);
        args.extend(self.extra.iter().cloned());
        for (enabled, flag) in [(self.reverse, "-R"), (self.force, "-f"), (self.quiet, "-s")] {
            if enabled {
                args.push(flag.to_string());
            }
        }
        Ok(args)
    }
}

fn print_fast_patch_report(report: &p2::ApplyReport, colors: Option<&PushColors>) {
    let mut out = String::new();
    for file in &report.files {
        out.push_str("patching file ");
        out.push_str(&file.path.display().to_string());
        out.push('\n');
        for hunk in &file.hunk_reports {
            match &hunk.status {
                HunkStatus::Applied {
                    requested_line,
                    applied_line,
                    fuzz,
                } => {
                    if requested_line != applied_line || *fuzz > 0 {
                        out.push_str(&format!(
                            "Hunk #{} succeeded at {}",
                            hunk.hunk, applied_line
                        ));
                        if *fuzz > 0 {
                            out.push_str(&format!(" with fuzz {fuzz}"));
                        }
                        let offset = *applied_line as isize - *requested_line as isize;
                        if offset != 0 {
                            let noun = if offset == 1 { "line" } else { "lines" };
                            out.push_str(&format!(" (offset {offset} {noun})"));
                        }
                        out.push_str(".\n");
                    }
                }
                HunkStatus::AlreadyApplied { applied_line, .. } => {
                    out.push_str(&format!(
                        "Hunk #{} already applied at {}.\n",
                        hunk.hunk, applied_line
                    ));
                }
                HunkStatus::Merged { ranges, .. } => {
                    out.push_str(&format!(
                        "Hunk #{} merged at {}.\n",
                        hunk.hunk,
                        format_hunk_ranges(ranges)
                    ));
                }
                HunkStatus::Conflict { .. } | HunkStatus::Failed { .. } => {}
            }
        }
    }
    print!("{}", colorize_patch_output(&out, colors));
}

fn format_hunk_ranges(ranges: &[(usize, usize)]) -> String {
    ranges
        .iter()
        .map(|(start, end)| {
            if start == end {
                start.to_string()
            } else {
                format!("{start}-{end}")
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn colorize_patch_output(output: &str, colors: Option<&PushColors>) -> String {
    let Some(colors) = colors else {
        return output.to_string();
    };
    let mut out = String::with_capacity(output.len());
    for line in output.split_inclusive('\n') {
        let (line, newline) = line
            .strip_suffix('\n')
            .map(|line| (line, "\n"))
            .unwrap_or((line, ""));
        if patch_failure_line(line) {
            out.push_str(&colors.fail);
            out.push_str(line);
            out.push_str(&colors.clear);
        } else if line.ends_with("already applied") {
            out.push_str(&colors.fuzz);
            out.push_str(line);
            out.push_str(&colors.clear);
        } else if line.starts_with("Hunk ") {
            out.push_str(&colorize_hunk_line(line, colors));
        } else {
            out.push_str(line);
        }
        out.push_str(newline);
    }
    out
}

fn patch_failure_line(line: &str) -> bool {
    line.contains("FAILED")
        || line.contains("hunk ignored")
        || line.contains("hunks ignored")
        || line.contains("can't find file")
        || line.contains("NOT MERGED")
        || line.contains("file ") && line.contains(" already exists")
}

fn colorize_hunk_line(line: &str, colors: &PushColors) -> String {
    let mut out = line.to_string();
    if let Some(start) = line.find(" with fuzz ") {
        let end = line[start..]
            .find(',')
            .map(|end| start + end)
            .or_else(|| line[start..].find(" and ").map(|end| start + end))
            .unwrap_or(line.len());
        out.replace_range(
            0..end,
            &format!("{}{}{}", colors.fuzz, &line[..end], colors.clear),
        );
    } else if let Some(start) = line.find("offset ")
        && let Some(end) = offset_phrase_end(&line[start..]).map(|end| start + end)
    {
        out.replace_range(
            start..end,
            &format!("{}{}{}", colors.offset, &line[start..end], colors.clear),
        );
    }
    out
}

fn offset_phrase_end(text: &str) -> Option<usize> {
    let rest = text.strip_prefix("offset ")?;
    let mut seen_digit = false;
    let mut idx = "offset ".len();
    for ch in rest.chars() {
        if ch == '-' || ch.is_ascii_digit() {
            if ch.is_ascii_digit() {
                seen_digit = true;
            }
            idx += ch.len_utf8();
        } else {
            break;
        }
    }
    if !seen_digit {
        return None;
    }
    let suffix = &text[idx..];
    if let Some(rest) = suffix.strip_prefix(" lines") {
        Some(idx + " lines".len() + trailing_s_len(rest))
    } else {
        suffix
            .strip_prefix(" line")
            .map(|rest| idx + " line".len() + trailing_s_len(rest))
    }
}

fn trailing_s_len(text: &str) -> usize {
    text.strip_prefix('s').map(|_| 1).unwrap_or(0)
}

/// Whether the patch would apply in reverse, i.e. is already applied.
fn reverse_applies(state: &State, entry: &PatchEntry) -> bool {
    let Ok(patch_path) = patch_file_for_apply(&state.patch_path(&entry.name), entry) else {
        return false;
    };
    let reverse = !is_reversed(entry);
    if let Ok(input) = fs::read(patch_path.path())
        && let Ok(mut args) = (PatchFlags {
            strip: strip_level(entry),
            reverse,
            force: false,
            quiet: true,
            extra: &[],
        })
        .args()
    {
        args.push("--dry-run".to_string());
        match apply_in_process(&state.root, &input, args, None) {
            InProcess::Applied => return true,
            InProcess::Rejected => return false,
            InProcess::Unsupported { .. } => {}
        }
    }
    let Ok(mut cmd) = gnu_patch(&state.root, patch_path.path(), entry, reverse) else {
        return false;
    };
    cmd.arg("--dry-run")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub(crate) fn restore_patch(state: &State, patch: &str, verbose: bool) -> Result<()> {
    let files = state.tracked_files(patch)?;
    restore_patch_files(state, patch, &files, verbose)
}

pub(crate) fn restore_patch_files(
    state: &State,
    patch: &str,
    files: &[PathBuf],
    verbose: bool,
) -> Result<()> {
    for file in files {
        let backup = state.backup_dir(patch).join(file);
        let dest = state.root.join(file);
        create_parent_dir(&dest)?;
        if backup.metadata()?.len() == 0 {
            if verbose {
                println!("Removing {}", file.display());
            }
            let _ = fs::remove_file(&dest);
        } else {
            if verbose {
                println!("Restoring {}", file.display());
            }
            copy_keeping_hard_links(&backup, &dest)?;
        }
    }
    Ok(())
}

/// Whether popping `entry` would lose no changes: the working tree matches
/// what applying the patch to its backups produces. `use_expected_state`
/// trusts a current recorded expected state outright (for the top patch).
pub(crate) fn patch_removes_cleanly(
    state: &State,
    entry: &PatchEntry,
    use_expected_state: bool,
    tracked: &[PathBuf],
) -> Result<bool> {
    let patch = &entry.name;
    if expected_state_is_current(state, patch) {
        if use_expected_state {
            return expected_state_matches(state, patch, tracked);
        }
        // A match is conclusive for any patch in the stack and saves
        // re-applying it below; a mismatch still gets the full check.
        if expected_state_matches(state, patch, tracked)? {
            return Ok(true);
        }
    }
    if patch_body(&state.patch_path(patch))?.is_empty() {
        return Ok(tracked.is_empty());
    }
    let patch_path = patch_file_for_apply(&state.patch_path(patch), entry)?;
    let input = fs::read(patch_path.path())?;

    let workdir = tempdir()?;
    populate_pop_workdir(state, patch, tracked, workdir.path())?;
    let args = PatchFlags {
        strip: strip_level(entry),
        reverse: is_reversed(entry),
        force: true,
        quiet: true,
        extra: &[],
    }
    .args()?;
    let outcome = apply_in_process(workdir.path(), &input, args, None);
    if !matches!(outcome, InProcess::Applied) {
        if outcome.touched_files() {
            fs::remove_dir_all(workdir.path())?;
            fs::create_dir_all(workdir.path())?;
            populate_pop_workdir(state, patch, tracked, workdir.path())?;
        }
        let status = gnu_patch(workdir.path(), patch_path.path(), entry, is_reversed(entry))?
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("failed to run patch")?;
        if !status.success() {
            return Ok(false);
        }
    }

    let mut files: HashSet<PathBuf> = tracked.iter().cloned().collect();
    files.extend(filenames_in_text_order(
        &String::from_utf8_lossy(&input),
        strip_level(entry),
    ));
    for file in files {
        if !files_match(&workdir.path().join(&file), &state.root.join(&file))? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn populate_pop_workdir(
    state: &State,
    patch: &str,
    tracked: &[PathBuf],
    workdir: &Path,
) -> Result<()> {
    for file in tracked {
        let backup = state.backup_dir(patch).join(file);
        if backup.metadata()?.len() == 0 {
            continue;
        }
        let dst = workdir.join(file);
        copy_creating_dirs(&backup, &dst)?;
    }
    Ok(())
}
