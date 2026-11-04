//! Parsing of unified, context, normal and ed diffs, including git extended
//! headers.

use std::path::PathBuf;

use crate::diff_format::{
    CONTEXT_HUNK_SEPARATOR, CONTEXT_NEW_RANGE_PREFIX, CONTEXT_NEW_RANGE_SUFFIX,
    CONTEXT_OLD_RANGE_PREFIX, CONTEXT_OLD_RANGE_SUFFIX,
};
use crate::{
    Error, FilePatch, GitOperation, Hunk, HunkLine, PatchFormat, PatchPath, PatchSet, Result,
};

fn parse_error(line: usize, message: impl Into<String>) -> Error {
    Error::Parse {
        line,
        message: message.into(),
    }
}

#[derive(Clone, Copy)]
enum GitFileMode {
    New { mode: u32 },
    Deleted { mode: u32 },
    Change { old_mode: u32, new_mode: u32 },
}

impl GitFileMode {
    /// The (old, new) file modes.
    fn modes(self) -> (Option<u32>, Option<u32>) {
        match self {
            Self::New { mode } => (None, Some(mode)),
            Self::Deleted { mode } => (Some(mode), None),
            Self::Change { old_mode, new_mode } => (Some(old_mode), Some(new_mode)),
        }
    }
}

/// What a `diff` line and the extended headers after it said about the next
/// file, until its hunks arrive.
#[derive(Default)]
struct PendingHeader {
    /// Paths from the `diff` line, without `a/`/`b/` prefixes.
    paths: Option<(String, String)>,
    /// Whether `paths` came from `diff --git`.
    is_git: bool,
    file_mode: Option<GitFileMode>,
    old_mode: Option<u32>,
    operation: Option<GitOperation>,
}

impl PendingHeader {
    fn git_paths(&self) -> Option<&(String, String)> {
        self.paths.as_ref().filter(|_| self.is_git)
    }

    /// Emits a git file patch that has no hunks (a mode change, empty
    /// create/delete, or pure copy/rename) for a header that is being
    /// replaced or that ends the input.
    fn finish_metadata_only(self, files: &mut Vec<FilePatch>) {
        let Some((old_path, new_path)) = self.paths.filter(|_| self.is_git) else {
            return;
        };
        if let Some(mode) = self.file_mode {
            let (old_path, new_path) = match mode {
                GitFileMode::New { .. } => (PatchPath::DevNull, PatchPath::File(new_path)),
                GitFileMode::Deleted { .. } => (PatchPath::File(old_path), PatchPath::DevNull),
                GitFileMode::Change { .. } => {
                    (PatchPath::File(old_path), PatchPath::File(new_path))
                }
            };
            files.push(git_file_patch(
                old_path,
                new_path,
                Some(mode),
                None,
                Vec::new(),
            ));
        } else if let Some(operation) = self.operation {
            files.push(git_file_patch(
                PatchPath::File(old_path),
                PatchPath::File(new_path),
                None,
                Some(operation),
                Vec::new(),
            ));
        }
    }
}

/// An `Index:` line waiting to be attached to the next file patch.
#[derive(Default)]
struct PendingIndex {
    line: Option<String>,
    path: Option<String>,
}

impl PendingIndex {
    /// Attaches the index line to `file_patch`. With `fill_paths`, the
    /// `Index:` path also stands in for header paths the patch lacks.
    fn attach(&mut self, file_patch: &mut FilePatch, fill_paths: bool) {
        if fill_paths && let Some(path) = &self.path {
            for patch_path in [&mut file_patch.old_path, &mut file_patch.new_path] {
                if matches!(patch_path, PatchPath::File(value) if value.is_empty()) {
                    *patch_path = PatchPath::File(path.clone());
                }
            }
        }
        file_patch.index = self.line.take();
        self.path = None;
    }
}

fn git_file_patch(
    old_path: PatchPath,
    new_path: PatchPath,
    mode: Option<GitFileMode>,
    git_operation: Option<GitOperation>,
    hunks: Vec<Hunk>,
) -> FilePatch {
    let (old_file_mode, new_file_mode) = mode.map_or((None, None), GitFileMode::modes);
    FilePatch {
        old_path,
        new_path,
        old_label: None,
        new_label: None,
        index: None,
        hunks,
        old_file_mode,
        new_file_mode,
        git_paths: true,
        git_operation,
        format: PatchFormat::Unified,
    }
}

