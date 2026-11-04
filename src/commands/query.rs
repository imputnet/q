use crate::cli::usage;
use crate::color::{color_arg, escape};
use crate::commands::*;
use crate::expected::*;
use crate::patchfile::*;
use crate::state::State;
use anyhow::{Context, Result, bail};
use lexopt::prelude::*;
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn cmd_top(args: Vec<OsString>) -> Result<()> {
    if !args.is_empty() {
        bail!("{}", usage("top"));
    }
    let state = State::discover(false)?;
    state.require_series()?;
    let top = state.top()?;
    println!("{}", state.display_patch(&top));
    Ok(())
}

pub(crate) fn cmd_applied(args: Vec<OsString>) -> Result<()> {
    let state = State::discover(false)?;
    state.require_series()?;
    let applied = state.applied()?;
    let upto = match args.first() {
        Some(p) => state.find_patch(&p.to_string_lossy())?,
        None => applied.last().cloned().context("No patches applied")?,
    };
    if !applied.contains(&upto) {
        bail!("Patch {} is not applied", state.display_patch(&upto));
    }
    for p in applied {
        println!("{}", state.display_patch(&p));
        if p == upto {
            break;
        }
    }
    Ok(())
}

pub(crate) fn cmd_unapplied(args: Vec<OsString>) -> Result<()> {
    let state = State::discover(false)?;
    state.require_series()?;
    let series: Vec<String> = state.entries()?.into_iter().map(|e| e.name).collect();
    let applied = state.applied()?;
    let start = match args.first() {
        Some(p) => state.find_patch(&p.to_string_lossy())?,
        None => applied.last().cloned().unwrap_or_default(),
    };
    let start_pos = if start.is_empty() {
        0
    } else {
        series
            .iter()
            .position(|p| p == &start)
            .map(|i| i + 1)
            .unwrap_or(0)
    };
    for p in &series[start_pos..] {
        if !applied.contains(p) {
            println!("{}", state.display_patch(p));
        }
    }
    Ok(())
}

pub(crate) fn cmd_series(args: Vec<OsString>) -> Result<()> {
    let mut verbose = false;
    let mut color = false;
    let mut parser = lexopt::Parser::from_args(args);
    while let Some(arg) = parser.next()? {
        match arg {
            Short('v') => verbose = true,
            Long("color") => color = color_arg(parser.optional_value(), "series")?,
            _ => {}
        }
    }
    let colors = SeriesColors::new(color);
    let state = State::discover(false)?;
    state.require_series()?;
    let applied = state.applied()?;
    let top = applied.last();
    let entries = state.entries()?;
    if verbose
        && !applied.is_empty()
        && entries
            .iter()
            .map(|e| &e.name)
            .take(applied.len())
            .ne(applied.iter())
    {
        bail!("The series file no longer matches the applied patches. Please run 'quilt pop -a'.");
    }
    let applied_set: HashSet<&str> = applied.iter().map(String::as_str).collect();
    for entry in &entries {
        let line = colors.line(&state, &entry.name, top, &applied_set, verbose);
        println!("{line}");
    }
    Ok(())
}

/// Escapes for `series` and `patches`; empty strings when not coloring.
struct SeriesColors {
    applied: String,
    top: String,
    unapplied: String,
    clear: String,
}

impl SeriesColors {
    /// How `series` and `patches` list a patch: with `-v`, marked `=` (top),
    /// `+` (applied) or blank, and colored the same way.
    fn line(
        &self,
        state: &State,
        patch: &str,
        top: Option<&String>,
        applied: &HashSet<&str>,
        verbose: bool,
    ) -> String {
        let (marker, color) = if top.is_some_and(|top| top == patch) {
            ("= ", &self.top)
        } else if applied.contains(patch) {
            ("+ ", &self.applied)
        } else {
            ("  ", &self.unapplied)
        };
        let marker = if verbose { marker } else { "" };
        format!(
            "{color}{marker}{}{}",
            state.display_patch(patch),
            self.clear
        )
    }

    fn new(enabled: bool) -> Self {
        let get = |name, default| {
            if enabled {
                escape(name, default)
            } else {
                String::new()
            }
        };
        Self {
            applied: get("series_app", "32"),
            top: get("series_top", "33"),
            unapplied: get("series_una", "00"),
            clear: get("clear", "00"),
        }
    }
}

