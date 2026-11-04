//! Reading and writing patch files: header/body split, touched file names,
//! and transparent (de)compression.

use crate::fsutil::{create_parent_dir, is_symlink};
use crate::state::PatchEntry;
use anyhow::{Result, bail};
use std::collections::HashSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::NamedTempFile;

/// Whether `line` starts the diff part of a patch; anything before is header.
fn is_diff_start(line: &str) -> bool {
    ["Index: ", "--- ", "*** ", "diff "]
        .iter()
        .any(|prefix| line.starts_with(prefix))
}

pub(crate) fn has_diff(text: &str) -> bool {
    text.lines().any(is_diff_start)
}

/// Split patch text into header and body, normalizing line endings to `\n`.
fn split_patch(text: &str) -> (String, String) {
    let (mut header, mut body) = (String::new(), String::new());
    let mut in_body = false;
    for line in text.lines() {
        in_body |= is_diff_start(line);
        let out = if in_body { &mut body } else { &mut header };
        out.push_str(line);
        out.push('\n');
    }
    (header, body)
}

pub(crate) fn header_of(text: &str) -> String {
    split_patch(text).0
}

pub(crate) fn body_of(text: &str) -> String {
    split_patch(text).1
}

/// The (decompressed) text of a patch file; empty if it doesn't exist.
pub(crate) fn patch_text(path: &Path) -> Result<String> {
    if !path.exists() {
        return Ok(String::new());
    }
    let readable = readable_patch_file(path)?;
    Ok(fs::read_to_string(readable.path())?)
}

pub(crate) fn patch_header(path: &Path) -> Result<String> {
    Ok(header_of(&patch_text(path)?))
}

pub(crate) fn patch_body(path: &Path) -> Result<String> {
    Ok(body_of(&patch_text(path)?))
}

pub(crate) fn strip_trailing_whitespace(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        out.push_str(line.trim_end_matches([' ', '\t']));
        out.push('\n');
    }
    out
}

/// Files a patch touches, sorted.
pub(crate) fn filenames_in_patch(path: &Path, strip: usize) -> Vec<PathBuf> {
    let mut files = filenames_in_patch_order(path, strip);
    files.sort();
    files
}

/// Files a patch touches, in the order the patch mentions them.
pub(crate) fn filenames_in_patch_order(path: &Path, strip: usize) -> Vec<PathBuf> {
    patch_text(path)
        .map(|text| filenames_in_text_order(&text, strip))
        .unwrap_or_default()
}

pub(crate) fn filenames_in_text_order(text: &str, strip: usize) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    let mut add = |old: Option<String>, new: String| {
        let mut name = match old {
            Some(old) if old != "/dev/null" => old,
            _ => new,
        };
        if name == "/dev/null" {
            return;
        }
        if strip == 0 && name.ends_with(".orig") {
            name.truncate(name.len() - ".orig".len());
        }
        let path: PathBuf = Path::new(&name).components().skip(strip).collect();
        if !path.as_os_str().is_empty() && seen.insert(path.clone()) {
            files.push(path);
        }
    };
    // Unified diffs name files in `---`/`+++` pairs, context diffs in
    // `***`/`---` pairs.
    let mut unified_old = None;
    let mut context_old = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("*** ") {
            let name = patch_label_name(rest);
            if !name.chars().all(|c| c.is_ascii_digit()) {
                context_old = Some(name);
            }
        } else if let Some(rest) = line.strip_prefix("--- ") {
            match context_old.take() {
                Some(old) => add(Some(old), patch_label_name(rest)),
                None => unified_old = Some(patch_label_name(rest)),
            }
        } else if let Some(rest) = line.strip_prefix("+++ ") {
            add(unified_old.take(), patch_label_name(rest));
        }
    }
    files
}

/// The per-file sections of a patch body that touch one of `wanted`.
pub(crate) fn body_for_files(body: &str, strip: usize, wanted: &HashSet<PathBuf>) -> String {
    let mut out = String::new();
    let mut block = String::new();
    let mut flush = |block: &mut String| {
        if filenames_in_text_order(block, strip)
            .iter()
            .any(|file| wanted.contains(file))
        {
            out.push_str(block);
        }
        block.clear();
    };
    for line in body.split_inclusive('\n') {
        if line.starts_with("Index: ") && !block.is_empty() {
            flush(&mut block);
        }
        block.push_str(line);
    }
    flush(&mut block);
    out
}

/// The file name in a `---`/`+++`/`***` label, without timestamp or quotes.
fn patch_label_name(label: &str) -> String {
    let label = label.split_once('\t').map_or(label, |(name, _)| name);
    label
        .split_once(" 20")
        .or_else(|| label.split_once(" 19"))
        .map_or(label, |(name, _)| name)
        .trim_matches('"')
        .to_string()
}

