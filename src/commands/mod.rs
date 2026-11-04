mod absorb;
mod diff;
mod files;
pub(crate) mod graph;
pub(crate) mod mail;
mod query;
mod refresh;
mod setup;
mod stack;

pub(crate) use absorb::{cmd_absorb, cmd_scratch};
pub(crate) use diff::cmd_diff;
pub(crate) use files::{cmd_add, cmd_edit, cmd_remove, cmd_revert, cmd_snapshot, cmd_upgrade};
pub(crate) use graph::cmd_graph;
pub(crate) use mail::cmd_mail;
pub(crate) use query::{
    cmd_annotate, cmd_applied, cmd_files, cmd_grep, cmd_next, cmd_patches, cmd_previous,
    cmd_series, cmd_top, cmd_unapplied,
};
pub(crate) use refresh::{cmd_header, cmd_refresh};
pub(crate) use setup::cmd_setup;
pub(crate) use stack::{
    cmd_delete, cmd_fold, cmd_fork, cmd_import, cmd_new, cmd_pop, cmd_push, cmd_rename,
};

use crate::state::State;
use anyhow::{Context, Result, bail};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub fn applied_patch(state: &State, patch: Option<&str>) -> Result<String> {
    state.require_series()?;
    let applied = state.applied()?;
    if let Some(name) = patch {
        let patch = state.find_patch(name)?;
        if !applied.contains(&patch) {
            bail!("Patch {} is not applied", state.display_patch(&patch));
        }
        Ok(patch)
    } else {
        state.top()
    }
}

fn newer_applied_patches(state: &State, patch: &str) -> Result<Vec<String>> {
    let applied = state.applied()?;
    let Some(pos) = applied.iter().position(|p| p == patch) else {
        return Ok(Vec::new());
    };
    Ok(applied[pos + 1..].to_vec())
}

fn newer_patch_tracking_file(state: &State, patch: &str, file: &Path) -> Result<Option<String>> {
    for later in newer_applied_patches(state, patch)? {
        if state
            .tracked_files(&later)?
            .iter()
            .any(|tracked| tracked == file)
        {
            return Ok(Some(later));
        }
    }
    Ok(None)
}

/// Which of `tracked` (files of `patch`) patches applied after it also modify.
fn files_modified_by_newer_patches(
    state: &State,
    patch: &str,
    tracked: &[PathBuf],
) -> Result<Vec<PathBuf>> {
    let mut modified = Vec::new();
    if tracked.is_empty() {
        return Ok(modified);
    }
    let tracked: HashSet<&PathBuf> = tracked.iter().collect();
    let mut seen = HashSet::new();
    for later in newer_applied_patches(state, patch)? {
        for file in state.tracked_files(&later)? {
            if tracked.contains(&file) && seen.insert(file.clone()) {
                modified.push(file);
            }
        }
    }
    Ok(modified)
}

/// The applied patches from `first` through `last`, for `--combine`. `first`
/// of `-` means the bottom of the stack.
fn applied_range(
    state: &State,
    applied: &[String],
    first: &str,
    last: &str,
) -> Result<Vec<String>> {
    let first = if first == "-" {
        applied.first().cloned().context("No patches applied")?
    } else {
        state.find_patch(first)?
    };
    let position = |patch: &str| {
        applied
            .iter()
            .position(|p| p == patch)
            .with_context(|| format!("Patch {} is not applied", state.display_patch(patch)))
    };
    let (start, end) = (position(&first)?, position(last)?);
    if start > end {
        bail!(
            "Patch {} not applied before patch {}",
            state.display_patch(&first),
            state.display_patch(last)
        );
    }
    Ok(applied[start..=end].to_vec())
}

fn refresh_marker(state: &State, patch: &str) -> PathBuf {
    state.pc_dir.join(format!("{patch}~refresh"))
}

fn snapshot_dir(state: &State) -> PathBuf {
    state.pc_dir.join(".snap")
}

fn next_filename(patch: &str) -> String {
    let (stem, ext) = patch
        .rsplit_once('.')
        .map(|(a, b)| (a, format!(".{b}")))
        .unwrap_or((patch, String::new()));
    if let Some((base, n)) = stem.rsplit_once('-')
        && let Ok(n) = n.parse::<u32>()
    {
        return format!("{base}-{}{}", n + 1, ext);
    }
    format!("{stem}-2{ext}")
}

#[cfg(test)]
mod tests {
    use super::next_filename;

    #[test]
    fn increments_patch_fork_names() {
        assert_eq!(next_filename("fix.diff"), "fix-2.diff");
        assert_eq!(next_filename("fix-2.patch"), "fix-3.patch");
    }
}