pub(crate) fn cmd_next(args: Vec<OsString>) -> Result<()> {
    let state = State::discover(false)?;
    state.require_series()?;
    let series: Vec<String> = state.entries()?.into_iter().map(|e| e.name).collect();
    let base = match args.first() {
        Some(p) => state.find_patch(&p.to_string_lossy())?,
        None => state.applied()?.last().cloned().unwrap_or_default(),
    };
    let pos = if base.is_empty() {
        0
    } else {
        series
            .iter()
            .position(|p| p == &base)
            .map(|i| i + 1)
            .unwrap_or(series.len())
    };
    if let Some(p) = series.get(pos) {
        println!("{}", state.display_patch(p));
        Ok(())
    } else {
        bail!("No next patch")
    }
}

pub(crate) fn cmd_previous(args: Vec<OsString>) -> Result<()> {
    let state = State::discover(false)?;
    state.require_series()?;
    let series: Vec<String> = state.entries()?.into_iter().map(|e| e.name).collect();
    let base = match args.first() {
        Some(p) => state.find_patch(&p.to_string_lossy())?,
        None => state.top()?,
    };
    let pos = series.iter().position(|p| p == &base).unwrap_or(0);
    if pos > 0 {
        println!("{}", state.display_patch(&series[pos - 1]));
        Ok(())
    } else if args.is_empty() {
        Ok(())
    } else {
        bail!("No previous patch")
    }
}

pub(crate) fn cmd_files(args: Vec<OsString>) -> Result<()> {
    let mut all = false;
    let mut label = false;
    let mut verbose = false;
    let mut patch = None::<String>;
    let mut combine = None::<String>;
    let mut iter = args.into_iter();
    while let Some(a) = iter.next() {
        if a == "-a" {
            all = true;
        } else if a == "-l" {
            label = true;
        } else if a == "-v" {
            verbose = true;
        } else if a == "--combine" {
            all = true;
            combine = iter.next().map(|v| v.to_string_lossy().into_owned());
        } else if let Some(value) = a.to_string_lossy().strip_prefix("--combine=") {
            all = true;
            combine = Some(value.to_string());
        } else if !a.to_string_lossy().starts_with('-') {
            patch = Some(a.to_string_lossy().into_owned());
        }
    }
    let state = State::discover(false)?;
    let entries = state.entries()?;
    let applied = state.applied()?;
    let patches = if all || combine.is_some() {
        let last = match patch {
            Some(patch) => state.find_patch(&patch)?,
            None => applied.last().cloned().context("No patches applied")?,
        };
        applied_range(&state, &applied, combine.as_deref().unwrap_or("-"), &last)?
    } else if let Some(patch) = patch {
        vec![state.find_patch(&patch)?]
    } else {
        vec![applied_patch(&state, None)?]
    };
    for p in patches {
        if (all || combine.is_some()) && verbose && !label {
            println!("{p}");
        }
        let files = if applied.contains(&p) {
            state.tracked_files(&p)?
        } else if let Some(entry) = entries.iter().find(|entry| entry.name == p) {
            filenames_in_patch(&state.patch_path(&p), strip_level(entry))
        } else {
            Vec::new()
        };
        for file in files {
            if label {
                if verbose {
                    println!("[{}] {}", state.display_patch(&p), file.display());
                } else {
                    println!("{} {}", state.display_patch(&p), file.display());
                }
            } else if verbose {
                let backup = state.backup_dir(&p).join(&file);
                let current = state.root.join(&file);
                let marker = if backup.metadata()?.len() == 0 && current.exists() {
                    "+ "
                } else if backup.metadata()?.len() != 0 && !current.exists() {
                    "- "
                } else {
                    "  "
                };
                println!("{marker}{}", file.display());
            } else {
                println!("{}", file.display());
            }
        }
    }
    Ok(())
}

pub(crate) fn cmd_patches(args: Vec<OsString>) -> Result<()> {
    let mut verbose = false;
    let mut files = Vec::new();
    let mut color = false;
    let mut parser = lexopt::Parser::from_args(args);
    while let Some(arg) = parser.next()? {
        match arg {
            Short('v') => verbose = true,
            Long("color") => color = color_arg(parser.optional_value(), "patches")?,
            Value(v) => files.push(PathBuf::from(v)),
            _ => {}
        }
    }
    let colors = SeriesColors::new(color);
    let state = State::discover(false)?;
    state.require_series()?;
    let entries = state.entries()?;
    let applied = state.applied()?;
    let top = applied.last();
    let applied_set: HashSet<&str> = applied.iter().map(String::as_str).collect();
    let wanted: HashSet<_> = files.iter().map(|f| state.rel_from_cwd(f)).collect();
    for entry in &entries {
        let patch = &entry.name;
        let touches_wanted = if applied_set.contains(patch.as_str()) {
            state
                .tracked_files(patch)?
                .iter()
                .any(|file| wanted.contains(file))
        } else {
            filenames_in_patch(&state.patch_path(patch), strip_level(entry))
                .iter()
                .any(|file| wanted.contains(file))
        };
        if touches_wanted {
            println!("{}", colors.line(&state, patch, top, &applied_set, verbose));
        }
    }
    Ok(())
}

