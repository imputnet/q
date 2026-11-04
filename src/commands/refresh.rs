use super::diff::strip_for;
use crate::cli::usage;
use crate::commands::*;
use crate::config::*;
use crate::diff::*;
use crate::expected::*;
use crate::fsutil::*;
use crate::patchfile::*;
use crate::state::{PatchEntry, State};
use anyhow::{Context, Result, bail};
use lexopt::prelude::*;
use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::process::Stdio;
use tempfile::NamedTempFile;

pub(crate) fn cmd_refresh(args: Vec<OsString>) -> Result<()> {
    let state = State::discover(false)?;
    let mut opts = DiffOpts::default();
    let mut force = false;
    let mut strip = None::<String>;
    let mut strip_ws = false;
    let mut backup = env_flag("QUILT_BACKUP");
    let mut diffstat = false;
    // `-z` forks the refreshed changes into a new patch, optionally named.
    let mut fork_name = None::<Option<String>>;
    let mut patch = None::<String>;
    let mut parser = lexopt::Parser::from_args(args);
    while let Some(arg) = parser.next()? {
        match arg {
            Short('f') => force = true,
            Short('p') => strip = Some(parser.value()?.string()?),
            Short('z') => {
                fork_name = Some(parser.optional_value().map(|v| v.string()).transpose()?);
            }
            Short('P') => patch = Some(parser.value()?.string()?),
            Long("strip-trailing-whitespace") => strip_ws = true,
            Long("backup") => backup = true,
            Long("diffstat") => diffstat = true,
            Value(value) if patch.is_none() => patch = Some(value.string()?),
            other => {
                let option = option_name(&other);
                if !opts.parse_option(&option, &mut parser, "refresh")? {
                    bail!("{}", usage("refresh"));
                }
            }
        }
    }
    let patch = applied_patch(&state, patch.as_deref())?;
    opts.strip = strip_for(&state, &patch, strip, "refresh")?;
    opts.relative = fork_name.is_some();
    let tracked = state.tracked_files(&patch)?;
    let preserved_files = files_modified_by_newer_patches(&state, &patch, &tracked)?;
    if !force && !preserved_files.is_empty() {
        bail!(
            "More recent patches modify files in patch {}. Enforce refresh with -f.",
            state.display_patch(&patch)
        );
    }
    report_or_strip_trailing_ws_for_files(&state, &tracked, strip_ws)?;
    let dest = state.patch_path(&patch);
    let new_text = refresh_patch_text(&state, &patch, &opts, &tracked, &preserved_files)?;
    if let Some(name) = fork_name {
        let new_patch = name.unwrap_or_else(|| next_filename(&patch));
        let dest = state.patch_path(&new_patch);
        if dest.exists() || state.entries()?.iter().any(|e| e.name == new_patch) {
            bail!("Patch {} exists already", state.display_patch(&new_patch));
        }
        create_parent_dir(&dest)?;
        write_patch_text(&dest, &new_text)?;
        seed_fork_backup(&state, &patch, &new_patch, &new_text, opts.strip.level())?;
        let before = state.patch_after(state.applied()?.last().map(String::as_str))?;
        state.insert_in_series(
            &PatchEntry {
                name: new_patch.clone(),
                args: Vec::new(),
            },
            before.as_deref(),
        )?;
        let mut applied = state.applied()?;
        applied.push(new_patch.clone());
        state.write_applied(&applied)?;
        record_expected_state(&state, &new_patch, &state.tracked_files(&new_patch)?)?;
        println!(
            "Fork of patch {} created as {}",
            state.display_patch(&patch),
            state.display_patch(&new_patch)
        );
        return Ok(());
    }
    create_parent_dir(&dest)?;
    let mut new_text = new_text;
    if diffstat {
        new_text = add_diffstat_to_patch_text(&state, &new_text)?;
    }
    if dest.exists() && patch_text(&dest)? == new_text && !force {
        println!("Patch {} is unchanged", state.display_patch(&patch));
        return Ok(());
    }
    if backup && dest.exists() {
        let backup_path = tilde_backup(&dest);
        if is_symlink(&dest) {
            copy_preserving_times(&dest, &backup_path)?;
        } else if let Err(err) = fs::rename(&dest, &backup_path) {
            let rel = dest.strip_prefix(&state.root).unwrap_or(&dest);
            let backup_rel = backup_path
                .strip_prefix(&state.root)
                .unwrap_or(&backup_path);
            bail!(
                "mv: cannot move '{}' to '{}': {}",
                rel.display(),
                backup_rel.display(),
                if err.kind() == std::io::ErrorKind::PermissionDenied {
                    "Permission denied".to_string()
                } else {
                    err.to_string()
                }
            );
        }
    }
    store_refreshed_patch(&state, &patch, &tracked, &new_text, opts.strip.level())?;
    if !has_diff(&new_text) && !strip_ws {
        println!("Nothing in patch {}", state.display_patch(&patch));
        return Ok(());
    }
    println!("Refreshed patch {}", state.display_patch(&patch));
    Ok(())
}

