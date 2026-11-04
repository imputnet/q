use crate::color::{color_arg, escape};
use crate::config::*;
use crate::expected::*;
use crate::patchfile::*;
use crate::state::State;
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local};
use lexopt::ValueExt;
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug)]
pub struct DiffOpts {
    pub strip: Strip,
    pub reverse: bool,
    pub no_index: bool,
    pub no_timestamps: bool,
    pub context: bool,
    pub context_lines: Option<String>,
    pub relative: bool,
    pub sort: bool,
    pub exclude: Vec<PathBuf>,
    pub use_expected_for: Vec<PathBuf>,
    pub diff_utility: Option<String>,
    pub color: bool,
    /// Read the new side of each file from here instead of the working tree.
    pub new_root: Option<PathBuf>,
}

impl Default for DiffOpts {
    fn default() -> Self {
        Self {
            strip: Strip::One,
            reverse: false,
            no_index: env_flag("QUILT_NO_DIFF_INDEX"),
            no_timestamps: env_flag("QUILT_NO_DIFF_TIMESTAMPS"),
            context: false,
            context_lines: None,
            relative: false,
            sort: false,
            exclude: Vec::new(),
            use_expected_for: Vec::new(),
            diff_utility: None,
            color: false,
            new_root: None,
        }
    }
}

/// How a generated diff names files: quilt's `-p0`, `-p1` and `-p ab`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Strip {
    /// `file.orig` / `file`
    Zero,
    /// `dir.orig/file` / `dir/file`, named after the tree's root directory
    One,
    /// `a/file` / `b/file`
    Ab,
}

impl Strip {
    pub(crate) fn parse(level: &str, command: &str) -> Result<Self> {
        match level {
            "0" => Ok(Self::Zero),
            "1" => Ok(Self::One),
            "ab" => Ok(Self::Ab),
            _ => bail!(
                "Cannot {command} patches with -p{level}, please specify -p0, -p1, or -pab instead"
            ),
        }
    }

    /// The strip level that applies such a diff.
    pub(crate) fn level(self) -> usize {
        match self {
            Self::Zero => 0,
            Self::One | Self::Ab => 1,
        }
    }
}

impl DiffOpts {
    /// Apply one of the diff format options `diff`, `refresh` and
    /// `QUILT_REFRESH_ARGS` share, named like `-U` or `--color`. Returns
    /// whether it was one of them.
    pub(crate) fn parse_option(
        &mut self,
        option: &str,
        parser: &mut lexopt::Parser,
        command: &str,
    ) -> Result<bool> {
        match option {
            "-u" => self.context = false,
            "-c" => self.context = true,
            "-U" | "-C" => {
                self.context = option == "-C";
                self.context_lines = Some(parser.value()?.string()?);
            }
            "--no-index" => self.no_index = true,
            "--no-timestamps" => self.no_timestamps = true,
            "--sort" => self.sort = true,
            "--color" => self.color = color_arg(parser.optional_value(), command)?,
            _ => return Ok(false),
        }
        Ok(true)
    }
}

/// An option argument's name as `-x` or `--name`; empty for a value.
pub(crate) fn option_name(arg: &lexopt::Arg) -> String {
    match arg {
        lexopt::Arg::Short(c) => format!("-{c}"),
        lexopt::Arg::Long(name) => format!("--{name}"),
        lexopt::Arg::Value(_) => String::new(),
    }
}

/// Write `patch`'s diff for `files` (narrowed to `selected`, if any) to `out`.
pub(crate) fn emit_diff<W: Write>(
    state: &State,
    patch: &str,
    files: &[PathBuf],
    selected: &[PathBuf],
    opts: &DiffOpts,
    mut out: W,
) -> Result<()> {
    let mut files = files.to_vec();
    if !opts.sort {
        let patch_order = filenames_in_patch_order(&state.patch_path(patch), opts.strip.level());
        let file_set: HashSet<_> = files.iter().cloned().collect();
        let mut seen = HashSet::new();
        let mut ordered = Vec::new();
        for file in patch_order {
            if file_set.contains(&file) && seen.insert(file.clone()) {
                ordered.push(file);
            }
        }
        for file in &files {
            if seen.insert(file.clone()) {
                ordered.push(file.clone());
            }
        }
        files = ordered;
    }
    if !selected.is_empty() {
        let selected: HashSet<_> = selected.iter().cloned().collect();
        files.retain(|f| selected.contains(f));
    }
    if !opts.exclude.is_empty() {
        let exclude: HashSet<_> = opts.exclude.iter().cloned().collect();
        files.retain(|f| !exclude.contains(f));
    }
    let colors = opts.color.then(DiffColors::from_env);
    let deleted_files = expected_deleted(state, patch)?;
    // Spawning `diff` per file dominates refresh time, so run them
    // concurrently and then emit the results in file order.
    let mut jobs = Vec::new();
    let mut prep_error = None;
    for file in files {
        match prepare_diff_job(state, patch, opts, &deleted_files, file) {
            Ok(Some(job)) => jobs.push(job),
            Ok(None) => {}
            Err(err) => {
                prep_error = Some(err);
                break;
            }
        }
    }
    let outputs = run_diff_jobs(&jobs);
    for (job, output) in jobs.iter().zip(outputs) {
        let file = &job.file;
        let output = output.context("failed to run diff")?;
        if output.status.code() == Some(0) {
            continue;
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr)
            .replace(&format!("{}/", state.root.display()), "");
        if output.status.code() != Some(1) {
            if !stdout.is_empty() {
                eprint!("{stdout}");
            }
            if !stderr.is_empty() {
                eprint!("{stderr}");
            }
            bail!("Diff failed on file '{}', aborting", file.display());
        }
        if stdout.starts_with("Binary files ") || stdout.starts_with("Files ") {
            if opts.no_index {
                out.write_all(&output.stdout)?;
            }
            bail!("Diff failed on file '{}', aborting", file.display());
        }
        if !opts.no_index {
            write_diff_line(
                &mut out,
                &format!("Index: {}\n", job.index_label),
                colors.as_ref(),
            )?;
            write_diff_line(
                &mut out,
                "===================================================================\n",
                colors.as_ref(),
            )?;
        }
        write_diff_bytes(&mut out, &output.stdout, colors.as_ref())?;
    }
    if let Some(err) = prep_error {
        return Err(err);
    }
    Ok(())
}

