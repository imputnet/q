use super::refresh::refresh_patch_for_files;
use crate::apply::*;
use crate::cli::{ExitStatus, usage};
use crate::color::color_arg;
use crate::commands::*;
use crate::config::*;
use crate::expected::*;
use crate::fsutil::*;
use crate::patchfile::*;
use crate::state::{PatchEntry, State};
use anyhow::{Context, Result, anyhow, bail};
use lexopt::prelude::*;
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use tempfile::{NamedTempFile, tempdir};

pub(crate) fn cmd_new(args: Vec<OsString>) -> Result<()> {
    let mut parser = lexopt::Parser::from_args(args);
    let mut strip = None::<String>;
    let mut patch = None::<String>;
    while let Some(arg) = parser.next()? {
        match arg {
            Short('p') => strip = Some(parser.value()?.string()?),
            Value(v) => {
                let s = v.to_string_lossy();
                patch = Some(s.strip_prefix("patches/").unwrap_or(&s).to_string());
            }
            _ => bail!("{}", usage("new")),
        }
    }
    let patch = patch.with_context(|| usage("new"))?;
    let state = State::discover(false)?;
    if state.entries()?.iter().any(|e| e.name == patch) {
        bail!("Patch {} exists already", state.display_patch(&patch));
    }
    state.create_db()?;
    fs::create_dir_all(state.backup_dir(&patch))?;
    let args = match strip.as_deref() {
        None | Some("1") => Vec::new(),
        Some("0") => vec!["-p0".to_string()],
        Some(n) => bail!("Cannot create patches with -p{n}, please specify -p0 or -p1 instead"),
    };
    let mut applied = state.applied()?;
    let before = state.patch_after(applied.last().map(String::as_str))?;
    state.insert_in_series(
        &PatchEntry {
            name: patch.clone(),
            args,
        },
        before.as_deref(),
    )?;
    applied.push(patch.clone());
    state.write_applied(&applied)?;
    println!("Patch {} is now on top", state.display_patch(&patch));
    Ok(())
}

