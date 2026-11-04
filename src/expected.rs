//! The "expected state" q records after pushing or refreshing a patch: a copy
//! of every file the patch touches as the patch left it, kept under
//! `.pc/.q/<patch>/`. While the patch file and its tracked files are
//! unchanged, this lets pop and diff compare against it instead of
//! re-applying the patch in a scratch tree.

use crate::fsutil::{copy_creating_dirs, files_match};
use crate::patchfile::patch_text;
use crate::state::{State, sort_tracked};
use anyhow::Result;
use std::collections::HashSet;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

fn meta_dir(state: &State, patch: &str) -> PathBuf {
    state.pc_dir.join(".q").join(patch)
}

pub(crate) fn expected_file(state: &State, patch: &str, file: &Path) -> PathBuf {
    meta_dir(state, patch).join("expected").join(file)
}

/// Files the patch is expected to have deleted.
pub(crate) fn expected_deleted(state: &State, patch: &str) -> Result<HashSet<PathBuf>> {
    let list = meta_dir(state, patch).join("deleted");
    if !list.exists() {
        return Ok(HashSet::new());
    }
    Ok(fs::read_to_string(list)?
        .lines()
        .map(PathBuf::from)
        .collect())
}

/// Record the working tree state of `tracked` as `patch`'s expected state,
/// using the patch file's current text.
pub(crate) fn record_expected_state(state: &State, patch: &str, tracked: &[PathBuf]) -> Result<()> {
    let text = patch_text(&state.patch_path(patch))?;
    record_expected_state_with_text(state, patch, tracked, &text)
}

pub(crate) fn record_expected_state_with_text(
    state: &State,
    patch: &str,
    tracked: &[PathBuf],
    patch_text: &str,
) -> Result<()> {
    let meta = meta_dir(state, patch);
    let expected = meta.join("expected");
    let _ = fs::remove_dir_all(&meta);
    fs::create_dir_all(&expected)?;
    let mut tracked = tracked.to_vec();
    sort_tracked(&mut tracked);
    let mut deleted = String::new();
    for file in &tracked {
        let src = state.root.join(file);
        if src.exists() {
            copy_creating_dirs(&src, &expected.join(file))?;
        } else {
            deleted.push_str(&file.to_string_lossy());
            deleted.push('\n');
        }
    }
    fs::write(meta.join("deleted"), deleted)?;
    fs::write(meta.join("patch"), patch_text)?;
    fs::write(meta.join("files"), file_list(&tracked))?;
    fs::create_dir_all(state.backup_dir(patch))?;
    File::create(state.backup_dir(patch).join(".timestamp"))?;
    Ok(())
}

fn file_list(files: &[PathBuf]) -> String {
    files
        .iter()
        .map(|file| format!("{}\n", file.to_string_lossy()))
        .collect()
}

/// Whether the recorded expected state still describes this patch: same patch
/// text and same tracked files as when it was recorded.
pub(crate) fn expected_state_is_current(state: &State, patch: &str) -> bool {
    let meta = meta_dir(state, patch);
    let (Ok(recorded_patch), Ok(recorded_files)) = (
        fs::read_to_string(meta.join("patch")),
        fs::read_to_string(meta.join("files")),
    ) else {
        return false;
    };
    patch_text(&state.patch_path(patch)).is_ok_and(|text| text == recorded_patch)
        && state
            .tracked_files(patch)
            .is_ok_and(|files| file_list(&files) == recorded_files)
}

/// Whether the working tree matches `patch`'s expected state for `files`.
pub(crate) fn expected_state_matches(
    state: &State,
    patch: &str,
    files: &[PathBuf],
) -> Result<bool> {
    let deleted = expected_deleted(state, patch)?;
    for file in files {
        let live = state.root.join(file);
        if deleted.contains(file) {
            if live.exists() {
                return Ok(false);
            }
            continue;
        }
        let expected = expected_file(state, patch, file);
        if !expected.exists() || !files_match(&expected, &live)? {
            return Ok(false);
        }
    }
    Ok(true)
}
