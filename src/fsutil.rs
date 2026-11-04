use anyhow::Result;
use std::fs::{self, File};
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use tempfile::NamedTempFile;

pub(crate) fn create_parent_dir(path: &Path) -> io::Result<()> {
    match path.parent() {
        Some(parent) => fs::create_dir_all(parent),
        None => Ok(()),
    }
}

/// `fs::copy`, creating `dst`'s directory first.
pub(crate) fn copy_creating_dirs(src: &Path, dst: &Path) -> io::Result<()> {
    create_parent_dir(dst)?;
    fs::copy(src, dst).map(drop)
}

/// Replace `dst` with a copy of `src` via a temporary file, so readers never
/// see a partial file and other hard links to `dst` are left alone.
pub(crate) fn copy_to_new_inode(src: &Path, dst: &Path) -> Result<()> {
    let parent = dst.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = NamedTempFile::new_in(parent)?;
    io::copy(&mut File::open(src)?, tmp.as_file_mut())?;
    tmp.as_file()
        .set_permissions(fs::metadata(src)?.permissions())?;
    tmp.persist(dst).map_err(|err| err.error)?;
    Ok(())
}

/// Copy `src` to `dst`. When `src` has other hard links, `dst` takes over
/// that inode so the links stay intact, and `src` gets a private copy.
pub(crate) fn copy_keeping_hard_links(src: &Path, dst: &Path) -> Result<()> {
    if fs::metadata(src)?.nlink() > 1 {
        let _ = fs::remove_file(dst);
        fs::hard_link(src, dst)?;
        return copy_to_new_inode(src, src);
    }
    copy_to_new_inode(src, dst)
}

pub(crate) fn copy_preserving_times(src: &Path, dst: &Path) -> Result<()> {
    fs::copy(src, dst)?;
    if let Ok(metadata) = fs::metadata(src) {
        let mtime = filetime::FileTime::from_last_modification_time(&metadata);
        let atime = filetime::FileTime::from_last_access_time(&metadata);
        let _ = filetime::set_file_times(dst, atime, mtime);
    }
    Ok(())
}

pub(crate) fn move_if_exists(from: &Path, to: &Path) -> Result<()> {
    if from.exists() {
        create_parent_dir(to)?;
        fs::rename(from, to)?;
    }
    Ok(())
}

/// Whether two paths have the same contents, treating "both missing" as equal.
pub(crate) fn files_match(a: &Path, b: &Path) -> Result<bool> {
    match (a.exists(), b.exists()) {
        (true, true) => {}
        (a_exists, b_exists) => return Ok(a_exists == b_exists),
    }
    if a.metadata()?.len() != b.metadata()?.len() {
        return Ok(false);
    }
    Ok(fs::read(a)? == fs::read(b)?)
}

pub(crate) fn is_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink())
}

/// The `foo~` backup path quilt uses for `--backup`.
pub(crate) fn tilde_backup(path: &Path) -> std::path::PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push("~");
    path.with_file_name(name)
}