pub fn parse_patch(input: &str) -> Result<PatchSet> {
    let lines: Vec<&str> = input.split_inclusive('\n').collect();
    let mut files = Vec::with_capacity(estimated_file_patch_count(&lines));
    let mut header = PendingHeader::default();
    let mut index = PendingIndex::default();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let line_number = i + 1;

        if line.starts_with("Index: ") {
            index.line = Some(line.to_string());
            index.path = parse_index_path(line, line_number)?;
            i += 1;
            continue;
        }
        if line.starts_with("diff ") {
            let (paths, is_git) = if line.starts_with("diff --git ") {
                (parse_git_header(line, line_number)?, true)
            } else {
                (parse_diff_header(line, line_number)?, false)
            };
            std::mem::take(&mut header).finish_metadata_only(&mut files);
            header.paths = Some(paths);
            header.is_git = is_git;
            i += 1;
            continue;
        }
        if let Some(mode) = line.strip_prefix("new file mode ") {
            header.file_mode = Some(GitFileMode::New {
                mode: parse_git_mode(mode, line_number)?,
            });
            i += 1;
            continue;
        }
        if let Some(mode) = line.strip_prefix("old mode ") {
            header.old_mode = Some(parse_git_mode(mode, line_number)?);
            i += 1;
            continue;
        }
        if let Some(mode) = line.strip_prefix("new mode ") {
            let new_mode = parse_git_mode(mode, line_number)?;
            header.file_mode = Some(GitFileMode::Change {
                old_mode: header.old_mode.unwrap_or(new_mode),
                new_mode,
            });
            i += 1;
            continue;
        }
        if let Some(mode) = line.strip_prefix("deleted file mode ") {
            header.file_mode = Some(GitFileMode::Deleted {
                mode: parse_git_mode(mode, line_number)?,
            });
            i += 1;
            continue;
        }
        if line.starts_with("index ") {
            if let Some(mode) = parse_git_index_symlink_mode(line) {
                header.file_mode = Some(GitFileMode::Change {
                    old_mode: mode,
                    new_mode: mode,
                });
            }
            i += 1;
            continue;
        }
        if line.starts_with("GIT binary patch") {
            let Some((_, new_path)) = header.git_paths() else {
                return Err(parse_error(
                    line_number,
                    "git binary patch without git header",
                ));
            };
            return Err(Error::UnsupportedGitBinary {
                path: PathBuf::from(new_path),
            });
        }
        if line.starts_with("copy from ") {
            header.operation = Some(GitOperation::Copy);
            i += 1;
            continue;
        }
        if line.starts_with("rename from ") {
            header.operation = Some(GitOperation::Rename);
            i += 1;
            continue;
        }
        if line.starts_with("copy to ") || line.starts_with("rename to ") {
            i += 1;
            continue;
        }

        // Formats without a usable header; the `Index:` path may fill in.
        let headerless = if looks_like_context_file_header(&lines, i) {
            Some((parse_context_file(&lines, i)?, false))
        } else if line.starts_with(CONTEXT_HUNK_SEPARATOR) {
            let (hunks, next) = parse_context_hunks(&lines, i)?;
            Some((
                (FilePatch::headerless(PatchFormat::Context, hunks), next),
                true,
            ))
        } else if let Some(parsed) = parse_normal_file(&lines, i)? {
            Some((parsed, true))
        } else if let Some((hunks, next)) = parse_ed_hunks(&lines, i)? {
            Some(((FilePatch::headerless(PatchFormat::Ed, hunks), next), true))
        } else {
            None
        };
        if let Some(((mut file_patch, next), fill_paths)) = headerless {
            index.attach(&mut file_patch, fill_paths);
            files.push(file_patch);
            i = next;
            continue;
        }

        if line.starts_with("@@ ") {
            let pending = std::mem::take(&mut header);
            let (hunks, next) = parse_unified_hunks(&lines, i)?;
            let mut file_patch = match pending.paths.filter(|_| pending.is_git) {
                Some((old_path, new_path)) => git_file_patch(
                    PatchPath::File(old_path),
                    PatchPath::File(new_path),
                    pending.file_mode,
                    pending.operation,
                    hunks,
                ),
                None => FilePatch::headerless(PatchFormat::Unified, hunks),
            };
            index.attach(&mut file_patch, true);
            files.push(file_patch);
            i = next;
            continue;
        }

        if !line.starts_with("--- ")
            || !lines
                .get(i + 1)
                .is_some_and(|line| line.starts_with("+++ "))
        {
            i += 1;
            continue;
        }

        let (old_path, old_label) = parse_header(line, "--- ", line_number)?;
        let (new_path, new_label) = parse_header(lines[i + 1], "+++ ", line_number + 1)?;
        i += 2;
        let pending = std::mem::take(&mut header);
        let git_paths = pending.paths.is_some();
        // The `diff` line's paths take precedence over the ---/+++ paths,
        // except that /dev/null still marks a creation or deletion.
        let (old_path, new_path) = match pending.paths {
            Some((git_old, git_new)) => (
                old_path
                    .as_file()
                    .map_or(PatchPath::DevNull, |_| PatchPath::File(git_old)),
                new_path
                    .as_file()
                    .map_or(PatchPath::DevNull, |_| PatchPath::File(git_new)),
            ),
            None => (old_path, new_path),
        };
        let (old_file_mode, new_file_mode) =
            pending.file_mode.map_or((None, None), GitFileMode::modes);

        let (mut hunks, next) = parse_unified_hunks(&lines, i)?;
        let mut format = PatchFormat::Unified;
        if hunks.is_empty()
            && let Some((ed_hunks, ed_next)) = parse_ed_hunks(&lines, i)?
        {
            hunks = ed_hunks;
            format = PatchFormat::Ed;
            i = ed_next;
        } else {
            i = next;
        }

        let mut file_patch = FilePatch {
            old_path,
            new_path,
            old_label,
            new_label,
            index: None,
            hunks,
            old_file_mode,
            new_file_mode,
            git_paths,
            git_operation: pending.operation,
            format,
        };
        index.attach(&mut file_patch, false);
        // Like GNU patch, a plain header pair with nothing after it patches
        // nothing; a git header may still carry a mode change or operation.
        if file_patch.hunks.is_empty() && !file_patch.git_paths {
            continue;
        }
        files.push(file_patch);
    }

    header.finish_metadata_only(&mut files);

    if files.is_empty() {
        return Err(parse_error(1, "no unified diff file headers found"));
    }
    Ok(PatchSet { files })
}