/// The refreshed text of `patch`: its header plus a fresh diff of `tracked`.
/// Files in `preserved` (changed by later patches too, under `refresh -f`)
/// keep their old sections, in the order the old patch had them.
fn refresh_patch_text(
    state: &State,
    patch: &str,
    opts: &DiffOpts,
    tracked: &[PathBuf],
    preserved_files: &[PathBuf],
) -> Result<String> {
    let mut out = Vec::new();
    let dest = state.patch_path(patch);
    let header = patch_header(&dest)?;
    out.write_all(header.as_bytes())?;
    if !preserved_files.is_empty() {
        let preserved_set: HashSet<_> = preserved_files.iter().cloned().collect();
        let tracked: Vec<_> = tracked
            .iter()
            .filter(|file| !preserved_set.contains(*file))
            .cloned()
            .collect();
        let mut generated = Vec::new();
        emit_diff(state, patch, &tracked, &[], opts, &mut generated)?;
        let generated =
            String::from_utf8(generated).context("generated patch is not valid UTF-8")?;
        let preserved = body_for_files(
            &patch_body(&dest)?,
            opts.strip.level(),
            &preserved_files.iter().cloned().collect(),
        );
        let body = merge_refresh_bodies(&dest, opts.strip.level(), &generated, &preserved);
        out.write_all(body.as_bytes())?;
    } else {
        emit_diff(state, patch, tracked, &[], opts, &mut out)?;
    }
    String::from_utf8(out).context("generated patch is not valid UTF-8")
}

fn merge_refresh_bodies(dest: &Path, strip: usize, generated: &str, preserved: &str) -> String {
    let mut out = String::new();
    let mut emitted = HashSet::new();
    let generated_files: HashSet<_> = filenames_in_text_order(generated, strip)
        .into_iter()
        .collect();
    let preserved_files: HashSet<_> = filenames_in_text_order(preserved, strip)
        .into_iter()
        .collect();

    for file in filenames_in_patch_order(dest, strip) {
        if generated_files.contains(&file) {
            append_refresh_body_for_file(&mut out, generated, strip, &file);
            emitted.insert(file);
        } else if preserved_files.contains(&file) {
            append_refresh_body_for_file(&mut out, preserved, strip, &file);
            emitted.insert(file);
        }
    }
    for file in filenames_in_text_order(generated, strip) {
        if emitted.insert(file.clone()) {
            append_refresh_body_for_file(&mut out, generated, strip, &file);
        }
    }
    for file in filenames_in_text_order(preserved, strip) {
        if emitted.insert(file.clone()) {
            append_refresh_body_for_file(&mut out, preserved, strip, &file);
        }
    }
    out
}

fn append_refresh_body_for_file(out: &mut String, body: &str, strip: usize, file: &Path) {
    let wanted = HashSet::from([file.to_path_buf()]);
    out.push_str(&body_for_files(body, strip, &wanted));
}

