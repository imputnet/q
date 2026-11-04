use crate::fsutil::create_parent_dir;
use anyhow::{Context, Result, bail};
use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

const DB_VERSION: &str = "2";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PatchEntry {
    pub name: String,
    pub args: Vec<String>,
}

#[derive(Debug)]
pub struct State {
    pub root: PathBuf,
    pub subdir: PathBuf,
    pub patches_dir: PathBuf,
    pub pc_dir: PathBuf,
    pub series_file: PathBuf,
    pub db_file: PathBuf,
    /// Prefix for user-facing patch names; set when `QUILT_PATCHES_PREFIX` is.
    patch_prefix: Option<String>,
}

impl State {
    pub fn discover(create_here: bool) -> Result<Self> {
        let cwd = env::current_dir()?;
        let pc_name = env::var("QUILT_PC").unwrap_or_else(|_| ".pc".to_string());
        let patches_name = env::var("QUILT_PATCHES").unwrap_or_else(|_| "patches".to_string());
        let mut root = cwd.clone();

        if !create_here && !root.join(&pc_name).is_dir() && !root.join(&patches_name).is_dir() {
            let mut cur = cwd.as_path();
            while let Some(parent) = cur.parent() {
                if parent.join(&pc_name).is_dir() || parent.join(&patches_name).is_dir() {
                    root = parent.to_path_buf();
                    break;
                }
                cur = parent;
            }
        }

        let pc_dir = root.join(&pc_name);
        let patches_name = read_trimmed(pc_dir.join(".quilt_patches")).unwrap_or(patches_name);
        let series_name = read_trimmed(pc_dir.join(".quilt_series"))
            .unwrap_or_else(|| env::var("QUILT_SERIES").unwrap_or_else(|_| "series".to_string()));
        let patches_dir = root.join(&patches_name);
        let series_file = if Path::new(&series_name).is_absolute() {
            PathBuf::from(&series_name)
        } else if pc_dir.join(&series_name).is_file() {
            pc_dir.join(&series_name)
        } else if root.join(&series_name).is_file() {
            root.join(&series_name)
        } else {
            patches_dir.join(&series_name)
        };
        let subdir = cwd
            .strip_prefix(&root)
            .unwrap_or(Path::new(""))
            .to_path_buf();
        let patch_prefix = env::var_os("QUILT_PATCHES_PREFIX")
            .filter(|value| !value.is_empty())
            .map(|_| {
                let mut prefix: PathBuf = subdir.components().map(|_| "..").collect();
                prefix.push(&patches_name);
                format!("{}/", prefix.display())
            });

        Ok(Self {
            root,
            subdir,
            patches_dir,
            db_file: pc_dir.join("applied-patches"),
            pc_dir,
            series_file,
            patch_prefix,
        })
    }

    pub fn create_db(&self) -> Result<()> {
        fs::create_dir_all(&self.pc_dir)?;
        write_if_missing(self.pc_dir.join(".version"), DB_VERSION)?;
        write_if_missing(
            self.pc_dir.join(".quilt_patches"),
            self.patches_dir
                .strip_prefix(&self.root)
                .unwrap_or(&self.patches_dir)
                .to_string_lossy()
                .as_ref(),
        )?;
        write_if_missing(
            self.pc_dir.join(".quilt_series"),
            self.series_file
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("series"),
        )?;
        Ok(())
    }

    pub fn require_series(&self) -> Result<()> {
        if !self.series_file.is_file() {
            bail!("No series file found");
        }
        Ok(())
    }

    pub fn entries(&self) -> Result<Vec<PatchEntry>> {
        if !self.series_file.exists() {
            return Ok(Vec::new());
        }
        let text = fs::read_to_string(&self.series_file)?;
        Ok(text.lines().filter_map(parse_series_line).collect())
    }

    /// The patch after `patch` in the series, or the first one for `None`.
    pub fn patch_after(&self, patch: Option<&str>) -> Result<Option<String>> {
        let entries = self.entries()?;
        let index = match patch {
            Some(patch) => match entries.iter().position(|e| e.name == patch) {
                Some(index) => index + 1,
                None => return Ok(None),
            },
            None => 0,
        };
        Ok(entries.get(index).map(|e| e.name.clone()))
    }

    // Series edits work on raw lines, like quilt's, so comments, blank lines
    // and formatting of other entries survive.