fn estimated_file_patch_count(lines: &[&str]) -> usize {
    let count = lines
        .iter()
        .filter(|line| line.starts_with("--- ") || line.starts_with("diff --git "))
        .count();
    count.max(1)
}

/// Parses a `--- path[\tlabel]` style file header line.
fn parse_header(
    line: &str,
    prefix: &str,
    line_number: usize,
) -> Result<(PatchPath, Option<String>)> {
    let Some(rest) = line.strip_prefix(prefix) else {
        return Err(parse_error(
            line_number,
            format!("expected {prefix:?} header"),
        ));
    };
    let rest = rest.trim_end_matches('\n');
    let (path, path_end) = parse_header_path(rest, line_number)?;
    let label = (path_end < rest.len()).then(|| rest[path_end..].to_string());

    if path.is_empty() {
        return Err(parse_error(line_number, "missing file path"));
    }
    if path == "/dev/null" {
        Ok((PatchPath::DevNull, label))
    } else {
        Ok((PatchPath::File(path), label))
    }
}

/// Parses a possibly quoted path, returning it and the byte offset just past it.
fn parse_header_path(rest: &str, line_number: usize) -> Result<(String, usize)> {
    if rest.starts_with('"') {
        return parse_quoted_path(rest, line_number);
    }
    let path_end = rest
        .find('\t')
        .unwrap_or_else(|| rest.find(char::is_whitespace).unwrap_or(rest.len()));
    Ok((rest[..path_end].trim_end().to_string(), path_end))
}

fn parse_quoted_path(rest: &str, line_number: usize) -> Result<(String, usize)> {
    // GNU treats malformed quoted header names as unusable, not as literal
    // backslash paths. "/" flows through target selection as a strip failure.
    let invalid = || Ok(("/".to_string(), rest.len()));
    let unterminated = || Err(parse_error(line_number, "unterminated quoted path"));

    let mut path = String::new();
    let mut chars = rest.char_indices().skip(1);
    while let Some((index, ch)) = chars.next() {
        match ch {
            '"' => return Ok((path, index + ch.len_utf8())),
            '\\' => {
                let Some((_, escaped)) = chars.next() else {
                    return unterminated();
                };
                match escaped {
                    'a' => path.push('\x07'),
                    'b' => path.push('\x08'),
                    'f' => path.push('\x0c'),
                    'n' => path.push('\n'),
                    'r' => path.push('\r'),
                    't' => path.push('\t'),
                    'v' => path.push('\x0b'),
                    '\\' => path.push('\\'),
                    '"' => path.push('"'),
                    '0'..='3' => {
                        let mut value = escaped.to_digit(8).unwrap();
                        for _ in 0..2 {
                            match chars.next() {
                                Some((_, digit @ '0'..='7')) => {
                                    value = (value << 3) | digit.to_digit(8).unwrap();
                                }
                                _ => return invalid(),
                            }
                        }
                        if value == 0 {
                            return invalid();
                        }
                        path.extend(char::from_u32(value));
                    }
                    _ => return invalid(),
                }
            }
            _ => path.push(ch),
        }
    }
    unterminated()
}