fn add_diffstat_to_patch_text(state: &State, patch_text: &str) -> Result<String> {
    let header = remove_diffstat(&header_of(patch_text), false);
    let body = body_of(patch_text);
    let mut cmd = Command::new("diffstat");
    cmd.args(env_args("QUILT_DIFFSTAT_OPTS")?);
    let output = cmd
        .current_dir(&state.root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            if let Some(stdin) = child.stdin.as_mut() {
                stdin.write_all(body.as_bytes())?;
            }
            child.wait_with_output()
        })
        .context("failed to run diffstat")?;
    if !output.status.success() {
        bail!("diffstat failed");
    }
    let diffstat = String::from_utf8_lossy(&output.stdout);
    Ok(format!("{header}---\n{diffstat}\n{body}"))
}

/// Drop a diffstat (`---` through the next blank line) from a header. With
/// `keep_separator`, the `---` line and blank line themselves stay.
fn remove_diffstat(header: &str, keep_separator: bool) -> String {
    let mut out = String::new();
    let mut lines = header.lines();
    while let Some(line) = lines.next() {
        if line != "---" {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        if keep_separator {
            out.push_str("---\n");
        }
        if lines.any(|next| next.trim().is_empty()) && keep_separator {
            out.push('\n');
        }
    }
    out
}

/// Write a refreshed patch and update the metadata that depends on it.
fn store_refreshed_patch(
    state: &State,
    patch: &str,
    tracked: &[PathBuf],
    text: &str,
    strip: usize,
) -> Result<()> {
    write_patch_text(&state.patch_path(patch), text)?;
    state.update_series_args(patch, strip)?;
    record_expected_state_with_text(state, patch, tracked, text)?;
    let _ = fs::remove_file(refresh_marker(state, patch));
    Ok(())
}

/// Refresh `patch` with `QUILT_REFRESH_ARGS` settings, as `push --refresh`,
/// `pop --refresh` and `absorb` do. With `new_root`, the new side of each file
/// is read from there instead of the working tree.
pub(super) fn refresh_patch_for_files(
    state: &State,
    patch: &str,
    tracked: &[PathBuf],
    new_root: Option<&Path>,
) -> Result<()> {
    let mut opts = DiffOpts {
        new_root: new_root.map(Path::to_path_buf),
        ..DiffOpts::default()
    };
    opts.strip = strip_for(state, patch, None, "refresh")?;
    apply_refresh_env_args(&mut opts)?;
    let dest = state.patch_path(patch);
    let new_text = refresh_patch_text(state, patch, &opts, tracked, &[])?;
    if dest.exists() && patch_text(&dest)? == new_text {
        println!("Patch {} is unchanged", state.display_patch(patch));
        return Ok(());
    }
    store_refreshed_patch(state, patch, tracked, &new_text, opts.strip.level())?;
    if has_diff(&new_text) {
        println!("Refreshed patch {}", state.display_patch(patch));
    } else {
        println!("Nothing in patch {}", state.display_patch(patch));
    }
    Ok(())
}

fn apply_refresh_env_args(opts: &mut DiffOpts) -> Result<()> {
    let mut parser = lexopt::Parser::from_args(env_args("QUILT_REFRESH_ARGS")?);
    while let Some(arg) = parser.next()? {
        match arg {
            Short('p') => opts.strip = Strip::parse(&parser.value()?.string()?, "refresh")?,
            Long("diffstat" | "backup" | "strip-trailing-whitespace") => {}
            other => {
                let option = option_name(&other);
                opts.parse_option(&option, &mut parser, "refresh")?;
            }
        }
    }
    Ok(())
}

fn report_or_strip_trailing_ws_for_files(
    state: &State,
    files: &[PathBuf],
    strip: bool,
) -> Result<()> {
    for file in files {
        let path = state.root.join(file);
        if !path.is_file() {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let lines: Vec<_> = text
            .lines()
            .enumerate()
            .filter_map(|(idx, line)| {
                if line.ends_with(' ') || line.ends_with('\t') {
                    Some((idx + 1).to_string())
                } else {
                    None
                }
            })
            .collect();
        if lines.is_empty() {
            continue;
        }
        if strip {
            println!(
                "Removing trailing whitespace from lines {} of {}",
                lines.join(","),
                file.display()
            );
            fs::write(&path, strip_trailing_whitespace(&text))?;
        } else {
            println!(
                "Warning: trailing whitespace in lines {} of {}",
                lines.join(","),
                file.display()
            );
        }
    }
    Ok(())
}

/// Back up the files of a `refresh -z` fork: each file as the old patch left
/// it, so the fork holds only the changes made since.
fn seed_fork_backup(
    state: &State,
    old_patch: &str,
    new_patch: &str,
    fork_text: &str,
    strip: usize,
) -> Result<()> {
    for file in filenames_in_text_order(fork_text, strip) {
        let backup = state.backup_dir(new_patch).join(&file);
        let expected = expected_file(state, old_patch, &file);
        let old_backup = state.backup_dir(old_patch).join(&file);
        if expected.exists() {
            copy_creating_dirs(&expected, &backup)?;
        } else if old_backup.metadata().is_ok_and(|m| m.len() != 0) {
            copy_creating_dirs(&old_backup, &backup)?;
        } else {
            create_parent_dir(&backup)?;
            File::create(backup)?;
        }
    }
    Ok(())
}

pub(crate) fn cmd_header(args: Vec<OsString>) -> Result<()> {
    let mut replace = false;
    let mut append = false;
    let mut edit = false;
    let mut strip_diffstat = false;
    let mut strip_ws = false;
    let mut backup = env_flag("QUILT_BACKUP");
    let mut patch_arg = None::<String>;
    for arg in args {
        if arg == "-r" {
            replace = true;
        } else if arg == "-a" {
            append = true;
        } else if arg == "-e" {
            edit = true;
        } else if arg == "--strip-trailing-whitespace" {
            strip_ws = true;
        } else if arg == "--backup" {
            backup = true;
        } else if arg == "--strip-diffstat" {
            strip_diffstat = true;
        } else if !arg.to_string_lossy().starts_with('-') {
            patch_arg = Some(arg.to_string_lossy().into_owned());
        }
    }
    if [replace, append, edit]
        .into_iter()
        .filter(|enabled| *enabled)
        .count()
        > 1
    {
        bail!("{}", usage("header"));
    }
    let state = State::discover(false)?;
    state.require_series()?;
    let patch = if let Some(arg) = patch_arg {
        state.find_patch(&arg)?
    } else {
        state.top()?
    };
    let path = state.patch_path(&patch);
    if !replace && !append && !edit {
        let mut header = patch_header(&path)?;
        if strip_diffstat {
            header = remove_diffstat(&header, true);
        }
        if strip_ws {
            header = strip_trailing_whitespace(&header);
        }
        print!("{header}");
        return Ok(());
    }

    let mut input = String::new();
    if replace || append {
        io::stdin().read_to_string(&mut input)?;
    }
    let mut header = String::new();
    if !replace {
        header.push_str(&patch_header(&path)?);
    }
    if append || replace {
        header.push_str(&input);
    }
    if edit {
        let mut tmp = NamedTempFile::new()?;
        tmp.write_all(header.as_bytes())?;
        tmp.flush()?;
        let editor = env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
        let status = Command::new(editor).arg(tmp.path()).status()?;
        if !status.success() {
            bail!("editor failed");
        }
        header = fs::read_to_string(tmp.path())?;
    }
    if !header.is_empty() && !header.ends_with('\n') {
        header.push('\n');
    }
    if strip_diffstat {
        header = remove_diffstat(&header, true);
    }
    if strip_ws {
        header = strip_trailing_whitespace(&header);
    }
    let body = patch_body(&path)?;
    if backup && path.exists() {
        copy_preserving_times(&path, &tilde_backup(&path))?;
    }
    write_patch_text(&path, &format!("{header}{body}"))?;
    let verb = if append {
        "Appended text to"
    } else {
        "Replaced"
    };
    println!("{verb} header of patch {}", state.display_patch(&patch));
    Ok(())
}
