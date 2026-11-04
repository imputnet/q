//! GNU patch-compatible patch parsing and application.

use std::fmt;
use std::path::{Path, PathBuf};

mod apply;
mod diff_format;
mod fs_util;
mod hunk;
mod merge;
mod options;
mod parse;
mod paths;
mod reject;
mod timestamp;

pub use apply::apply_patch;
pub use options::{ApplyOptionParseError, ParsedApplyOptions, parse_apply_options};
pub use parse::parse_patch;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchSet {
    pub files: Vec<FilePatch>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePatch {
    pub old_path: PatchPath,
    pub new_path: PatchPath,
    pub old_label: Option<String>,
    pub new_label: Option<String>,
    pub index: Option<String>,
    pub hunks: Vec<Hunk>,
    pub old_file_mode: Option<u32>,
    pub new_file_mode: Option<u32>,
    pub git_paths: bool,
    pub git_operation: Option<GitOperation>,
    pub format: PatchFormat,
}

impl FilePatch {
    /// A file patch whose paths are not known yet (e.g. a bare hunk fragment).
    pub(crate) fn headerless(format: PatchFormat, hunks: Vec<Hunk>) -> Self {
        Self {
            old_path: PatchPath::File(String::new()),
            new_path: PatchPath::File(String::new()),
            old_label: None,
            new_label: None,
            index: None,
            hunks,
            old_file_mode: None,
            new_file_mode: None,
            git_paths: false,
            git_operation: None,
            format,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitOperation {
    Copy,
    Rename,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchFormat {
    Unified,
    Context,
    Normal,
    Ed,
}

impl PatchFormat {
    /// Parses a `--reject-format` value.
    pub fn from_reject_format(name: &str) -> Option<Self> {
        match name {
            "unified" => Some(Self::Unified),
            "context" => Some(Self::Context),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatchPath {
    File(String),
    DevNull,
}

impl PatchPath {
    pub(crate) fn as_file(&self) -> Option<&str> {
        match self {
            Self::File(path) => Some(path),
            Self::DevNull => None,
        }
    }

    /// The path as written in a diff header.
    pub(crate) fn label(&self) -> &str {
        self.as_file().unwrap_or("/dev/null")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub section: Option<String>,
    pub lines: Vec<HunkLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HunkLine {
    Context(String),
    Remove(String),
    Add(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyOptions {
    pub strip: usize,
    pub reverse: bool,
    pub force: bool,
    pub batch: bool,
    pub read_only: ReadOnlyBehavior,
    pub dry_run: bool,
    pub target_file: Option<PathBuf>,
    pub remove_empty: bool,
    pub reject_format: Option<PatchFormat>,
    pub ifdef_symbol: Option<String>,
    pub merge: MergeMode,
    pub posix: bool,
    pub set_time: bool,
    pub set_utc: bool,
    pub follow_symlinks: bool,
    pub ignore_whitespace: bool,
    pub max_fuzz: usize,
}

impl Default for ApplyOptions {
    fn default() -> Self {
        Self {
            strip: 0,
            reverse: false,
            force: false,
            batch: false,
            read_only: ReadOnlyBehavior::Ignore,
            dry_run: false,
            target_file: None,
            remove_empty: false,
            reject_format: None,
            ifdef_symbol: None,
            merge: MergeMode::None,
            posix: false,
            set_time: false,
            set_utc: false,
            follow_symlinks: false,
            ignore_whitespace: false,
            max_fuzz: 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeMode {
    None,
    Merge,
    Diff3,
}

impl MergeMode {
    /// Parses a `--merge=` value.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "merge" => Some(Self::Merge),
            "diff3" => Some(Self::Diff3),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadOnlyBehavior {
    Ignore,
    Warn,
    Fail,
}

impl ReadOnlyBehavior {
    /// Parses a `--read-only=` value.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "ignore" => Some(Self::Ignore),
            "warn" => Some(Self::Warn),
            "fail" => Some(Self::Fail),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyReport {
    pub files: Vec<FileReport>,
}

impl ApplyReport {
    pub fn failed_hunks(&self) -> usize {
        self.files.iter().map(FileReport::failed_hunks).sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileReport {
    pub path: PathBuf,
    pub operation: ReportOperation,
    pub backup_path: Option<PathBuf>,
    pub hunks_applied: usize,
    pub hunk_reports: Vec<HunkReport>,
    pub deleted: bool,
    pub delete_conflict: bool,
    pub create_conflict: bool,
    pub delete_missing: bool,
    pub empty_conflict: bool,
    pub read_only: bool,
    pub read_only_refused: bool,
    pub not_regular_refused: bool,
    pub invalid_path_refused: bool,
    pub skipped: bool,
    pub ignored_hunks: usize,
    pub assumed_reverse: bool,
    pub created: bool,
    pub time_mismatch: bool,
    pub symbolic_link: bool,
}

impl FileReport {
    /// A plain patch report for `path` with every flag cleared.
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            operation: ReportOperation::Patch,
            backup_path: None,
            hunks_applied: 0,
            hunk_reports: Vec::new(),
            deleted: false,
            delete_conflict: false,
            create_conflict: false,
            delete_missing: false,
            empty_conflict: false,
            read_only: false,
            read_only_refused: false,
            not_regular_refused: false,
            invalid_path_refused: false,
            skipped: false,
            ignored_hunks: 0,
            assumed_reverse: false,
            created: false,
            time_mismatch: false,
            symbolic_link: false,
        }
    }

    pub fn failed_hunks(&self) -> usize {
        self.hunk_reports
            .iter()
            .filter(|hunk| hunk.status.is_failure())
            .count()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReportOperation {
    Patch,
    Copy { from: PathBuf },
    Rename { from: PathBuf, already_done: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HunkReport {
    pub hunk: usize,
    pub status: HunkStatus,
    pub reject: Option<String>,
    pub different_line_endings: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HunkStatus {
    Applied {
        requested_line: usize,
        applied_line: usize,
        fuzz: usize,
    },
    Merged {
        requested_line: usize,
        ranges: Vec<(usize, usize)>,
    },
    AlreadyApplied {
        requested_line: usize,
        applied_line: usize,
        end_line: usize,
    },
    Conflict {
        requested_line: usize,
        start_line: usize,
        end_line: usize,
        merged_ranges: Vec<(usize, usize)>,
    },
    Failed {
        requested_line: usize,
    },
}

impl HunkStatus {
    pub(crate) fn is_failure(&self) -> bool {
        matches!(self, Self::Failed { .. } | Self::Conflict { .. })
    }
}

#[derive(Debug)]
pub enum Error {
    Parse {
        line: usize,
        message: String,
    },
    StripPath {
        path: String,
        strip: usize,
    },
    DangerousPath {
        path: String,
    },
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    UnsupportedGitBinary {
        path: PathBuf,
    },
    HunkFailed {
        path: PathBuf,
        hunk: usize,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Parse { line, message } => write!(f, "parse error at line {line}: {message}"),
            Error::StripPath { path, strip } => {
                write!(f, "cannot strip {strip} component(s) from path {path:?}")
            }
            Error::DangerousPath { path } => write!(f, "potentially dangerous file name {path}"),
            Error::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Error::UnsupportedGitBinary { path } => write!(
                f,
                "File {}: git binary diffs are not supported.",
                path.display()
            ),
            Error::HunkFailed { path, hunk } => {
                write!(f, "{}: hunk #{hunk} FAILED", path.display())
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Builds an `Error::Io` mapper for `path`, for use with `map_err`.
pub(crate) fn io_error(path: &Path) -> impl FnOnce(std::io::Error) -> Error + '_ {
    move |source| Error::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// The paths `apply_patch` would write to, in patch order.
pub fn affected_paths(
    root: impl AsRef<Path>,
    patch: &PatchSet,
    options: &ApplyOptions,
) -> Result<Vec<PathBuf>> {
    let root = root.as_ref();
    let allow_absolute = paths::root_allows_absolute_patch_paths(root);
    patch
        .files
        .iter()
        .map(|file_patch| paths::selected_file_target(root, file_patch, options, allow_absolute))
        .collect()
}