fn context_section(line: &str) -> Option<String> {
    line.trim_end_matches('\n')
        .strip_prefix(CONTEXT_HUNK_SEPARATOR)
        .filter(|section| !section.is_empty())
        .map(str::to_string)
}

/// Parses `@@ -old +new @@[ section]` into a hunk with no lines yet.
fn parse_hunk_header(line: &str, line_number: usize) -> Result<Hunk> {
    let Some(rest) = line.strip_prefix("@@ -") else {
        return Err(parse_error(line_number, "expected unified hunk header"));
    };
    let Some((old_range, rest)) = rest.split_once(" +") else {
        return Err(parse_error(line_number, "missing new range in hunk header"));
    };
    let Some((new_range, suffix)) = rest.split_once(" @@") else {
        return Err(parse_error(line_number, "unterminated hunk header"));
    };
    let (old_start, old_count) = parse_range(old_range, line_number)?;
    let (new_start, new_count) = parse_range(new_range, line_number)?;
    let suffix = suffix.trim_end_matches('\n');
    Ok(Hunk {
        old_start,
        old_count,
        new_start,
        new_count,
        section: (!suffix.is_empty()).then(|| suffix.to_string()),
        lines: Vec::with_capacity(old_count.max(new_count)),
    })
}

/// Parses a unified `start[,count]` range.
fn parse_range(range: &str, line_number: usize) -> Result<(usize, usize)> {
    match range.split_once(',') {
        Some((start, count)) => Ok((
            parse_usize(start, line_number)?,
            parse_usize(count, line_number)?,
        )),
        None => Ok((parse_usize(range, line_number)?, 1)),
    }
}

fn parse_usize(value: &str, line_number: usize) -> Result<usize> {
    value
        .parse()
        .map_err(|_| parse_error(line_number, format!("invalid number {value:?}")))
}

fn starts_file_header(line: &str) -> bool {
    line.starts_with("--- ") || line.starts_with("*** ") || line.starts_with("diff ")
}

fn parse_unified_hunks(lines: &[&str], mut i: usize) -> Result<(Vec<Hunk>, usize)> {
    let mut hunks = Vec::new();
    while i < lines.len() && !starts_file_header(lines[i]) {
        if !lines[i].starts_with("@@ ") {
            i += 1;
            continue;
        }

        let mut hunk = parse_hunk_header(lines[i], i + 1)?;
        i += 1;
        let mut old_seen = 0;
        let mut new_seen = 0;
        while i < lines.len() && (old_seen < hunk.old_count || new_seen < hunk.new_count) {
            let line = lines[i];
            if line.starts_with("@@ ") || starts_file_header(line) {
                break;
            }
            i += 1;
            if line.starts_with('\\') {
                drop_last_newline(&mut hunk);
                continue;
            }
            // Some tools drop the leading space from empty context lines.
            if line == "\n" || line.is_empty() {
                hunk.lines.push(HunkLine::Context(line.to_string()));
                old_seen += 1;
                new_seen += 1;
                continue;
            }

            let (tag, content) = line.split_at(1);
            let content = content.to_string();
            match tag {
                " " => {
                    hunk.lines.push(HunkLine::Context(content));
                    old_seen += 1;
                    new_seen += 1;
                }
                "-" => {
                    hunk.lines.push(HunkLine::Remove(content));
                    old_seen += 1;
                }
                "+" => {
                    hunk.lines.push(HunkLine::Add(content));
                    new_seen += 1;
                }
                _ => {
                    return Err(parse_error(
                        i,
                        "expected hunk line starting with space, -, or +",
                    ));
                }
            }
        }
        // The marker for the hunk's last line comes after its counts are met.
        if lines.get(i).is_some_and(|line| line.starts_with('\\')) {
            drop_last_newline(&mut hunk);
            i += 1;
        }
        hunks.push(hunk);
    }
    Ok((hunks, i))
}

