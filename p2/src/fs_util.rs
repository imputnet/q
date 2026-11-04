use std::ffi::OsString;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};

use crate::{Error, Result, io_error};

pub(crate) fn read_text(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(io_error(path))
}

pub(crate) fn read_lines(path: &Path) -> Result<Vec<String>> {
    Ok(read_text(path)?
        .split_inclusive('\n')
        .map(str::to_string)
        .collect())
}

pub(crate) fn split_lines(text: &str) -> Vec<&str> {
    text.split_inclusive('\n').collect()
}

pub(crate) struct TargetStatus {
    pub(crate) existed: bool,
    pub(crate) is_symlink: bool,
    /// Permissions of an existing regular file; `None` for symlinks and missing files.
    pub(crate) permissions: Option<fs::Permissions>,
}

pub(crate) fn target_status(path: &Path) -> Result<TargetStatus> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            let is_symlink = metadata.file_type().is_symlink();
            Ok(TargetStatus {
                existed: true,
                is_symlink,
                permissions: (!is_symlink).then(|| metadata.permissions()),
            })
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(TargetStatus {
            existed: false,
            is_symlink: false,
            permissions: None,
        }),
        Err(source) => Err(io_error(path)(source)),
    }
}

#[cfg(unix)]
pub(crate) fn is_read_only_permissions(permissions: &fs::Permissions) -> bool {
    use std::os::unix::fs::PermissionsExt;

    permissions.mode() & 0o222 == 0
}

#[cfg(not(unix))]
pub(crate) fn is_read_only_permissions(permissions: &fs::Permissions) -> bool {
    permissions.readonly()
}

#[cfg(unix)]
pub(crate) fn set_file_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(io_error(path))
}

#[cfg(not(unix))]
pub(crate) fn set_file_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

/// Replaces `path` with `contents` via a temporary file and rename, so readers
/// never see a partially written file.
pub(crate) fn write_output_file(
    path: &Path,
    contents: String,
    permissions: Option<fs::Permissions>,
) -> Result<()> {
    let Some(parent) = path.parent() else {
        return fs::write(path, contents).map_err(io_error(path));
    };
    let temp_path = create_temp_output_file(parent, &contents)?;
    if let Some(permissions) = permissions {
        fs::set_permissions(&temp_path, permissions).map_err(io_error(&temp_path))?;
    }
    fs::rename(&temp_path, path).map_err(|source| {
        let _ = fs::remove_file(&temp_path);
        io_error(path)(source)
    })
}

fn create_temp_output_file(parent: &Path, contents: &str) -> Result<PathBuf> {
    let process_id = std::process::id();
    for attempt in 0..1000 {
        let temp_path = parent.join(format!(".p2-tmp-{process_id}-{attempt}"));
        let mut file = match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(source) => return Err(io_error(&temp_path)(source)),
        };
        if let Err(source) = file.write_all(contents.as_bytes()) {
            let _ = fs::remove_file(&temp_path);
            return Err(io_error(&temp_path)(source));
        }
        return Ok(temp_path);
    }
    Err(Error::Io {
        path: parent.to_path_buf(),
        source: std::io::Error::new(
            ErrorKind::AlreadyExists,
            "could not create temporary output file",
        ),
    })
}

pub(crate) fn create_parent_dirs(path: &Path) -> Result<()> {
    match path.parent() {
        Some(parent) => fs::create_dir_all(parent).map_err(io_error(parent)),
        None => Ok(()),
    }
}

/// Removes now-empty directories between `path` and `root` (exclusive).
pub(crate) fn remove_empty_parent_dirs(root: &Path, path: &Path) -> Result<()> {
    let mut current = path.parent();
    while let Some(directory) = current {
        if directory == root || !directory.starts_with(root) {
            break;
        }
        match fs::remove_dir(directory) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) if error.kind() == ErrorKind::DirectoryNotEmpty => break,
            Err(source) => return Err(io_error(directory)(source)),
        }
        current = directory.parent();
    }
    Ok(())
}

/// Reads a symlink target as patchable text: one line, newline-terminated.
#[cfg(unix)]
pub(crate) fn read_symlink_text(path: &Path) -> Result<String> {
    let target = fs::read_link(path).map_err(io_error(path))?;
    let mut line = target.to_string_lossy().into_owned();
    line.push('\n');
    Ok(line)
}

#[cfg(not(unix))]
pub(crate) fn read_symlink_text(path: &Path) -> Result<String> {
    Err(symlinks_unsupported(path))
}

#[cfg(unix)]
pub(crate) fn write_symlink_output(path: &Path, contents: &str) -> Result<()> {
    create_parent_dirs(path)?;
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(source) => return Err(io_error(path)(source)),
    }
    std::os::unix::fs::symlink(contents.trim_end_matches('\n'), path).map_err(io_error(path))
}

#[cfg(not(unix))]
pub(crate) fn write_symlink_output(path: &Path, _contents: &str) -> Result<()> {
    Err(symlinks_unsupported(path))
}

#[cfg(not(unix))]
fn symlinks_unsupported(path: &Path) -> Error {
    Error::Io {
        path: path.to_path_buf(),
        source: std::io::Error::new(ErrorKind::Unsupported, "symbolic links are not supported"),
    }
}

/// Whether `target`'s parent directory resolves (following symlinks) to a
/// location outside `root`, or cannot be resolved.
pub(crate) fn path_parent_is_invalid(root: &Path, target: &Path) -> Result<bool> {
    let Some(parent) = target.parent() else {
        return Ok(false);
    };
    resolve_existing_path_prefix(root, &root.join(parent))
}

fn resolve_existing_path_prefix(root: &Path, path: &Path) -> Result<bool> {
    // A stack of components still to walk; the next component is on top.
    let mut pending = normal_components(path.strip_prefix(root).unwrap_or(path));
    pending.reverse();

    let mut current = root.to_path_buf();
    let mut followed = 0;
    while let Some(component) = pending.pop() {
        let candidate = current.join(&component);
        let metadata = match fs::symlink_metadata(&candidate) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
            Err(_) => return Ok(true),
        };
        if !metadata.file_type().is_symlink() {
            current = candidate;
            continue;
        }
        followed += 1;
        if followed > 40 {
            return Ok(true);
        }
        let link = fs::read_link(&candidate).map_err(io_error(&candidate))?;
        let resolved = normalize_path(&current.join(link));
        if !resolved.starts_with(root) {
            return Ok(true);
        }
        let replacement = normal_components(resolved.strip_prefix(root).unwrap_or(&resolved));
        pending.extend(replacement.into_iter().rev());
        current = root.to_path_buf();
    }
    Ok(false)
}

fn normal_components(path: &Path) -> Vec<OsString> {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(component) => Some(component.to_os_string()),
            _ => None,
        })
        .collect()
}

/// Lexically resolves `.` and `..` components.
fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(component) => out.push(component),
            Component::RootDir | Component::Prefix(_) => out.push(component.as_os_str()),
        }
    }
    out
}