    fn series_lines(&self) -> Result<Vec<String>> {
        if !self.series_file.exists() {
            return Ok(Vec::new());
        }
        let text = fs::read_to_string(&self.series_file)?;
        Ok(text.lines().map(str::to_string).collect())
    }

    fn write_series_lines(&self, lines: &[String]) -> Result<()> {
        create_parent_dir(&self.series_file)?;
        let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
        fs::write(&self.series_file, text)?;
        Ok(())
    }

    /// Add `entry` just before the `before` patch, or at the end.
    pub fn insert_in_series(&self, entry: &PatchEntry, before: Option<&str>) -> Result<()> {
        let mut line = entry.name.clone();
        for arg in &entry.args {
            line.push(' ');
            line.push_str(arg);
        }
        let mut lines = self.series_lines()?;
        let index = before
            .and_then(|before| lines.iter().position(|l| series_line_is(l, before)))
            .unwrap_or(lines.len());
        lines.insert(index, line);
        self.write_series_lines(&lines)
    }

    pub fn remove_from_series(&self, patch: &str) -> Result<()> {
        let mut lines = self.series_lines()?;
        lines.retain(|line| !series_line_is(line, patch));
        self.write_series_lines(&lines)
    }

    /// Rename a patch in the series and the applied list.
    pub fn rename_patch(&self, from: &str, to: &str) -> Result<()> {
        self.rename_in_series(from, to)?;
        let applied: Vec<String> = self
            .applied()?
            .into_iter()
            .map(|patch| if patch == from { to.to_string() } else { patch })
            .collect();
        self.write_applied(&applied)
    }

    fn rename_in_series(&self, from: &str, to: &str) -> Result<()> {
        let mut lines = self.series_lines()?;
        for line in &mut lines {
            if series_line_is(line, from) {
                line.replace_range(..from.len(), to);
            }
        }
        self.write_series_lines(&lines)
    }

    /// After a refresh: drop `-R` and record the strip level, leaving out
    /// the default `-p1` (quilt's `change_db_strip_level`).
    pub fn update_series_args(&self, patch: &str, strip: usize) -> Result<()> {
        let mut lines = self.series_lines()?;
        let mut changed = false;
        for line in &mut lines {
            if !series_line_is(line, patch) {
                continue;
            }
            let (fields, comment) = match line.find('#') {
                Some(index) => line.split_at(index),
                None => (line.as_str(), ""),
            };
            let mut args: Vec<String> = fields
                .split_whitespace()
                .skip(1)
                .filter(|arg| *arg != "-R")
                .map(str::to_string)
                .collect();
            let existing = args.iter().position(|arg| arg.starts_with("-p"));
            match (existing, strip) {
                (Some(index), 1) => drop(args.remove(index)),
                (Some(index), _) => args[index] = format!("-p{strip}"),
                (None, 1) => {}
                (None, _) => args.insert(0, format!("-p{strip}")),
            }
            let mut new_line = patch.to_string();
            for arg in &args {
                new_line.push(' ');
                new_line.push_str(arg);
            }
            if !comment.is_empty() {
                new_line.push(' ');
                new_line.push_str(comment);
            }
            if new_line != *line {
                *line = new_line;
                changed = true;
            }
        }
        if changed {
            self.write_series_lines(&lines)?;
        }
        Ok(())
    }

    pub fn applied(&self) -> Result<Vec<String>> {
        if !self.db_file.exists() {
            return Ok(Vec::new());
        }
        Ok(fs::read_to_string(&self.db_file)?
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(ToOwned::to_owned)
            .collect())
    }

    pub fn write_applied(&self, patches: &[String]) -> Result<()> {
        if patches.is_empty() {
            let _ = fs::remove_file(&self.db_file);
            return Ok(());
        }
        fs::create_dir_all(&self.pc_dir)?;
        fs::write(&self.db_file, format!("{}\n", patches.join("\n")))?;
        Ok(())
    }

    pub fn top(&self) -> Result<String> {
        self.applied()?
            .last()
            .cloned()
            .context("No patches applied")
    }

