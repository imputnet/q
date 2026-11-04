//! Side effects of the `p2` binary beyond patching: progress messages,
//! backup files and reject files.

use std::collections::HashSet;
use std::env;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::process::exit;

use p2::{ApplyReport, FileReport, HunkStatus, ReportOperation};

/// Looks up `value` in `table`, accepting any prefix that names one value.
fn lookup_abbreviation<T: Copy + PartialEq>(table: &[(&str, T)], value: &str) -> Option<T> {
    if let Some(&(_, found)) = table.iter().find(|(name, _)| *name == value) {
        return Some(found);
    }
    let mut matches = table
        .iter()
        .filter(|(name, _)| name.starts_with(value))
        .map(|&(_, found)| found);
    let found = matches.next()?;
    matches.all(|other| other == found).then_some(found)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum QuotingStyle {
    Literal,
    Shell,
    ShellAlways,
    C,
    Escape,
}

impl QuotingStyle {
    pub(crate) fn from_name(value: &str) -> Option<Self> {
        lookup_abbreviation(
            &[
                ("literal", Self::Literal),
                ("shell", Self::Shell),
                ("shell-always", Self::ShellAlways),
                ("shell-escape", Self::Shell),
                ("shell-escape-always", Self::ShellAlways),
                ("c", Self::C),
                ("c-maybe", Self::Literal),
                ("escape", Self::Escape),
                ("locale", Self::ShellAlways),
                ("clocale", Self::C),
            ],
            value,
        )
    }

    fn quote(self, path: &Path) -> String {
        let text = path.display().to_string();
        match self {
            Self::Literal | Self::Escape => text,
            Self::C => c_quote(&text),
            Self::Shell
                if text.bytes().all(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'.' | b'_' | b'-')
                }) =>
            {
                text
            }
            Self::Shell | Self::ShellAlways => shell_quote(&text),
        }
    }
}