pub(crate) fn cmd_annotate(args: Vec<OsString>) -> Result<()> {
    let mut parser = lexopt::Parser::from_args(args);
    let mut stop_patch = None::<String>;
    let mut file = None::<PathBuf>;
    while let Some(arg) = parser.next()? {
        match arg {
            Short('P') => stop_patch = Some(parser.value()?.string()?),
            Value(v) => file = Some(PathBuf::from(v)),
            _ => bail!("{}", usage("annotate")),
        }
    }
    let state = State::discover(false)?;
    let file = state.rel_from_cwd(&file.with_context(|| usage("annotate"))?);
    let stop_patch = applied_patch(&state, stop_patch.as_deref())?;
    let applied = state.applied()?;
    let stop_index = applied
        .iter()
        .position(|p| p == &stop_patch)
        .context("Patch is not applied")?;
    let considered = &applied[..=stop_index];
    let mut touching = Vec::new();
    for patch in considered {
        if state.backup_dir(patch).join(&file).is_file() {
            touching.push(patch.clone());
        }
    }

    if touching.is_empty() {
        for line in read_lines_lossy(&state.root.join(&file))? {
            println!("\t{line}");
        }
        return Ok(());
    }

    let mut snapshots = Vec::new();
    snapshots.push(read_lines_lossy(
        &state.backup_dir(&touching[0]).join(&file),
    )?);
    for (idx, patch) in touching.iter().enumerate() {
        let next = touching
            .iter()
            .skip(idx + 1)
            .find(|p| state.backup_dir(p).join(&file).is_file());
        let path = next
            .map(|p| state.backup_dir(p).join(&file))
            .unwrap_or_else(|| state.root.join(&file));
        let lines = if patch == &stop_patch && expected_file(&state, patch, &file).exists() {
            read_lines_lossy(&expected_file(&state, patch, &file))?
        } else {
            read_lines_lossy(&path)?
        };
        snapshots.push(lines);
    }

    let mut annotations = vec![String::new(); snapshots[0].len()];
    for (idx, pair) in snapshots.windows(2).enumerate() {
        annotations = annotate_transition(&pair[0], &pair[1], &annotations, &(idx + 1).to_string());
    }
    for (ann, line) in annotations.iter().zip(snapshots.last().unwrap()) {
        println!("{ann}\t{line}");
    }
    println!();
    for (idx, patch) in touching.iter().enumerate() {
        println!("{}\t{}", idx + 1, state.display_patch(patch));
    }
    Ok(())
}

fn annotate_transition(
    old: &[String],
    new: &[String],
    old_annotations: &[String],
    annotation: &str,
) -> Vec<String> {
    let n = old.len();
    let m = new.len();
    let mut dp = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if old[i] == new[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old[i] == new[j] {
            out.push(old_annotations.get(i).cloned().unwrap_or_default());
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            i += 1;
        } else {
            out.push(annotation.to_string());
            j += 1;
        }
    }
    while j < m {
        out.push(annotation.to_string());
        j += 1;
    }
    out
}

fn read_lines_lossy(path: &Path) -> Result<Vec<String>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    Ok(fs::read_to_string(path)?
        .lines()
        .map(ToOwned::to_owned)
        .collect())
}

pub(crate) fn cmd_grep(args: Vec<OsString>) -> Result<()> {
    let state = State::discover(false)?;
    let args = if args.first().is_some_and(|arg| arg == "--") {
        args.into_iter().skip(1).collect()
    } else {
        args
    };
    let mut cmd = Command::new("grep");
    cmd.current_dir(&state.root).arg("-R");
    if !grep_suppresses_filename(&args) {
        cmd.arg("-H");
    }
    if let Some(name) = state.pc_dir.file_name() {
        cmd.arg(format!("--exclude-dir={}", name.to_string_lossy()));
    }
    if let Some(name) = state.patches_dir.file_name() {
        cmd.arg(format!("--exclude-dir={}", name.to_string_lossy()));
    }
    cmd.arg("--exclude=*.orig");
    let status = cmd.args(args).status()?;
    if status.success() {
        Ok(())
    } else {
        std::process::exit(status.code().unwrap_or(1));
    }
}

fn grep_suppresses_filename(args: &[OsString]) -> bool {
    for arg in args {
        if arg == "--" {
            return false;
        }
        if arg == "-h" || arg == "--no-filename" {
            return true;
        }
    }
    false
}