/// Applies a `\ No newline at end of file` marker, which refers to the line
/// before it.
fn drop_last_newline(hunk: &mut Hunk) {
    if let Some(HunkLine::Context(text) | HunkLine::Remove(text) | HunkLine::Add(text)) =
        hunk.lines.last_mut()
        && text.ends_with('\n')
    {
        text.pop();
    }
}

fn parse_git_header(line: &str, line_number: usize) -> Result<(String, String)> {
    const INVALID: &str = "invalid git diff header";
    let rest = line
        .trim_end_matches('\n')
        .strip_prefix("diff --git ")
        .ok_or_else(|| parse_error(line_number, INVALID))?
        .trim_start();
    let (old_path, old_end) = parse_header_path(rest, line_number)?;
    let rest = rest[old_end..].trim_start();
    if rest.is_empty() {
        return Err(parse_error(line_number, INVALID));
    }
    let (new_path, _) = parse_header_path(rest, line_number)?;
    Ok((strip_git_prefix(&old_path), strip_git_prefix(&new_path)))
}

/// Parses `diff [options] old new`, taking the last two words as the paths.
fn parse_diff_header(line: &str, line_number: usize) -> Result<(String, String)> {
    const INVALID: &str = "invalid diff header";
    let mut rest = line
        .trim_end_matches('\n')
        .strip_prefix("diff ")
        .ok_or_else(|| parse_error(line_number, INVALID))?
        .trim_start();
    let mut paths = Vec::new();
    while !rest.is_empty() {
        let (path, end) = parse_header_path(rest, line_number)?;
        paths.push(path);
        rest = rest[end..].trim_start();
    }
    let [.., old_path, new_path] = paths.as_slice() else {
        return Err(parse_error(line_number, INVALID));
    };
    Ok((strip_git_prefix(old_path), strip_git_prefix(new_path)))
}

fn strip_git_prefix(path: &str) -> String {
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
        .to_string()
}

fn parse_index_path(line: &str, line_number: usize) -> Result<Option<String>> {
    let Some(rest) = line.strip_prefix("Index:") else {
        return Ok(None);
    };
    let rest = rest.trim_start().trim_end_matches('\n');
    if rest.is_empty() {
        return Ok(None);
    }
    let (path, _) = parse_header_path(rest, line_number)?;
    Ok((!path.is_empty()).then_some(path))
}

/// Parses the octal mode that follows a git extended header keyword.
fn parse_git_mode(raw: &str, line_number: usize) -> Result<u32> {
    let raw = raw.trim_end_matches('\n');
    u32::from_str_radix(raw, 8)
        .map_err(|_| parse_error(line_number, format!("invalid git file mode {raw:?}")))
}

/// The mode from an `index <hash>..<hash> <mode>` line, if it is a symlink.
fn parse_git_index_symlink_mode(line: &str) -> Option<u32> {
    let mode = line.trim_end_matches('\n').split_whitespace().nth(2)?;
    let parsed = u32::from_str_radix(mode, 8).ok()?;
    (parsed & 0o170000 == 0o120000).then_some(parsed)
}

/// Collects the bodies of consecutive lines starting with `prefix`.
fn take_prefixed_lines(lines: &[&str], i: &mut usize, prefix: &str) -> Vec<String> {
    let mut taken = Vec::new();
    while let Some(body) = lines.get(*i).and_then(|line| line.strip_prefix(prefix)) {
        taken.push(body.to_string());
        *i += 1;
    }
    taken
}

fn parse_normal_file(lines: &[&str], start: usize) -> Result<Option<(FilePatch, usize)>> {
    if parse_normal_command(lines[start]).is_none() {
        return Ok(None);
    }

    let mut i = start;
    let mut hunks = Vec::new();
    while let Some(command) = lines.get(i).and_then(|line| parse_normal_command(line)) {
        i += 1;
        let mut removed = Vec::new();
        let mut added = Vec::new();
        match command.kind {
            CommandKind::Add => added = take_prefixed_lines(lines, &mut i, "> "),
            CommandKind::Delete => removed = take_prefixed_lines(lines, &mut i, "< "),
            CommandKind::Change => {
                removed = take_prefixed_lines(lines, &mut i, "< ");
                if !lines.get(i).is_some_and(|line| line.starts_with("---")) {
                    return Err(parse_error(i + 1, "expected --- separator in normal diff"));
                }
                i += 1;
                added = take_prefixed_lines(lines, &mut i, "> ");
            }
        }

        let (old_start, old_count) = match command.kind {
            CommandKind::Add => (command.old_end, 0),
            _ => (command.old_start, command.old_end - command.old_start + 1),
        };
        let (new_start, new_count) = match command.kind {
            CommandKind::Delete => (command.new_end, 0),
            _ => (command.new_start, command.new_end - command.new_start + 1),
        };
        let mut hunk_lines = Vec::with_capacity(removed.len() + added.len());
        hunk_lines.extend(removed.into_iter().map(HunkLine::Remove));
        hunk_lines.extend(added.into_iter().map(HunkLine::Add));
        hunks.push(Hunk {
            old_start,
            old_count,
            new_start,
            new_count,
            section: None,
            lines: hunk_lines,
        });
    }

    Ok(Some((FilePatch::headerless(PatchFormat::Normal, hunks), i)))
}

