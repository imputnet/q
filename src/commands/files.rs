use crate::cli::usage;
use crate::commands::*;
use crate::expected::*;
use crate::fsutil::*;
use crate::state::State;
use anyhow::{Context, Result, bail};
use lexopt::prelude::*;
use std::collections::HashSet;
use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn cmd_add(args: Vec<OsString>) -> Result<()> {
    let mut parser = lexopt::Parser::from_args(args);
    let mut patch = None::<String>;
    let mut new = false;
    let mut files = Vec::new();
    while let Some(arg) = parser.next()? {
        match arg {
            Short('P') => patch = Some(parser.value()?.string()?),
            Long("new") => new = true,
            Value(v) => files.push(PathBuf::from(v)),
            _ => bail!("{}", usage("add")),
        }
    }
    if files.is_empty() {
        bail!("{}", usage("add"));
    }
    let state = State::discover(false)?;
    let patch = applied_patch(&state, patch.as_deref())?;
    for file in files {
        let rel = state.rel_from_cwd(&file);
        if let Some(later) = newer_patch_tracking_file(&state, &patch, &rel)? {
            bail!(
                "File {} modified by patch {}",
                rel.display(),
                state.display_patch(&later)
            );
        }
        if rel.starts_with(".pc") {
            bail!("File {} is located below .pc/", rel.display());
        }
        if rel.starts_with("patches") {
            bail!("File {} is located below patches/", rel.display());
        }
        let backup = state.backup_dir(&patch).join(&rel);
        if backup.exists() {
            eprintln!(
                "File {} is already in patch {}",
                rel.display(),
                state.display_patch(&patch)
            );
            continue;
        }
        create_parent_dir(&backup)?;
        let src = state.root.join(&rel);
        if new {
            File::create(&backup)?;
        } else if src.exists() {
            copy_keeping_hard_links(&src, &backup)
                .with_context(|| format!("Failed to back up file {}", rel.display()))?;
        } else {
            File::create(&backup)?;
        }
        println!(
            "File {} added to patch {}",
            rel.display(),
            state.display_patch(&patch)
        );
    }
    Ok(())
}

pub(crate) fn cmd_remove(args: Vec<OsString>) -> Result<()> {
    let mut parser = lexopt::Parser::from_args(args);
    let mut patch = None::<String>;
    let mut files = Vec::new();
    while let Some(arg) = parser.next()? {
        match arg {
            Short('P') => patch = Some(parser.value()?.string()?),
            Value(v) => files.push(PathBuf::from(v)),
            _ => bail!("{}", usage("remove")),
        }
    }
    let state = State::discover(false)?;
    let patch = applied_patch(&state, patch.as_deref())?;
    for f in files {
        let rel = state.rel_from_cwd(&f);
        let _ = fs::remove_file(state.backup_dir(&patch).join(&rel));
        println!(
            "File {} removed from patch {}",
            rel.display(),
            state.display_patch(&patch)
        );
    }
    Ok(())
}

pub(crate) fn cmd_revert(args: Vec<OsString>) -> Result<()> {
    let mut parser = lexopt::Parser::from_args(args);
    let mut patch = None::<String>;
    let mut files = Vec::new();
    while let Some(arg) = parser.next()? {
        match arg {
            Short('P') => patch = Some(parser.value()?.string()?),
            Value(v) => files.push(PathBuf::from(v)),
            _ => bail!("{}", usage("revert")),
        }
    }
    let state = State::discover(false)?;
    let patch = applied_patch(&state, patch.as_deref())?;
    for f in files {
        let rel = state.rel_from_cwd(&f);
        let expected = expected_file(&state, &patch, &rel);
        let deleted = expected_deleted(&state, &patch)?.contains(&rel);
        let src = if expected.exists() {
            expected
        } else {
            state.backup_dir(&patch).join(&rel)
        };
        let dst = state.root.join(&rel);
        if deleted || src.metadata()?.len() == 0 {
            if !dst.exists() {
                println!("File {} is unchanged", rel.display());
                continue;
            }
            let _ = fs::remove_file(&dst);
        } else {
            if dst.exists() && fs::read(&dst)? == fs::read(&src)? {
                println!("File {} is unchanged", rel.display());
                continue;
            }
            fs::copy(src, dst)?;
        }
        println!(
            "Changes to {} in patch {} reverted",
            rel.display(),
            state.display_patch(&patch)
        );
    }
    Ok(())
}

pub(crate) fn cmd_edit(args: Vec<OsString>) -> Result<()> {
    let state = State::discover(false)?;
    let mut parser = lexopt::Parser::from_args(args.clone());
    let mut files = Vec::new();
    while let Some(arg) = parser.next()? {
        match arg {
            Short('P') => {
                let _ = parser.value()?;
            }
            Value(v) => files.push(state.rel_from_cwd(Path::new(&v))),
            _ => {}
        }
    }
    cmd_add(args.clone())?;
    let editor = env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    let status = Command::new(editor)
        .current_dir(&state.root)
        .args(&files)
        .status()?;
    if !status.success() {
        bail!("editor failed")
    }
    let patch = state.top()?;
    for file in files {
        let backup = state.backup_dir(&patch).join(&file);
        if backup.exists() && backup.metadata()?.len() == 0 && !state.root.join(&file).exists() {
            let _ = fs::remove_file(backup);
            println!(
                "File {} removed from patch {}",
                file.display(),
                state.display_patch(&patch)
            );
        }
    }
    Ok(())
}

pub(crate) fn cmd_snapshot(args: Vec<OsString>) -> Result<()> {
    let state = State::discover(false)?;
    state.require_series()?;
    let snap = snapshot_dir(&state);
    let _ = fs::remove_dir_all(&snap);
    if !args.iter().any(|a| a == "-d") {
        fs::create_dir_all(&snap)?;

        let mut seen = HashSet::new();
        for patch in state.applied()? {
            for rel in state.tracked_files(&patch)? {
                if !seen.insert(rel.clone()) {
                    continue;
                }
                let src = state.root.join(&rel);
                let dst = snap.join(&rel);
                create_parent_dir(&dst)?;
                if src.exists() {
                    fs::copy(&src, &dst)
                        .with_context(|| format!("failed to snapshot {}", rel.display()))?;
                } else {
                    File::create(&dst)
                        .with_context(|| format!("failed to snapshot {}", rel.display()))?;
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn cmd_upgrade(_args: Vec<OsString>) -> Result<()> {
    State::discover(true)?.create_db()
}