struct DiffJob {
    file: PathBuf,
    index_label: String,
    args: Vec<OsString>,
}

/// Resolve one file's diff inputs. Returns `None` when there is nothing to
/// diff, or when an external `--diff` utility was run directly.
fn prepare_diff_job(
    state: &State,
    patch: &str,
    opts: &DiffOpts,
    deleted_files: &HashSet<PathBuf>,
    file: PathBuf,
) -> Result<Option<DiffJob>> {
    let expected = expected_file(state, patch, &file);
    let expected_is_deleted = deleted_files.contains(&file);
    let old = if opts.relative && expected.exists() {
        expected.clone()
    } else {
        state.backup_dir(patch).join(&file)
    };
    let use_expected = opts.use_expected_for.iter().any(|f| f == &file);
    let new = if use_expected && expected.exists() {
        expected_file(state, patch, &file)
    } else if let Some(new_root) = &opts.new_root {
        new_root.join(&file)
    } else {
        state.root.join(&file)
    };
    if opts.relative && expected_is_deleted && !new.exists() {
        return Ok(None);
    }
    let old_empty = old.metadata()?.len() == 0;
    if old_empty && !new.exists() {
        return Ok(None);
    }
    let old_arg = if old_empty {
        Path::new("/dev/null")
    } else {
        old.as_path()
    };
    let new_arg = if !new.exists() {
        Path::new("/dev/null")
    } else {
        new.as_path()
    };
    if let Some(utility) = &opts.diff_utility {
        emit_external_diff(utility, old_arg, new_arg)?;
        return Ok(None);
    }
    let (old_label, new_label) = diff_labels(
        state,
        &file,
        opts.strip,
        old_arg,
        new_arg,
        opts.reverse,
        opts.no_timestamps,
    );
    let index_label = diff_index_label(state, &file, opts.strip);
    let mut args: Vec<OsString> = env_args("QUILT_DIFF_OPTS")?
        .into_iter()
        .map(OsString::from)
        .collect();
    args.push(OsString::from(if let Some(lines) = &opts.context_lines {
        if opts.context {
            format!("-C{lines}")
        } else {
            format!("-U{lines}")
        }
    } else if opts.context {
        "-c".to_string()
    } else {
        "-u".to_string()
    }));
    args.push("--label".into());
    args.push(old_label.into());
    args.push("--label".into());
    args.push(new_label.into());
    if opts.reverse {
        args.push(new_arg.into());
        args.push(old_arg.into());
    } else {
        args.push(old_arg.into());
        args.push(new_arg.into());
    }
    Ok(Some(DiffJob {
        file,
        index_label,
        args,
    }))
}

fn run_diff_job(job: &DiffJob) -> io::Result<std::process::Output> {
    diff_command().args(&job.args).output()
}

fn run_diff_jobs(jobs: &[DiffJob]) -> Vec<io::Result<std::process::Output>> {
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(16)
        .min(jobs.len());
    if workers <= 1 {
        return jobs.iter().map(run_diff_job).collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let mut results: Vec<Option<io::Result<std::process::Output>>> =
        jobs.iter().map(|_| None).collect();
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let index = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(job) = jobs.get(index) else {
                            break done;
                        };
                        done.push((index, run_diff_job(job)));
                    }
                })
            })
            .collect();
        for handle in handles {
            for (index, result) in handle.join().expect("diff worker panicked") {
                results[index] = Some(result);
            }
        }
    });
    results
        .into_iter()
        .map(|result| result.expect("every diff job runs"))
        .collect()
}