/// The `a`/`c`/`d` command of a normal or ed diff.
#[derive(Clone, Copy)]
enum CommandKind {
    Add,
    Change,
    Delete,
}

impl CommandKind {
    fn from_letter(letter: &str) -> Option<Self> {
        match letter {
            "a" => Some(Self::Add),
            "c" => Some(Self::Change),
            "d" => Some(Self::Delete),
            _ => None,
        }
    }
}

/// A normal diff command such as `3,5c3,4`.
struct NormalCommand {
    old_start: usize,
    old_end: usize,
    new_start: usize,
    new_end: usize,
    kind: CommandKind,
}

fn is_line_range(range: &str) -> bool {
    !range.is_empty() && range.chars().all(|ch| ch.is_ascii_digit() || ch == ',')
}

fn parse_normal_command(line: &str) -> Option<NormalCommand> {
    let line = line.trim_end_matches(['\r', '\n']);
    let op_index = line.find(['a', 'c', 'd'])?;
    let (old_range, rest) = line.split_at(op_index);
    let (op, new_range) = rest.split_at(1);
    if !is_line_range(old_range) || !is_line_range(new_range) {
        return None;
    }
    let (old_start, old_end) = parse_line_range(old_range)?;
    let (new_start, new_end) = parse_line_range(new_range)?;
    Some(NormalCommand {
        old_start,
        old_end,
        new_start,
        new_end,
        kind: CommandKind::from_letter(op)?,
    })
}

/// Parses a `start[,end]` line range.
fn parse_line_range(range: &str) -> Option<(usize, usize)> {
    match range.split_once(',') {
        Some((start, end)) => Some((start.parse().ok()?, end.parse().ok()?)),
        None => {
            let line = range.parse().ok()?;
            Some((line, line))
        }
    }
}

/// Reads ed text lines up to the terminating `.` line.
fn take_ed_text(lines: &[&str], i: &mut usize) -> Result<Vec<String>> {
    let mut text = Vec::new();
    while let Some(line) = lines.get(*i) {
        *i += 1;
        if line.trim_end_matches('\n') == "." {
            return Ok(text);
        }
        text.push(line.to_string());
    }
    Err(parse_error(*i + 1, "unterminated ed command"))
}

fn parse_ed_hunks(lines: &[&str], mut i: usize) -> Result<Option<(Vec<Hunk>, usize)>> {
    if lines
        .get(i)
        .and_then(|line| parse_ed_command(line))
        .is_none()
    {
        return Ok(None);
    }

    let mut hunks = Vec::new();
    while let Some((start, end, kind)) = lines.get(i).and_then(|line| parse_ed_command(line)) {
        i += 1;
        // Ed scripts do not carry the removed text; only the count matters.
        let (removed_count, old_count) = match kind {
            CommandKind::Add => (0, 0),
            CommandKind::Change | CommandKind::Delete => ((start..=end).count(), end - start + 1),
        };
        let added = match kind {
            CommandKind::Add | CommandKind::Change => take_ed_text(lines, &mut i)?,
            CommandKind::Delete => Vec::new(),
        };
        let new_count = match kind {
            CommandKind::Delete => 0,
            _ => added.len(),
        };
        let (old_start, new_start) = match kind {
            CommandKind::Add => (end, if end == 0 { 1 } else { start }),
            CommandKind::Change => (start, start),
            CommandKind::Delete => (start, start.saturating_sub(1)),
        };
        let mut hunk_lines = Vec::with_capacity(removed_count + added.len());
        hunk_lines.extend((0..removed_count).map(|_| HunkLine::Remove(String::new())));
        hunk_lines.extend(added.into_iter().map(HunkLine::Add));
        hunks.push(Hunk {
            old_start,
            old_count,
            new_start,
            new_count,
            section: None,
            lines: hunk_lines,
        });
    }

    Ok(Some((hunks, i)))
}