pub(crate) fn cmd_push(args: Vec<OsString>) -> Result<()> {
    let mut all = false;
    let mut force = false;
    let mut quiet = false;
    let mut refresh = false;
    let mut leave_rejects = false;
    let mut color = false;
    let mut target = None::<String>;
    let mut patch_args = Vec::<String>::new();
    let mut parser = lexopt::Parser::from_args(args);
    while let Some(arg) = parser.next()? {
        match arg {
            Short('a') => all = true,
            Short('f') => force = true,
            Short('q') => quiet = true,
            Short('v') => {}
            Short('m') | Long("merge") => {
                let style = parser.optional_value().map(|v| v.string()).transpose()?;
                match style.as_deref() {
                    None | Some("" | "merge") => patch_args.push("--merge".to_string()),
                    Some("diff3") => patch_args.push("--merge=diff3".to_string()),
                    Some(_) => bail!("{}", usage("push")),
                }
            }
            Long("fuzz") => patch_args.push(format!("--fuzz={}", parser.value()?.string()?)),
            Long("leave-rejects") => leave_rejects = true,
            Long("refresh") => refresh = true,
            Long("color") => color = color_arg(parser.optional_value(), "push")?,
            Value(value) => target = Some(value.string()?),
            _ => bail!("{}", usage("push")),
        }
    }
    let state = State::discover(false)?;
    if force && refresh {
        bail!("Options -f and --refresh are mutually exclusive");
    }
    state.require_series()?;
    state.create_db()?;
    let series = state.entries()?;
    let mut applied = state.applied()?;
    let mut applied_set: HashSet<String> = applied.iter().cloned().collect();
    let mut idx = applied.len();
    let requested_target = target.clone();
    let end = if all {
        series.len()
    } else if let Some(t) = target {
        if t.chars().all(|c| c.is_ascii_digit()) {
            (idx + t.parse::<usize>()?).min(series.len())
        } else {
            let target_patch = state.find_patch(&t)?;
            if applied_set.contains(&target_patch) {
                bail!(
                    "Patch {} is currently applied",
                    state.display_patch(&target_patch)
                );
            }
            series
                .iter()
                .position(|e| e.name == target_patch)
                .map(|i| i + 1)
                .context("Patch not in series")?
        }
    } else {
        (idx + 1).min(series.len())
    };
    if all {
        let mut seen = HashSet::new();
        for entry in &series[idx..end] {
            if !seen.insert(entry.name.clone()) || applied_set.contains(&entry.name) {
                println!(
                    "Patch {} is already applied; check your series file",
                    state.display_patch(&entry.name)
                );
                return Ok(());
            }
        }
    }
    let mut changed = false;
    let mut show_now = true;
    let mut forced_dirty = false;
    let colors = color.then(PushColors::from_env);
    if let Some(top) = applied.last()
        && refresh_marker(&state, top).exists()
    {
        println!(
            "The topmost patch {} needs to be refreshed first.",
            state.display_patch(top)
        );
        return Err(anyhow!(""));
    }
    while idx < end {
        let entry = &series[idx];
        if applied_set.contains(&entry.name) {
            println!(
                "Patch {} is already applied; check your series file",
                state.display_patch(&entry.name)
            );
            return Ok(());
        }
        println!("Applying patch {}", state.display_patch(&entry.name));
        if !state.patch_path(&entry.name).exists() {
            fs::create_dir_all(state.backup_dir(&entry.name))?;
            applied.push(entry.name.clone());
            applied_set.insert(entry.name.clone());
            state.write_applied(&applied)?;
            println!(
                "Patch {} does not exist; applied empty patch",
                state.display_patch(&entry.name)
            );
            let _ = fs::remove_file(refresh_marker(&state, &entry.name));
            changed = true;
            idx += 1;
            continue;
        }
        if patch_body(&state.patch_path(&entry.name))?.is_empty() {
            fs::create_dir_all(state.backup_dir(&entry.name))?;
            applied.push(entry.name.clone());
            applied_set.insert(entry.name.clone());
            state.write_applied(&applied)?;
            record_expected_state(&state, &entry.name, &state.tracked_files(&entry.name)?)?;
            println!(
                "Patch {} appears to be empty; applied",
                state.display_patch(&entry.name)
            );
            let _ = fs::remove_file(refresh_marker(&state, &entry.name));
            changed = true;
            idx += 1;
            continue;
        }
        let result = apply_patch(
            &state,
            entry,
            &PushOptions {
                force,
                quiet,
                leave_rejects,
                patch_args: &patch_args,
                colors: colors.as_ref(),
            },
        )?;
        applied.push(entry.name.clone());
        applied_set.insert(entry.name.clone());
        state.write_applied(&applied)?;
        record_expected_state(&state, &entry.name, &result.files)?;
        if refresh {
            refresh_patch_for_files(&state, &entry.name, &result.files, None)?;
        }
        if force && !result.clean {
            println!(
                "Applied patch {} (forced; needs refresh)",
                state.display_patch(&entry.name)
            );
            File::create(refresh_marker(&state, &entry.name))?;
            show_now = false;
            forced_dirty = true;
        } else {
            let _ = fs::remove_file(refresh_marker(&state, &entry.name));
            if !quiet {
                println!();
            }
        }
        changed = true;
        idx += 1;
    }
    if forced_dirty {
        return Err(anyhow!(""));
    }
    if changed {
        if show_now && let Some(top) = applied.last() {
            println!("Now at patch {}", state.display_patch(top));
        }
        return Ok(());
    }
    let message = match applied.last() {
        Some(top) if requested_target.is_none() => format!(
            "File series fully applied, ends at patch {}",
            state.display_patch(top)
        ),
        _ => "No patch applied".to_string(),
    };
    Err(ExitStatus { code: 2, message }.into())
}