fn shell_quote(text: &str) -> String {
    if !text.contains('\'') {
        return format!("'{text}'");
    }
    let escaped = text
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`");
    format!("\"{escaped}\"")
}

fn c_quote(text: &str) -> String {
    let mut quoted = String::from("\"");
    for ch in text.chars() {
        match ch {
            '\\' => quoted.push_str("\\\\"),
            '"' => quoted.push_str("\\\""),
            '\n' => quoted.push_str("\\n"),
            '\r' => quoted.push_str("\\r"),
            '\t' => quoted.push_str("\\t"),
            _ => quoted.push(ch),
        }
    }
    quoted.push('"');
    quoted
}

pub(crate) fn print_file_action(action: &str, file: &FileReport, quoting_style: QuotingStyle) {
    let kind = if file.symbolic_link {
        "symbolic link"
    } else {
        "file"
    };
    print!("{action} {kind} {}", quoting_style.quote(&file.path));
    match &file.operation {
        ReportOperation::Patch => {}
        ReportOperation::Copy { from } => print!(" (copied from {})", quoting_style.quote(from)),
        ReportOperation::Rename { from, already_done } => {
            let already = if *already_done { "already " } else { "" };
            print!(" ({already}renamed from {})", quoting_style.quote(from));
        }
    }
    println!();
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BackupStyle {
    Simple,
    Numbered,
    Existing,
}

impl BackupStyle {
    fn from_name(value: &str) -> Option<Self> {
        lookup_abbreviation(
            &[
                ("none", Self::Numbered),
                ("off", Self::Numbered),
                ("simple", Self::Simple),
                ("never", Self::Simple),
                ("existing", Self::Existing),
                ("nil", Self::Existing),
                ("numbered", Self::Numbered),
                ("t", Self::Numbered),
            ],
            value,
        )
    }

    fn from_env() -> Option<Self> {
        env::var("PATCH_VERSION_CONTROL")
            .or_else(|_| env::var("VERSION_CONTROL"))
            .ok()
            .and_then(|value| Self::from_name(&value))
    }
}

pub(crate) fn parse_backup_style(program_name: &str, value: &str) -> BackupStyle {
    BackupStyle::from_name(value).unwrap_or_else(|| {
        eprintln!("{program_name}: invalid version control style {value:?}");
        exit(2);
    })
}

#[derive(Default)]
pub(crate) struct BackupConfig {
    /// Back up every patched file (`-b`, `-B`, `-Y`, `-z`).
    pub(crate) enabled: bool,
    /// Back up files whose hunks did not apply exactly.
    pub(crate) if_mismatch: bool,
    pub(crate) prefix: PathBuf,
    pub(crate) basename_prefix: PathBuf,
    pub(crate) suffix: Option<String>,
    pub(crate) style: Option<BackupStyle>,
}

impl BackupConfig {
    fn backup_path(&self, root: &Path, path: &Path) -> PathBuf {
        if self.prefix.as_os_str().is_empty()
            && self.basename_prefix.as_os_str().is_empty()
            && self.suffix.is_none()
        {
            return root.join(self.default_backup_path(root, path));
        }

        let mut backup = if self.basename_prefix.as_os_str().is_empty() {
            path.to_path_buf()
        } else {
            match path.parent() {
                Some(parent) => parent.join(prefixed_path(
                    &self.basename_prefix,
                    Path::new(path.file_name().unwrap_or_default()),
                )),
                None => prefixed_path(&self.basename_prefix, path),
            }
        };
        if !self.prefix.as_os_str().is_empty() {
            backup = prefixed_path(&self.prefix, &backup);
        }
        backup
            .as_mut_os_string()
            .push(self.suffix.as_deref().unwrap_or(""));
        root.join(backup)
    }

    fn default_backup_path(&self, root: &Path, path: &Path) -> PathBuf {
        let numbered = match self.style.or_else(BackupStyle::from_env) {
            Some(BackupStyle::Numbered) => true,
            Some(BackupStyle::Existing) => {
                (1..=1000).any(|number| root.join(numbered_backup_candidate(path, number)).exists())
            }
            Some(BackupStyle::Simple) | None => false,
        };
        if numbered {
            (1..)
                .map(|number| numbered_backup_candidate(path, number))
                .find(|candidate| !root.join(candidate).exists())
                .expect("numbered backup search is unbounded")
        } else {
            let mut backup = path.to_path_buf();
            backup
                .as_mut_os_string()
                .push(env::var("SIMPLE_BACKUP_SUFFIX").unwrap_or_else(|_| ".orig".to_string()));
            backup
        }
    }

    fn create_empty_backup(&self, root: &Path, path: &Path) {
        let destination = self.backup_path(root, path);
        create_parent_or_exit(&destination);
        fs::write(&destination, "").unwrap_or_else(|error| exit_io(&destination, error));
    }
}

/// Joins `prefix` and `path` as a directory if `prefix` looks like one,
/// otherwise concatenates them (`-B old_` makes `old_file`).
fn prefixed_path(prefix: &Path, path: &Path) -> PathBuf {
    if prefix.components().count() > 1 || prefix.to_string_lossy().ends_with('/') {
        prefix.join(path)
    } else {
        let mut raw = prefix.as_os_str().to_os_string();
        raw.push(path.as_os_str());
        PathBuf::from(raw)
    }
}

fn numbered_backup_candidate(path: &Path, number: usize) -> PathBuf {
    let mut backup = path.to_path_buf();
    backup.as_mut_os_string().push(format!(".~{number}~"));
    backup
}

pub(crate) fn exit_io(path: &Path, error: std::io::Error) -> ! {
    eprintln!("p2: {}: {error}", path.display());
    exit(1);
}

fn create_parent_or_exit(path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|error| exit_io(parent, error));
    }
}

/// Backs up the files a dry run says will be patched, before patching them.
pub(crate) fn create_backups(root: &Path, report: &ApplyReport, config: &BackupConfig) {
    let mut seen = HashSet::new();
    for file in &report.files {
        if !config.enabled && (!config.if_mismatch || !file_has_mismatch(file)) {
            continue;
        }
        let path = file.backup_path.as_ref().unwrap_or(&file.path);
        if !seen.insert(path.clone()) {
            continue;
        }
        let source = root.join(path);
        let metadata = match fs::symlink_metadata(&source) {
            Ok(metadata) if metadata.file_type().is_symlink() || source.exists() => metadata,
            Ok(_) => {
                if config.enabled && file.created {
                    config.create_empty_backup(root, &file.path);
                }
                continue;
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                if config.enabled && file.created {
                    config.create_empty_backup(root, &file.path);
                }
                continue;
            }
            Err(error) => exit_io(&source, error),
        };
        let destination = config.backup_path(root, path);
        create_parent_or_exit(&destination);
        copy_backup_entry(&source, &destination, &metadata)
            .unwrap_or_else(|error| exit_io(&destination, error));
        if config.enabled && file.created && file.backup_path.as_ref() != Some(&file.path) {
            config.create_empty_backup(root, &file.path);
        }
    }
}

fn copy_backup_entry(
    source: &Path,
    destination: &Path,
    metadata: &fs::Metadata,
) -> std::io::Result<()> {
    match fs::remove_file(destination) {
        Err(error) if error.kind() != ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    if metadata.file_type().is_symlink() {
        copy_symlink(source, destination)
    } else {
        fs::copy(source, destination).map(|_| ())
    }
}

#[cfg(unix)]
fn copy_symlink(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(fs::read_link(source)?, destination)
}

#[cfg(not(unix))]
fn copy_symlink(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::copy(source, destination).map(|_| ())
}

/// Whether any hunk applied somewhere other than exactly as written.
fn file_has_mismatch(file: &FileReport) -> bool {
    file.hunk_reports.iter().any(|hunk| match hunk.status {
        HunkStatus::Applied {
            requested_line,
            applied_line,
            fuzz,
        } => requested_line != applied_line || fuzz > 0,
        HunkStatus::Merged { .. }
        | HunkStatus::AlreadyApplied { .. }
        | HunkStatus::Conflict { .. } => true,
        HunkStatus::Failed { .. } => false,
    })
}

/// Reports failed hunks and saves them to the reject file.
pub(crate) fn report_rejects(
    root: &Path,
    file: &FileReport,
    reject_file: Option<&Path>,
    dry_run: bool,
    reject_files_seen: &mut HashSet<PathBuf>,
) {
    let failed = file
        .hunk_reports
        .iter()
        .filter(|hunk| hunk.reject.is_some())
        .count();
    let total = file.hunk_reports.len();
    let summary = format!("{failed} out of {total} hunk{} FAILED", plural(total));
    save_rejects(
        root,
        file,
        reject_file,
        dry_run,
        reject_files_seen,
        failed,
        &summary,
    );
}

/// Reports hunks of a file that was not patched at all and saves them to
/// the reject file.
pub(crate) fn report_ignored_rejects(
    root: &Path,
    file: &FileReport,
    reject_file: Option<&Path>,
    dry_run: bool,
    reject_files_seen: &mut HashSet<PathBuf>,
) {
    let ignored = file.ignored_hunks;
    let summary = format!("{ignored} out of {ignored} hunk{} ignored", plural(ignored));
    save_rejects(
        root,
        file,
        reject_file,
        dry_run,
        reject_files_seen,
        ignored,
        &summary,
    );
}

fn save_rejects(
    root: &Path,
    file: &FileReport,
    reject_file: Option<&Path>,
    dry_run: bool,
    reject_files_seen: &mut HashSet<PathBuf>,
    count: usize,
    summary: &str,
) {
    if count == 0 {
        return;
    }
    if dry_run {
        println!("{summary}");
        return;
    }

    let reject = reject_file
        .map(PathBuf::from)
        .unwrap_or_else(|| reject_path(&file.path));
    let full_reject = root.join(&reject);
    if let Some(parent) = full_reject.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let body = file
        .hunk_reports
        .iter()
        .filter_map(|hunk| hunk.reject.as_deref())
        .collect::<String>();
    // Rejects for several files can share one `-r` file; append after the first.
    if reject_files_seen.insert(reject.clone()) {
        let _ = fs::remove_file(&full_reject);
        let _ = fs::write(&full_reject, body);
    } else if let Ok(mut out) = fs::OpenOptions::new().append(true).open(&full_reject) {
        let _ = out.write_all(body.as_bytes());
    }
    println!("{summary} -- saving rejects to file {}", reject.display());
}

pub(crate) fn plural(count: usize) -> &'static str {
    if count == 1 { "" } else { "s" }
}

fn reject_path(path: &Path) -> PathBuf {
    let mut raw = path.as_os_str().to_os_string();
    raw.push(".rej");
    PathBuf::from(raw)
}