/// Parses an ed command such as `3,5c`, returning (start, end, kind).
fn parse_ed_command(line: &str) -> Option<(usize, usize, CommandKind)> {
    let line = line.trim_end_matches(['\r', '\n']);
    let op_index = line.find(['a', 'c', 'd'])?;
    if op_index + 1 != line.len() {
        return None;
    }
    let (range, op) = line.split_at(op_index);
    if !is_line_range(range) {
        return None;
    }
    let (start, end) = parse_line_range(range)?;
    Some((start, end, CommandKind::from_letter(op)?))
}

fn looks_like_context_file_header(lines: &[&str], start: usize) -> bool {
    let starts = |offset: usize, prefix: &str| {
        lines
            .get(start + offset)
            .is_some_and(|line| line.starts_with(prefix))
    };
    starts(0, "*** ") && starts(1, "--- ") && starts(2, CONTEXT_HUNK_SEPARATOR)
}

fn parse_context_file(lines: &[&str], start: usize) -> Result<(FilePatch, usize)> {
    let (old_path, old_label) = parse_header(lines[start], "*** ", start + 1)?;
    let mut i = start + 1;
    if !lines.get(i).is_some_and(|line| line.starts_with("--- ")) {
        return Err(parse_error(
            i + 1,
            "expected --- file header in context diff",
        ));
    }
    let (new_path, new_label) = parse_header(lines[i], "--- ", i + 1)?;
    i += 1;

    let mut hunks = Vec::new();
    while i < lines.len() && !lines[i].starts_with("*** ") {
        if lines[i].starts_with(CONTEXT_HUNK_SEPARATOR) {
            let (hunk, next) = parse_context_hunk(lines, i, false)?;
            hunks.push(hunk);
            i = next;
        } else {
            i += 1;
        }
    }

    Ok((
        FilePatch {
            old_path,
            new_path,
            old_label,
            new_label,
            hunks,
            ..FilePatch::headerless(PatchFormat::Context, Vec::new())
        },
        i,
    ))
}

/// Parses consecutive context hunks that appear without file headers.
fn parse_context_hunks(lines: &[&str], mut i: usize) -> Result<(Vec<Hunk>, usize)> {
    let mut hunks = Vec::new();
    while lines
        .get(i)
        .is_some_and(|line| line.starts_with(CONTEXT_HUNK_SEPARATOR))
    {
        let (hunk, next) = parse_context_hunk(lines, i, true)?;
        hunks.push(hunk);
        i = next;
    }
    Ok((hunks, i))
}

/// Parses one context hunk starting at its `***************` line. With
/// `stop_at_diff`, a `diff ` line also ends the new-side lines.
fn parse_context_hunk(lines: &[&str], start: usize, stop_at_diff: bool) -> Result<(Hunk, usize)> {
    let section = context_section(lines[start]);
    let mut i = start + 1;
    if !lines.get(i).is_some_and(|line| line.starts_with("*** ")) {
        return Err(parse_error(i + 1, "expected old range in context hunk"));
    }
    let old_range_line = i + 1;
    let (old_start, old_count) = parse_context_range_header(
        lines[i],
        CONTEXT_OLD_RANGE_PREFIX,
        CONTEXT_OLD_RANGE_SUFFIX,
        old_range_line,
    )?;
    i += 1;

    let mut old_lines = Vec::new();
    while i < lines.len() && !lines[i].starts_with("--- ") {
        old_lines.push(parse_context_hunk_line(lines[i], i + 1)?);
        i += 1;
    }
    if i >= lines.len() {
        return Err(parse_error(i + 1, "expected new range in context hunk"));
    }
    let (new_start, new_count) = parse_context_range_header(
        lines[i],
        CONTEXT_NEW_RANGE_PREFIX,
        CONTEXT_NEW_RANGE_SUFFIX,
        i + 1,
    )?;
    i += 1;

    let mut new_lines = Vec::new();
    while i < lines.len()
        && !lines[i].starts_with(CONTEXT_HUNK_SEPARATOR)
        && !lines[i].starts_with("*** ")
        && !(stop_at_diff && lines[i].starts_with("diff "))
    {
        new_lines.push(parse_context_hunk_line(lines[i], i + 1)?);
        i += 1;
    }
    // An omitted new side means the hunk only removes lines.
    if new_lines.is_empty() {
        new_lines = old_lines
            .iter()
            .filter(|(tag, _)| *tag != '-')
            .cloned()
            .collect();
    }

    let hunk_lines = merge_context_hunk_lines(&old_lines, &new_lines);
    let old_body_count = old_lines.iter().filter(|(tag, _)| *tag != '+').count();
    let new_body_count = new_lines.iter().filter(|(tag, _)| *tag != '-').count();
    if old_body_count > old_count || new_body_count > new_count {
        return Err(parse_error(
            old_range_line,
            "replacement text or line numbers mangled in hunk",
        ));
    }
    let hunk = Hunk {
        old_start: if old_body_count == 0 {
            new_start
        } else {
            old_start
        },
        old_count: old_body_count,
        new_start,
        new_count: new_body_count,
        section,
        lines: hunk_lines,
    };
    Ok((hunk, i))
}