pub(crate) fn cmd_pop(args: Vec<OsString>) -> Result<()> {
    let mut all = false;
    let mut force = false;
    let mut quiet = false;
    let mut refresh = false;
    let mut target = None::<String>;
    let mut parser = lexopt::Parser::from_args(args);
    while let Some(arg) = parser.next()? {
        match arg {
            Short('a') => all = true,
            Short('f') => force = true,
            Short('q') => quiet = true,
            Short('R' | 'v') => {}
            Long("refresh") => refresh = true,
            Value(value) => target = Some(value.string()?),
            _ => bail!("{}", usage("pop")),
        }
    }
    let state = State::discover(false)?;
    if force && refresh {
        bail!("Options -f and --refresh are mutually exclusive");
    }
    state.require_series()?;
    let mut applied = state.applied()?;
    if applied.is_empty() {
        if let Some(t) = target
            && !t.chars().all(|c| c.is_ascii_digit())
        {
            let _ = state.find_patch(&t)?;
        }
        return Err(ExitStatus {
            code: 2,
            message: "No patch removed".to_string(),
        }
        .into());
    }
    let keep = if all {
        0
    } else if let Some(t) = target {
        if t.chars().all(|c| c.is_ascii_digit()) {
            applied.len().saturating_sub(t.parse::<usize>()?)
        } else {
            let p = state.find_patch(&t)?;
            applied
                .iter()
                .position(|x| x == &p)
                .context("Patch is not applied")?
                + 1
        }
    } else {
        applied.len() - 1
    };
    let entries = state.entries()?;
    let mut removed_any = false;
    while applied.len() > keep {
        let patch = applied.pop().unwrap();
        let entry = entries
            .iter()
            .find(|entry| entry.name == patch)
            .with_context(|| format!("Patch {} is not in series", state.display_patch(&patch)))?;
        let tracked = state.tracked_files(&patch)?;
        if refresh {
            refresh_patch_for_files(&state, &patch, &tracked, None)?;
        }
        if !force && !refresh && !tracked.is_empty() && !state.patch_path(&patch).exists() {
            bail!(
                "Patch {} does not remove cleanly (refresh it or enforce with -f)\nHint: `quilt diff -z' will show the pending changes.",
                state.display_patch(&patch)
            );
        }
        if !force
            && !refresh
            && !patch_removes_cleanly(&state, entry, !removed_any, &tracked).unwrap_or(false)
        {
            bail!(
                "Patch {} does not remove cleanly (refresh it or enforce with -f)\nHint: `quilt diff -z' will show the pending changes.",
                state.display_patch(&patch)
            );
        }
        let empty_patch = tracked.is_empty()
            && (!state.patch_path(&patch).exists()
                || patch_body(&state.patch_path(&patch))?.is_empty());
        if empty_patch {
            println!(
                "Patch {} appears to be empty, removing",
                state.display_patch(&patch)
            );
        } else {
            println!("Removing patch {}", state.display_patch(&patch));
            restore_patch_files(&state, &patch, &tracked, !quiet)?;
        }
        if !quiet {
            println!();
        }
        state.write_applied(&applied)?;
        removed_any = true;
    }
    if applied.is_empty() {
        println!("No patches applied");
    } else if let Some(top) = applied.last() {
        println!("Now at patch {}", state.display_patch(top));
    }
    Ok(())
}

pub(crate) fn cmd_delete(args: Vec<OsString>) -> Result<()> {
    let mut remove_file = false;
    let mut backup = env_flag("QUILT_BACKUP");
    let mut patch = None::<String>;
    let mut next = false;
    let mut parser = lexopt::Parser::from_args(args);
    while let Some(arg) = parser.next()? {
        match arg {
            Short('r') => remove_file = true,
            Short('n') => next = true,
            Long("backup") => backup = true,
            Value(value) => patch = Some(value.string()?),
            _ => bail!("{}", usage("delete")),
        }
    }
    let state = State::discover(false)?;
    let entries = state.entries()?;
    let applied = state.applied()?;
    let patch = if next {
        let start = applied.last();
        let pos = start
            .and_then(|top| entries.iter().position(|e| &e.name == top).map(|i| i + 1))
            .unwrap_or(0);
        entries
            .get(pos)
            .map(|e| e.name.clone())
            .context("No next patch")?
    } else if let Some(p) = patch {
        state.find_patch(&p)?
    } else {
        state.top()?
    };
    let top = applied.last();
    if applied.contains(&patch) && top != Some(&patch) {
        bail!("Patch {} is currently applied", state.display_patch(&patch));
    }
    if top == Some(&patch) {
        println!("Removing patch {}", state.display_patch(&patch));
        let mut new_applied = applied;
        new_applied.retain(|p| p != &patch);
        let verbose_restore = !patch_body(&state.patch_path(&patch))?.is_empty();
        if let Err(err) = restore_patch(&state, &patch, verbose_restore) {
            state.write_applied(&new_applied)?;
            return Err(err);
        }
        if verbose_restore {
            println!();
        }
        state.write_applied(&new_applied)?;
        if new_applied.is_empty() {
            println!("No patches applied");
        } else if let Some(top) = new_applied.last() {
            println!("Now at patch {}", state.display_patch(top));
        }
    }
    state.remove_from_series(&patch)?;
    if remove_file {
        let path = state.patch_path(&patch);
        if backup {
            fs::rename(&path, tilde_backup(&path))?;
        } else {
            let _ = fs::remove_file(path);
        }
    }
    println!("Removed patch {}", state.display_patch(&patch));
    Ok(())
}