fn emit_external_diff(utility: &str, old_arg: &Path, new_arg: &Path) -> Result<()> {
    let status = diff_command()
        .arg("-q")
        .arg(old_arg)
        .arg(new_arg)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("failed to run diff")?;
    if status.success() {
        return Ok(());
    }
    let mut parts = shlex::split(utility).unwrap_or_else(|| vec![utility.to_string()]);
    if parts.is_empty() {
        return Ok(());
    }
    let program = parts.remove(0);
    let status = Command::new(program)
        .args(parts)
        .arg(old_arg)
        .arg(new_arg)
        .status()
        .with_context(|| format!("failed to run diff utility {utility}"))?;
    if !status.success() {
        bail!("Diff failed, aborting");
    }
    Ok(())
}

pub(crate) struct DiffColors {
    header: String,
    add: String,
    remove: String,
    modify: String,
    hunk: String,
    cctx: String,
    clear: String,
}

impl DiffColors {
    pub(crate) fn from_env() -> Self {
        Self {
            header: escape("diff_hdr", "32"),
            add: escape("diff_add", "36"),
            remove: escape("diff_rem", "35"),
            modify: escape("diff_mod", "35"),
            hunk: escape("diff_hunk", "33"),
            cctx: escape("diff_cctx", "33"),
            clear: escape("clear", "00"),
        }
    }
}

pub(crate) fn write_diff_bytes<W: Write>(
    out: &mut W,
    bytes: &[u8],
    colors: Option<&DiffColors>,
) -> Result<()> {
    if colors.is_none() {
        out.write_all(bytes)?;
        return Ok(());
    }
    let text = String::from_utf8_lossy(bytes);
    for line in text.split_inclusive('\n') {
        write_diff_line(out, line, colors)?;
    }
    Ok(())
}

pub(crate) fn write_diff_line<W: Write>(
    out: &mut W,
    line: &str,
    colors: Option<&DiffColors>,
) -> Result<()> {
    let Some(colors) = colors else {
        out.write_all(line.as_bytes())?;
        return Ok(());
    };
    let color = if line.starts_with("Index:")
        || line.starts_with("--- ")
        || line.starts_with("+++ ")
        || line.starts_with("*** ")
    {
        Some(&colors.header)
    } else if line.starts_with("@@ ") {
        Some(&colors.hunk)
    } else if line.starts_with("***************") {
        Some(&colors.cctx)
    } else if line.starts_with('+') {
        Some(&colors.add)
    } else if line.starts_with('-') {
        Some(&colors.remove)
    } else if line.starts_with('!') {
        Some(&colors.modify)
    } else {
        None
    };
    if let Some(color) = color {
        write!(out, "{color}{line}{}", colors.clear)?;
    } else {
        out.write_all(line.as_bytes())?;
    }
    Ok(())
}

fn diff_labels(
    state: &State,
    file: &Path,
    strip: Strip,
    old_arg: &Path,
    new_arg: &Path,
    reverse: bool,
    no_timestamps: bool,
) -> (String, String) {
    let old_base = if old_arg == Path::new("/dev/null") {
        "/dev/null".to_string()
    } else if new_arg == Path::new("/dev/null") && strip == Strip::Zero {
        file.display().to_string()
    } else {
        label_old(state, file, strip)
    };
    let new_base = if new_arg == Path::new("/dev/null") {
        "/dev/null".to_string()
    } else {
        label_new(state, file, strip)
    };
    let old_timestamp_path = if reverse { new_arg } else { old_arg };
    let new_timestamp_path = if reverse { old_arg } else { new_arg };
    (
        diff_label_with_timestamp(&old_base, old_timestamp_path, no_timestamps),
        diff_label_with_timestamp(&new_base, new_timestamp_path, no_timestamps),
    )
}

fn label_old(state: &State, file: &Path, strip: Strip) -> String {
    match strip {
        Strip::Zero => format!("{}.orig", file.display()),
        Strip::Ab => format!("a/{}", file.display()),
        Strip::One => format!(
            "{}.orig/{}",
            state.root.file_name().unwrap_or_default().to_string_lossy(),
            file.display()
        ),
    }
}

fn label_new(state: &State, file: &Path, strip: Strip) -> String {
    match strip {
        Strip::Zero => file.display().to_string(),
        Strip::Ab => format!("b/{}", file.display()),
        Strip::One => format!(
            "{}/{}",
            state.root.file_name().unwrap_or_default().to_string_lossy(),
            file.display()
        ),
    }
}

pub fn diff_index_label(state: &State, file: &Path, strip: Strip) -> String {
    label_new(state, file, strip)
}

fn quote_diff_label(label: &str) -> String {
    if label != "/dev/null" && label.contains(' ') {
        format!("\"{label}\"")
    } else {
        label.to_string()
    }
}

fn diff_label_with_timestamp(label: &str, path: &Path, no_timestamps: bool) -> String {
    let label = quote_diff_label(label);
    if no_timestamps {
        return label;
    }
    let timestamp = if path == Path::new("/dev/null") {
        "1970-01-01 00:00:00.000000000 +0000".to_string()
    } else {
        fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .map(|modified| {
                let datetime: DateTime<Local> = DateTime::from(modified);
                datetime.format("%Y-%m-%d %H:%M:%S%.9f %z").to_string()
            })
            .unwrap_or_else(|_| "1970-01-01 00:00:00.000000000 +0000".to_string())
    };
    format!("{label}\t{timestamp}")
}
