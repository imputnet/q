use crate::commands::*;
use crate::config::*;
use crate::diff::*;
use crate::expected::*;
use crate::patchfile::strip_arg;
use crate::state::State;
use anyhow::{Context, Result, bail};
use lexopt::prelude::*;
use std::collections::HashSet;
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub(crate) fn cmd_diff(args: Vec<OsString>) -> Result<()> {
    let state = State::discover(false)?;
    let mut opts = DiffOpts::default();
    let mut strip = None;
    let mut patch = None;
    let mut combine = None;
    let mut snapshot = false;
    let mut files = Vec::new();
    let mut parser = lexopt::Parser::from_args(args);
    while let Some(arg) = parser.next()? {
        match arg {
            Short('P') => patch = Some(parser.value()?.string()?),
            Short('p') => strip = Some(parser.value()?.string()?),
            Short('R') => opts.reverse = true,
            Short('z') => opts.relative = true,
            Long("combine") => combine = Some(parser.value()?.string()?),
            Long("snapshot") => snapshot = true,
            Long("diff") => opts.diff_utility = Some(parser.value()?.string()?),
            Value(file) => files.push(state.rel_from_cwd(Path::new(&file))),
            other => {
                let option = option_name(&other);
                opts.parse_option(&option, &mut parser, "diff")?;
            }
        }
    }
    if snapshot {
        return emit_snapshot_diff(&state);
    }
    let patch = applied_patch(&state, patch.as_deref())?;
    opts.strip = strip_for(&state, &patch, strip, "diff")?;
    let tracked = state.tracked_files(&patch)?;
    let newer_files = files_modified_by_newer_patches(&state, &patch, &tracked)?;
    if let Some(combine) = combine {
        let range = applied_range(&state, &state.applied()?, &combine, &patch)?;
        emit_combined_diff(&state, &range, &files, &opts, &newer_files)?;
    } else {
        if opts.relative {
            opts.exclude = newer_files.clone();
        } else {
            opts.use_expected_for = newer_files.clone();
        }
        emit_diff(&state, &patch, &tracked, &files, &opts, io::stdout())?;
    }
    if !newer_files.is_empty() {
        eprintln!(
            "Warning: more recent patches modify files in patch {}",
            state.display_patch(&patch)
        );
    }
    Ok(())
}

fn emit_combined_diff(
    state: &State,
    patches: &[String],
    selected: &[PathBuf],
    opts: &DiffOpts,
    use_expected_for: &[PathBuf],
) -> Result<()> {
    let Some(first) = patches.first() else {
        return Ok(());
    };
    let Some(last) = patches.last() else {
        return Ok(());
    };
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    for patch in patches {
        for file in state.tracked_files(patch)? {
            if seen.insert(file.clone()) {
                files.push(file);
            }
        }
    }
    if !selected.is_empty() {
        let selected: HashSet<_> = selected.iter().cloned().collect();
        files.retain(|file| selected.contains(file));
    }
    let colors = opts.color.then(DiffColors::from_env);
    let mut stdout = io::stdout();
    for file in files {
        let old = state.backup_dir(first).join(&file);
        let new = if use_expected_for.iter().any(|candidate| candidate == &file) {
            expected_file(state, last, &file)
        } else {
            state.root.join(&file)
        };
        let old_empty = old
            .metadata()
            .map(|metadata| metadata.len() == 0)
            .unwrap_or(false);
        let new_missing = !new.exists();
        let old_arg = if old.exists() {
            old.as_path()
        } else {
            Path::new("/dev/null")
        };
        let new_arg = if new_missing {
            Path::new("/dev/null")
        } else {
            new.as_path()
        };
        let mut cmd = diff_command();
        cmd.args(env_args("QUILT_DIFF_OPTS")?);
        if let Some(lines) = &opts.context_lines {
            cmd.arg(if opts.context {
                format!("-C{lines}")
            } else {
                format!("-U{lines}")
            });
        } else {
            cmd.arg(if opts.context { "-c" } else { "-u" });
        }
        cmd.arg("--label")
            .arg(combined_diff_label(
                &file,
                opts.strip,
                true,
                !old.exists() || old_empty,
            ))
            .arg("--label")
            .arg(combined_diff_label(&file, opts.strip, false, new_missing));
        cmd.arg(old_arg).arg(new_arg);
        let output = cmd.output().context("failed to run diff")?;
        if output.status.code() == Some(0) {
            continue;
        }
        if output.status.code() != Some(1) {
            bail!("Diff failed on file '{}', aborting", file.display());
        }
        if !opts.no_index {
            write_diff_line(
                &mut stdout,
                &format!(
                    "Index: {}\n",
                    combined_index_label(state, &file, opts.strip)
                ),
                colors.as_ref(),
            )?;
            write_diff_line(
                &mut stdout,
                "===================================================================\n",
                colors.as_ref(),
            )?;
        }
        write_diff_bytes(&mut stdout, &output.stdout, colors.as_ref())?;
    }
    Ok(())
}