fn parse_context_range_header(
    line: &str,
    prefix: &str,
    suffix: &str,
    line_number: usize,
) -> Result<(usize, usize)> {
    let Some(range) = line
        .strip_prefix(prefix)
        .and_then(|value| value.split(suffix).next())
    else {
        return Err(parse_error(line_number, "invalid context range header"));
    };
    parse_context_range(range.trim(), line_number)
}

/// Parses a context `start[,end]` range into (start, count).
fn parse_context_range(range: &str, line_number: usize) -> Result<(usize, usize)> {
    match range.split_once(',') {
        Some((start, end)) => {
            let start = parse_usize(start.trim(), line_number)?;
            let end = parse_usize(end.trim(), line_number)?;
            Ok((start, end.saturating_sub(start).saturating_add(1)))
        }
        None => Ok((parse_usize(range.trim(), line_number)?, 1)),
    }
}

fn parse_context_hunk_line(line: &str, line_number: usize) -> Result<(char, String)> {
    if line.starts_with("\\ No newline at end of file") {
        return Ok((' ', String::new()));
    }
    let mut chars = line.chars();
    let Some(tag @ (' ' | '-' | '+' | '!')) = chars.next() else {
        return Err(parse_error(
            line_number,
            "expected context hunk line starting with space, -, +, or !",
        ));
    };
    let content = chars.as_str().strip_prefix(' ').unwrap_or(chars.as_str());
    Ok((tag, content.to_string()))
}

/// Interleaves the old and new sides of a context hunk into unified-style lines.
fn merge_context_hunk_lines(
    old_lines: &[(char, String)],
    new_lines: &[(char, String)],
) -> Vec<HunkLine> {
    let mut merged = Vec::new();
    let mut old_index = 0;
    let mut new_index = 0;
    let is_context = |lines: &[(char, String)], index: usize| {
        lines.get(index).is_some_and(|(tag, _)| *tag == ' ')
    };

    while old_index < old_lines.len() || new_index < new_lines.len() {
        if is_context(old_lines, old_index) && is_context(new_lines, new_index) {
            merged.push(HunkLine::Context(old_lines[old_index].1.clone()));
            old_index += 1;
            new_index += 1;
            continue;
        }

        while old_index < old_lines.len() && !is_context(old_lines, old_index) {
            merged.push(HunkLine::Remove(old_lines[old_index].1.clone()));
            old_index += 1;
        }
        while new_index < new_lines.len() && !is_context(new_lines, new_index) {
            merged.push(HunkLine::Add(new_lines[new_index].1.clone()));
            new_index += 1;
        }

        if old_index < old_lines.len()
            && (new_index >= new_lines.len() || is_context(old_lines, old_index))
        {
            merged.push(HunkLine::Context(old_lines[old_index].1.clone()));
            old_index += 1;
            if is_context(new_lines, new_index) {
                new_index += 1;
            }
        } else if new_index < new_lines.len() {
            merged.push(HunkLine::Context(new_lines[new_index].1.clone()));
            new_index += 1;
        }
    }

    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unified_patch() {
        let patch =
            parse_patch("--- a/file\n+++ b/file\n@@ -1,2 +1,2 @@\n one\n-two\n+three\n").unwrap();
        assert_eq!(patch.files.len(), 1);
        assert_eq!(patch.files[0].hunks[0].old_start, 1);
        assert_eq!(patch.files[0].hunks[0].new_count, 2);
    }

    #[test]
    fn parses_context_patch() {
        let patch = parse_patch(
            "*** a\n--- b\n***************\n*** 1,2 ****\n  1\n- 2\n--- 1,2 ----\n  1\n+ two\n",
        )
        .unwrap();
        assert_eq!(patch.files.len(), 1);
        assert_eq!(patch.files[0].hunks[0].lines.len(), 3);
    }
}