pub(crate) fn cmd_rename(args: Vec<OsString>) -> Result<()> {
    let mut old = None::<String>;
    let mut new = None::<String>;
    let mut parser = lexopt::Parser::from_args(args);
    while let Some(arg) = parser.next()? {
        match arg {
            Short('P') => old = Some(parser.value()?.string()?),
            Value(v) => new = Some(v.to_string_lossy().into_owned()),
            _ => bail!("{}", usage("rename")),
        }
    }
    let state = State::discover(false)?;
    let old = if let Some(p) = old {
        state.find_patch(&p)?
    } else {
        state.top()?
    };
    let new = new.with_context(|| usage("rename"))?;
    if state.entries()?.iter().any(|e| e.name == new) {
        bail!(
            "Patch {} exists already, please choose a different name",
            state.display_patch(&new)
        );
    }
    state.rename_patch(&old, &new)?;
    move_if_exists(&state.patch_path(&old), &state.patch_path(&new))?;
    move_if_exists(&state.backup_dir(&old), &state.backup_dir(&new))?;
    println!(
        "Patch {} renamed to {}",
        state.display_patch(&old),
        state.display_patch(&new)
    );
    Ok(())
}

pub(crate) fn cmd_fork(args: Vec<OsString>) -> Result<()> {
    let state = State::discover(false)?;
    state.require_series()?;
    let old = state.top()?;
    let new = args
        .first()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| next_filename(&old));
    if state.patch_path(&new).exists() || state.entries()?.iter().any(|e| e.name == new) {
        bail!(
            "Patch {} exists already, please choose a new name",
            state.display_patch(&new)
        );
    }
    if state.patch_path(&old).exists() {
        copy_creating_dirs(&state.patch_path(&old), &state.patch_path(&new))?;
    }
    state.rename_patch(&old, &new)?;
    move_if_exists(&state.backup_dir(&old), &state.backup_dir(&new))?;
    println!(
        "Fork of patch {} created as {}",
        state.display_patch(&old),
        state.display_patch(&new)
    );
    Ok(())
}

pub(crate) fn cmd_import(args: Vec<OsString>) -> Result<()> {
    let mut parser = lexopt::Parser::from_args(args);
    let mut patch_name = None::<String>;
    let mut strip = None::<usize>;
    let mut reverse = false;
    let mut header_policy = None::<String>;
    let mut force = false;
    let mut files = Vec::new();
    while let Some(arg) = parser.next()? {
        match arg {
            Short('P') => patch_name = Some(parser.value()?.string()?),
            Short('p') => strip = Some(parser.value()?.parse::<usize>()?),
            Short('R') => reverse = true,
            Short('d') => header_policy = Some(parser.value()?.string()?),
            Short('f') => force = true,
            Value(v) => files.push(PathBuf::from(v)),
            _ => {}
        }
    }
    if files.is_empty() {
        bail!("{}", usage("import"));
    }
    let state = State::discover(false)?;
    state.create_db()?;
    let applied = state.applied()?;
    let before = state.patch_after(applied.last().map(String::as_str))?;
    for src in files {
        if !src.exists() {
            bail!("Patch {} does not exist", src.display());
        }
        let name = match (&patch_name, src.file_name()) {
            (Some(name), _) => name.clone(),
            (None, Some(name)) => name.to_string_lossy().into_owned(),
            (None, None) => bail!("Patch {} does not exist", src.display()),
        };
        let dest = state.patch_path(&name);
        if applied.contains(&name) {
            bail!("Patch {} is applied", state.display_patch(&name));
        }
        let in_series = state.entries()?.iter().any(|e| e.name == name);
        let same_file =
            dest.exists() && fs::canonicalize(&src).ok() == fs::canonicalize(&dest).ok();
        if in_series && !force && same_file {
            bail!(
                "Patch {} already exists in series.",
                state.display_patch(&name)
            );
        }
        if in_series && !force {
            bail!(
                "Patch {} exists. Replace with -f.",
                state.display_patch(&name)
            );
        }
        if dest.exists() && !force && !same_file {
            bail!(
                "Patch {} exists. Replace with -f.",
                state.display_patch(&name)
            );
        }
        let import_headers = if force && dest.exists() && !same_file {
            Some((patch_header(&dest)?, patch_header(&src)?))
        } else {
            None
        };
        if let Some((old_header, new_header)) = &import_headers
            && !old_header.is_empty()
            && !new_header.is_empty()
            && old_header != new_header
            && header_policy.is_none()
        {
            bail!(
                "Patch headers differ:\n@@ -1 +1 @@\n-{}\n+{}\nPlease use -d {{o|a|n}} to specify which patch header(s) to keep.",
                old_header.lines().next().unwrap_or(""),
                new_header.lines().next().unwrap_or("")
            );
        }
        create_parent_dir(&dest)?;
        if !same_file {
            fs::copy(&src, &dest)?;
        }
        if let Some((old_header, new_header)) = &import_headers
            && force
            && header_policy.is_none()
            && !old_header.is_empty()
            && new_header.is_empty()
        {
            let combined = format!("{old_header}{}", patch_body(&src)?);
            write_patch_text(&dest, &combined)?;
        }
        if let (Some(policy), Some((old_header, new_header))) = (&header_policy, &import_headers) {
            if policy == "a" && !old_header.is_empty() && !new_header.is_empty() {
                let combined = format!("{old_header}---\n{new_header}{}", patch_body(&src)?);
                write_patch_text(&dest, &combined)?;
            } else if policy == "o" && !old_header.is_empty() {
                let combined = format!("{old_header}{}", patch_body(&src)?);
                write_patch_text(&dest, &combined)?;
            }
        }
        if !in_series {
            let mut entry_args = Vec::new();
            if let Some(strip) = &strip {
                entry_args.push(format!("-p{strip}"));
            }
            if reverse {
                entry_args.push("-R".to_string());
            }
            state.insert_in_series(
                &PatchEntry {
                    name: name.clone(),
                    args: entry_args,
                },
                before.as_deref(),
            )?;
        }
        if force && dest.exists() && !same_file {
            println!(
                "Replacing patch {} with new version",
                state.display_patch(&name)
            );
        } else if src.parent() == Some(Path::new("patches")) || same_file {
            println!("Importing patch {}", state.display_patch(&name));
        } else {
            println!(
                "Importing patch {} (stored as {})",
                src.display(),
                state.display_patch(&name)
            );
        }
    }
    Ok(())
}