    pub fn find_patch(&self, name: &str) -> Result<String> {
        let name = name
            .strip_prefix("patches/")
            .or_else(|| name.strip_prefix("./patches/"))
            .unwrap_or(name);
        let entries = self.entries()?;
        let mut matches = Vec::new();
        for e in &entries {
            if patch_name_matches(&e.name, name) {
                return Ok(e.name.clone());
            }
        }
        if Path::new(name).components().count() == 1 {
            for e in entries {
                if Path::new(&e.name)
                    .file_name()
                    .and_then(|file| file.to_str())
                    .map(|basename| patch_name_matches(basename, name))
                    .unwrap_or(false)
                {
                    matches.push(e.name);
                }
            }
        }
        if matches.len() == 1 {
            Ok(matches.remove(0))
        } else if matches.len() > 1 {
            bail!("Patch {name} is ambiguous")
        } else {
            bail!("Patch {name} is not in series")
        }
    }

    pub fn patch_path(&self, patch: &str) -> PathBuf {
        self.patches_dir.join(patch)
    }

    /// A patch name as quilt prints it (`print_patch`).
    pub fn display_patch(&self, patch: &str) -> String {
        match &self.patch_prefix {
            Some(prefix) => format!("{prefix}{patch}"),
            None => patch.to_string(),
        }
    }

    pub fn backup_dir(&self, patch: &str) -> PathBuf {
        self.pc_dir.join(patch)
    }

    pub fn rel_from_cwd(&self, path: &Path) -> PathBuf {
        let rel = if path.is_absolute() {
            path.strip_prefix(&self.root).unwrap_or(path).to_path_buf()
        } else {
            self.subdir.join(path)
        };
        rel.components()
            .filter_map(|component| match component {
                Component::CurDir => None,
                other => Some(other.as_os_str().to_owned()),
            })
            .collect()
    }

    pub fn tracked_files(&self, patch: &str) -> Result<Vec<PathBuf>> {
        let dir = self.backup_dir(patch);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut files = Vec::new();
        for entry in WalkDir::new(&dir) {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    if let Some(path) = err.path() {
                        let rel = path.strip_prefix(&dir).unwrap_or(path);
                        let reason = err
                            .io_error()
                            .map(|io| {
                                if io.kind() == std::io::ErrorKind::PermissionDenied {
                                    "Permission denied".to_string()
                                } else {
                                    io.to_string()
                                }
                            })
                            .unwrap_or_else(|| err.to_string());
                        bail!("find: `./{}': {}", rel.display(), reason);
                    }
                    return Err(err.into());
                }
            };
            if entry.file_type().is_file() && entry.file_name() != ".timestamp" {
                files.push(entry.path().strip_prefix(&dir)?.to_path_buf());
            }
        }
        sort_tracked(&mut files);
        Ok(files)
    }
}

/// quilt's order for a patch's files: shallower paths first, then by name.
pub(crate) fn sort_tracked(files: &mut [PathBuf]) {
    files.sort_by(|a, b| {
        a.components()
            .count()
            .cmp(&b.components().count())
            .then_with(|| a.cmp(b))
    });
}

/// Whether a series file line is the entry for `patch`.
fn series_line_is(line: &str, patch: &str) -> bool {
    line.strip_prefix(patch)
        .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '\t']))
}

fn patch_name_matches(candidate: &str, name: &str) -> bool {
    candidate == name
        || candidate == format!("{name}.patch")
        || candidate == format!("{name}.diff")
        || candidate.starts_with(&format!("{name}.patch."))
        || candidate.starts_with(&format!("{name}.diff."))
}

pub fn parse_series_line(line: &str) -> Option<PatchEntry> {
    let trimmed = line.trim_start();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let mut parts = trimmed.split_whitespace();
    let name = parts.next()?.to_string();
    let mut args = Vec::new();
    for p in parts {
        if p.starts_with('#') {
            break;
        }
        args.push(p.to_string());
    }
    Some(PatchEntry { name, args })
}

fn read_trimmed(path: PathBuf) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn write_if_missing(path: PathBuf, value: &str) -> Result<()> {
    if !path.exists() {
        fs::write(path, format!("{value}\n"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_series_line;

    #[test]
    fn parses_series_lines_with_options_and_comments() {
        let entry = parse_series_line(" fix/foo.diff -p0 # comment").unwrap();
        assert_eq!(entry.name, "fix/foo.diff");
        assert_eq!(entry.args, ["-p0"]);
    }

    #[test]
    fn ignores_comments_and_blank_lines() {
        assert!(parse_series_line("# patch.diff").is_none());
        assert!(parse_series_line("   ").is_none());
    }
}