fn combined_index_label(state: &State, file: &Path, strip: Strip) -> String {
    if strip == Strip::Ab {
        format!("b/{}", file.display())
    } else {
        format!(
            "{}/{}",
            state.root.file_name().unwrap_or_default().to_string_lossy(),
            file.display()
        )
    }
}

fn combined_diff_label(file: &Path, strip: Strip, old: bool, missing: bool) -> String {
    if missing {
        return "/dev/null".to_string();
    }
    match strip {
        Strip::Zero if old => format!("{}.orig", file.display()),
        Strip::Ab if old => format!("a/{}", file.display()),
        Strip::Ab => format!("b/{}", file.display()),
        Strip::Zero | Strip::One => file.display().to_string(),
    }
}

fn emit_snapshot_diff(state: &State) -> Result<()> {
    let snap = snapshot_dir(state);
    if !snap.is_dir() {
        bail!("No snapshot to diff against");
    }

    let mut files = HashSet::new();
    for entry in WalkDir::new(&snap).into_iter().filter_map(Result::ok) {
        if entry.file_type().is_file() {
            files.insert(entry.path().strip_prefix(&snap)?.to_path_buf());
        }
    }
    for patch in state.applied()? {
        for file in state.tracked_files(&patch)? {
            files.insert(file);
        }
    }

    let mut files: Vec<_> = files.into_iter().collect();
    files.sort();
    for file in files {
        let old = snap.join(&file);
        let new = state.root.join(&file);
        let old_arg = if old.exists() {
            old.as_path()
        } else {
            Path::new("/dev/null")
        };
        let new_arg = if new.exists() {
            new.as_path()
        } else {
            Path::new("/dev/null")
        };
        let mut cmd = diff_command();
        cmd.args(env_args("QUILT_DIFF_OPTS")?);
        let output = cmd
            .arg("-u")
            .arg("--label")
            .arg(format!(
                "{}.orig/{}",
                state.root.file_name().unwrap_or_default().to_string_lossy(),
                file.display()
            ))
            .arg("--label")
            .arg(format!(
                "{}/{}",
                state.root.file_name().unwrap_or_default().to_string_lossy(),
                file.display()
            ))
            .arg(old_arg)
            .arg(new_arg)
            .output()?;
        if output.status.code() == Some(0) {
            continue;
        }
        if output.status.code() != Some(1) {
            bail!("Diff failed on file '{}'", file.display());
        }
        println!(
            "Index: {}/{}",
            state.root.file_name().unwrap_or_default().to_string_lossy(),
            file.display()
        );
        println!("===================================================================");
        io::stdout().write_all(&output.stdout)?;
    }
    Ok(())
}

/// How to name files when diffing `patch`: `explicit` if given, else the
/// strip level of its series entry. `command` names the command in errors.
pub(super) fn strip_for(
    state: &State,
    patch: &str,
    explicit: Option<String>,
    command: &str,
) -> Result<Strip> {
    let entries = state.entries()?;
    let from_series = entries
        .iter()
        .find(|entry| entry.name == patch)
        .and_then(strip_arg);
    Strip::parse(explicit.as_deref().or(from_series).unwrap_or("1"), command)
}
