use std::path::{Component, Path, PathBuf};

use crate::{ApplyOptions, Error, FilePatch, Result};

/// The path a patch reads from, or `None` for `/dev/null`.
pub(crate) fn source_path(patch: &FilePatch, reverse: bool) -> Option<&str> {
    if reverse {
        patch.new_path.as_file()
    } else {
        patch.old_path.as_file()
    }
}

/// The path a patch writes to, or `None` for `/dev/null`.
pub(crate) fn target_path(patch: &FilePatch, reverse: bool) -> Option<&str> {
    if reverse {
        patch.old_path.as_file()
    } else {
        patch.new_path.as_file()
    }
}

pub(crate) fn patch_changes_symlink(patch: &FilePatch, reverse: bool) -> bool {
    let (preferred, fallback) = if reverse {
        (patch.old_file_mode, patch.new_file_mode)
    } else {
        (patch.new_file_mode, patch.old_file_mode)
    };
    preferred
        .or(fallback)
        .is_some_and(|mode| mode & 0o170000 == 0o120000)
}

pub(crate) fn is_creation_only_patch(patch: &FilePatch) -> bool {
    !patch.hunks.is_empty()
        && patch
            .hunks
            .iter()
            .all(|hunk| hunk.old_start == 0 || hunk.old_count == 0)
}

pub(crate) fn empties_file(patch: &FilePatch) -> bool {
    !patch.hunks.is_empty()
        && patch
            .hunks
            .iter()
            .all(|hunk| hunk.new_start == 0 || hunk.new_count == 0)
}

pub(crate) fn root_allows_absolute_patch_paths(root: &Path) -> bool {
    root.canonicalize()
        .is_ok_and(|canonical| canonical.parent().is_none())
}

pub(crate) fn effective_strip(patch: &FilePatch, options: &ApplyOptions) -> usize {
    if patch.git_paths { 0 } else { options.strip }
}

pub(crate) fn selected_file_target(
    root: &Path,
    patch: &FilePatch,
    options: &ApplyOptions,
    allow_absolute: bool,
) -> Result<PathBuf> {
    if let Some(target_file) = &options.target_file {
        return Ok(target_file.clone());
    }
    let strip = effective_strip(patch, options);
    choose_target_path(root, patch, options.reverse, strip, allow_absolute)
}

fn choose_target_path(
    root: &Path,
    patch: &FilePatch,
    reverse: bool,
    strip: usize,
    allow_absolute: bool,
) -> Result<PathBuf> {
    match (source_path(patch, reverse), target_path(patch, reverse)) {
        (None, Some(path)) | (Some(path), None) => strip_patch_path(path, strip, allow_absolute),
        (Some(source), Some(target)) => {
            let stripped_target = match strip_patch_path(target, strip, allow_absolute) {
                Ok(path) => path,
                Err(Error::DangerousPath { .. }) => {
                    let stripped_source = strip_patch_path(source, strip, allow_absolute)?;
                    if root.join(&stripped_source).exists() {
                        return Ok(stripped_source);
                    }
                    return Err(Error::StripPath {
                        path: target.to_string(),
                        strip,
                    });
                }
                Err(error) => return Err(error),
            };
            if patch.git_operation.is_some() || root.join(&stripped_target).exists() {
                return Ok(stripped_target);
            }
            let stripped_source = strip_patch_path(source, strip, allow_absolute)?;
            if root.join(&stripped_source).exists()
                || (!patch.git_paths && is_creation_only_patch(patch))
            {
                return Ok(stripped_source);
            }
            Ok(stripped_target)
        }
        (None, None) => Err(no_usable_path()),
    }
}

pub(crate) fn no_usable_path() -> Error {
    Error::Parse {
        line: 0,
        message: "patch has no usable file path".to_string(),
    }
}

/// Strips `strip` leading components from a header path, rejecting paths
/// that would escape the working directory.
pub(crate) fn strip_patch_path(raw: &str, strip: usize, allow_absolute: bool) -> Result<PathBuf> {
    if let Some(path) = fast_strip_patch_path(raw, strip) {
        return Ok(path);
    }

    let strip_error = || Error::StripPath {
        path: raw.to_string(),
        strip,
    };
    let is_absolute = raw.starts_with('/');
    let components: Vec<&str> = raw
        .split('/')
        .filter(|component| !component.is_empty() && *component != ".")
        .collect();
    let kept = match components.get(strip..) {
        Some(kept) if !kept.is_empty() => kept,
        _ => return Err(strip_error()),
    };
    if kept.contains(&"..") || (is_absolute && !allow_absolute) {
        return Err(Error::DangerousPath {
            path: if is_absolute && strip == 0 {
                raw.to_string()
            } else {
                kept.join("/")
            },
        });
    }

    let mut out = if is_absolute && allow_absolute {
        PathBuf::from("/")
    } else {
        PathBuf::new()
    };
    out.extend(kept);
    if out.components().any(|component| {
        matches!(component, Component::ParentDir | Component::Prefix(_))
            || (matches!(component, Component::RootDir) && !allow_absolute)
    }) {
        return Err(strip_error());
    }
    Ok(out)
}

/// Handles the common `-p1` on `a/some/path` case without allocating a
/// component list.
fn fast_strip_patch_path(raw: &str, strip: usize) -> Option<PathBuf> {
    if strip != 1 || raw.starts_with('/') {
        return None;
    }
    let (stripped, kept) = raw.split_once('/')?;
    if matches!(stripped, "" | "." | "..") || kept.is_empty() {
        return None;
    }
    if kept
        .split('/')
        .any(|component| matches!(component, "" | "." | ".."))
    {
        return None;
    }

    let path = PathBuf::from(kept);
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::Prefix(_) | Component::RootDir
        )
    }) {
        return None;
    }
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PatchPath, parse_patch};

    #[test]
    fn strips_plain_diff_header_paths() {
        let patch = parse_patch(
            "diff -ruN a/deleted b/deleted\n--- a/deleted\n+++ /dev/null\n@@ -1 +0,0 @@\n-data\n",
        )
        .unwrap();
        let old_path = patch.files[0].old_path.as_file().unwrap();
        assert_eq!(
            strip_patch_path(old_path, 0, false).unwrap(),
            PathBuf::from("deleted")
        );
        assert_eq!(patch.files[0].new_path, PatchPath::DevNull);
    }

    #[test]
    fn strips_git_style_paths() {
        let path = strip_patch_path("a/src/lib.rs", 1, false).unwrap();
        assert_eq!(path, PathBuf::from("src/lib.rs"));
    }
}