/// A patch file in plain text: the file itself, or a decompressed copy.
pub(crate) enum ReadablePatch {
    Original(PathBuf),
    Temp(NamedTempFile),
}

impl ReadablePatch {
    pub(crate) fn path(&self) -> &Path {
        match self {
            Self::Original(path) => path,
            Self::Temp(file) => file.path(),
        }
    }
}

fn compression_tool(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_str()?;
    [(".gz", "gzip"), (".bz2", "bzip2"), (".xz", "xz")]
        .into_iter()
        .find(|(suffix, _)| name.ends_with(suffix))
        .map(|(_, tool)| tool)
}

pub(crate) fn readable_patch_file(path: &Path) -> Result<ReadablePatch> {
    let Some(tool) = compression_tool(path) else {
        return Ok(ReadablePatch::Original(path.to_path_buf()));
    };
    let output = Command::new(tool).arg("-dc").arg(path).output()?;
    if !output.status.success() {
        bail!("failed to decompress {}", path.display());
    }
    let mut tmp = NamedTempFile::new()?;
    tmp.write_all(&output.stdout)?;
    Ok(ReadablePatch::Temp(tmp))
}

/// The patch file to feed to `patch` for `entry`. A reversed `-p0` patch has
/// its `.orig` old-file labels stripped so patch finds the working files.
pub(crate) fn patch_file_for_apply(path: &Path, entry: &PatchEntry) -> Result<ReadablePatch> {
    let readable = readable_patch_file(path)?;
    if !(strip_level(entry) == 0 && is_reversed(entry)) {
        return Ok(readable);
    }
    let text = fs::read_to_string(readable.path())?;
    let mut tmp = NamedTempFile::new()?;
    for line in text.lines() {
        match line
            .strip_prefix("--- ")
            .and_then(|name| name.strip_suffix(".orig"))
        {
            Some(name) => writeln!(tmp, "--- {name}")?,
            None => writeln!(tmp, "{line}")?,
        }
    }
    Ok(ReadablePatch::Temp(tmp))
}

/// Write a patch file, compressing to match its extension. A symlinked patch
/// is written through to its target when that is writable.
pub(crate) fn write_patch_text(path: &Path, text: &str) -> Result<()> {
    create_parent_dir(path)?;
    let Some(tool) = compression_tool(path) else {
        let target = symlink_write_target(path).unwrap_or_else(|| path.to_path_buf());
        let parent = target.parent().unwrap_or_else(|| Path::new("."));
        let mut tmp = NamedTempFile::new_in(parent)?;
        tmp.write_all(text.as_bytes())?;
        tmp.persist(&target).map_err(|err| err.error)?;
        return Ok(());
    };
    let mut child = Command::new(tool)
        .arg("-c")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(text.as_bytes())?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!("failed to compress {}", path.display());
    }
    fs::write(path, output.stdout)?;
    Ok(())
}

fn symlink_write_target(path: &Path) -> Option<PathBuf> {
    if !is_symlink(path) {
        return None;
    }
    let target = path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(fs::read_link(path).ok()?);
    let writable = !fs::metadata(&target).ok()?.permissions().readonly();
    writable.then_some(target)
}

/// The `-p` value written in a series entry, if any.
pub(crate) fn strip_arg(entry: &PatchEntry) -> Option<&str> {
    entry.args.iter().find_map(|arg| arg.strip_prefix("-p"))
}

/// The strip level a series entry applies with; 1 unless it says otherwise.
pub(crate) fn strip_level(entry: &PatchEntry) -> usize {
    strip_arg(entry)
        .and_then(|level| level.parse().ok())
        .unwrap_or(1)
}

pub(crate) fn is_reversed(entry: &PatchEntry) -> bool {
    entry.args.iter().any(|arg| arg == "-R")
}

#[cfg(test)]
mod tests {
    use super::{body_of, filenames_in_text_order, header_of};
    use std::path::PathBuf;

    #[test]
    fn extracts_filenames_using_strip_level() {
        let text = "--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-a\n+b\n";
        assert_eq!(
            filenames_in_text_order(text, 1),
            [PathBuf::from("src/lib.rs")]
        );
    }

    #[test]
    fn extracts_filenames_from_context_diffs() {
        let text = "*** a/old.c\n--- b/old.c\n***************\n*** 1 ****\n";
        assert_eq!(filenames_in_text_order(text, 1), [PathBuf::from("old.c")]);
    }

    #[test]
    fn splits_header_from_body() {
        let text = "Subject: x\n\nIndex: f\n--- f\n+++ f\n";
        assert_eq!(header_of(text), "Subject: x\n\n");
        assert_eq!(body_of(text), "Index: f\n--- f\n+++ f\n");
    }
}