pub(crate) fn cmd_fold(args: Vec<OsString>) -> Result<()> {
    let mut reverse = false;
    let mut strip = 1;
    let mut force = false;
    let mut quiet = false;
    let mut parser = lexopt::Parser::from_args(args);
    while let Some(arg) = parser.next()? {
        match arg {
            Short('R') => reverse = true,
            Short('p') => strip = parser.value()?.parse()?,
            Short('f') => force = true,
            Short('q') => quiet = true,
            _ => {}
        }
    }
    let state = State::discover(false)?;
    let patch = state.top()?;
    let mut input = Vec::new();
    io::stdin().read_to_end(&mut input)?;
    let files: Vec<PathBuf> = filenames_in_text_order(&String::from_utf8_lossy(&input), strip)
        .iter()
        .map(|file| state.rel_from_cwd(file))
        .collect();

    // Like quilt: keep the files as they are before folding, roll back to
    // that on failure, and on success use it as the backup of files new to
    // the top patch.
    let before = tempdir()?;
    for rel in &files {
        let src = state.root.join(rel);
        if src.exists() {
            copy_creating_dirs(&src, &before.path().join(rel))
                .with_context(|| format!("Failed to back up file {}", rel.display()))?;
        }
    }
    let restore = || -> Result<()> {
        for rel in &files {
            let saved = before.path().join(rel);
            let live = state.root.join(rel);
            if saved.exists() {
                copy_to_new_inode(&saved, &live)?;
            } else {
                let _ = fs::remove_file(&live);
            }
        }
        Ok(())
    };

    let workdir = state.root.join(&state.subdir);
    let args = PatchFlags {
        strip,
        reverse,
        force,
        quiet,
        extra: &[],
    }
    .args()?;
    let outcome = apply_in_process(&workdir, &input, args, None);
    if outcome.touched_files() {
        restore()?;
    }
    if !matches!(outcome, InProcess::Applied) {
        let mut tmp = NamedTempFile::new()?;
        tmp.write_all(&input)?;
        let mut cmd = patch_command();
        cmd.current_dir(&workdir)
            .arg(format!("-p{strip}"))
            .arg("-i")
            .arg(tmp.path())
            .args(env_args("QUILT_PATCH_OPTS")?);
        if reverse {
            cmd.arg("-R");
        }
        if force {
            cmd.arg("-f");
        }
        if quiet {
            cmd.arg("-s");
        }
        if !cmd.status()?.success() && !force {
            restore()?;
            bail!("Fold failed");
        }
    }

    for rel in &files {
        let backup = state.backup_dir(&patch).join(rel);
        if backup.exists() {
            continue;
        }
        let saved = before.path().join(rel);
        if saved.exists() {
            copy_creating_dirs(&saved, &backup)?;
        } else {
            create_parent_dir(&backup)?;
            File::create(&backup)?;
        }
    }
    Ok(())
}
