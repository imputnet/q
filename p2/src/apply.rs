use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use filetime::set_file_mtime;
use rayon::prelude::*;

use crate::fs_util::{
    create_parent_dirs, is_read_only_permissions, path_parent_is_invalid, read_lines,
    read_symlink_text, read_text, remove_empty_parent_dirs, set_file_mode, split_lines,
    target_status, write_output_file, write_symlink_output,
};
use crate::hunk::{
    HunkMatchOptions, OrderedMatch, estimate_fuzz, find_hunk_match,
    hunk_matches_after_crlf_normalization, hunk_start, match_ordered_hunk, unfuzzed_replacement,
    unfuzzed_replacement_lines,
};
use crate::merge::{already_applied_merge_hunk, apply_merge_hunk};
use crate::paths::{
    effective_strip, empties_file, is_creation_only_patch, no_usable_path, patch_changes_symlink,
    selected_file_target, source_path, strip_patch_path, target_path,
};
use crate::reject::render_reject;
use crate::timestamp::{
    TimestampMode, has_deletion_timestamp, source_timestamp, source_timestamp_matches,
    target_timestamp,
};
use crate::{
    ApplyOptions, ApplyReport, FilePatch, FileReport, GitOperation, Hunk, HunkLine, HunkReport,
    HunkStatus, MergeMode, PatchFormat, PatchSet, ReadOnlyBehavior, ReportOperation, Result,
    io_error,
};

/// Patch sets touching at least this many distinct files apply files in parallel.
const PARALLEL_APPLY_MIN_FILES: usize = 512;
/// Series of at least this many patches against one file apply in memory.
const GROUPED_APPLY_MIN_FILES: usize = 16;

/// Original contents of git copy/rename sources, captured before any file in
/// the patch set is modified.
type GitSources = HashMap<PathBuf, Vec<String>>;

struct ApplyPathPolicy {
    canonical_root: PathBuf,
    allow_absolute: bool,
}

impl ApplyPathPolicy {
    fn new(root: &Path) -> Result<Self> {
        let canonical_root = root.canonicalize().map_err(io_error(root))?;
        let allow_absolute = canonical_root.parent().is_none();
        Ok(Self {
            canonical_root,
            allow_absolute,
        })
    }
}

pub fn apply_patch(
    root: impl AsRef<Path>,
    patch: &PatchSet,
    options: &ApplyOptions,
) -> Result<ApplyReport> {
    let root = root.as_ref();
    let policy = ApplyPathPolicy::new(root)?;
    if let Some(report) = grouped_apply_patch(root, patch, options, &policy)? {
        return Ok(report);
    }
    let git_sources = snapshot_git_sources(root, patch, options.reverse)?;
    let canonical_root = &policy.canonical_root;
    let files = match parallel_apply_targets(root, patch, options, policy.allow_absolute)? {
        Some(targets) => patch
            .files
            .par_iter()
            .zip(targets.into_par_iter())
            .map(|(file_patch, target)| {
                apply_file_patch(
                    root,
                    file_patch,
                    options,
                    &git_sources,
                    target,
                    canonical_root,
                )
            })
            .collect::<Result<Vec<_>>>()?,
        // Each target is chosen only after earlier files have been patched,
        // since target selection looks at which files exist.
        None => patch
            .files
            .iter()
            .map(|file_patch| {
                let target =
                    selected_file_target(root, file_patch, options, policy.allow_absolute)?;
                apply_file_patch(
                    root,
                    file_patch,
                    options,
                    &git_sources,
                    target,
                    canonical_root,
                )
            })
            .collect::<Result<Vec<_>>>()?,
    };
    Ok(ApplyReport { files })
}

/// Applies a long series of simple patches to a single file in memory,
/// reading and writing it once. Returns `None` when the patch set does not
/// qualify or any hunk needs the general path.
fn grouped_apply_patch(
    root: &Path,
    patch: &PatchSet,
    options: &ApplyOptions,
    policy: &ApplyPathPolicy,
) -> Result<Option<ApplyReport>> {
    if patch.files.len() < GROUPED_APPLY_MIN_FILES || !grouped_apply_options_are_simple(options) {
        return Ok(None);
    }

    let mut target = None;
    for file_patch in &patch.files {
        let Some(file_target) =
            grouped_apply_file_target(file_patch, options, policy.allow_absolute)?
        else {
            return Ok(None);
        };
        if target.as_ref().is_some_and(|target| target != &file_target) {
            return Ok(None);
        }
        target = Some(file_target);
    }
    let Some(target) = target else {
        return Ok(None);
    };

    if path_parent_is_invalid(&policy.canonical_root, &target)? {
        return Ok(None);
    }
    let full_path = root.join(&target);
    let Some(permissions) = target_status(&full_path)?.permissions else {
        return Ok(None);
    };
    if is_read_only_permissions(&permissions) {
        return Ok(None);
    }

    let mut output = read_lines(&full_path)?;
    let mut output_changed = false;
    let mut reports = Vec::with_capacity(patch.files.len());
    for file_patch in &patch.files {
        let Some(applied) = apply_ordered_hunks_in_place(file_patch, false, &mut output, options)
        else {
            return Ok(None);
        };
        output_changed |= applied.changed;
        reports.push(FileReport {
            hunks_applied: count_applied(&applied.hunk_reports),
            hunk_reports: applied.hunk_reports,
            ..FileReport::new(target.clone())
        });
    }

    if output_changed {
        write_output_file(&full_path, output.concat(), Some(permissions))?;
    }
    Ok(Some(ApplyReport { files: reports }))
}

fn grouped_apply_options_are_simple(options: &ApplyOptions) -> bool {
    !options.reverse
        && !options.dry_run
        && options.target_file.is_none()
        && !options.remove_empty
        && options.read_only == ReadOnlyBehavior::Ignore
        && options.reject_format.is_none()
        && options.ifdef_symbol.is_none()
        && options.merge == MergeMode::None
        && !options.set_time
        && !options.set_utc
        && !options.follow_symlinks
        && !options.ignore_whitespace
}

/// The file a patch modifies in place, if it is a plain content change.
fn grouped_apply_file_target(
    patch: &FilePatch,
    options: &ApplyOptions,
    allow_absolute: bool,
) -> Result<Option<PathBuf>> {
    if patch.format != PatchFormat::Unified
        || patch.git_operation.is_some()
        || patch.old_file_mode.is_some()
        || patch.new_file_mode.is_some()
        || patch_changes_symlink(patch, options.reverse)
        || has_deletion_timestamp(patch, options.reverse)
        || is_creation_only_patch(patch)
        || empties_file(patch)
    {
        return Ok(None);
    }

    let (Some(source), Some(target)) = (
        source_path(patch, options.reverse),
        target_path(patch, options.reverse),
    ) else {
        return Ok(None);
    };
    let strip = effective_strip(patch, options);
    let stripped_source = strip_patch_path(source, strip, allow_absolute)?;
    let stripped_target = strip_patch_path(target, strip, allow_absolute)?;
    Ok((stripped_source == stripped_target).then_some(stripped_target))
}

/// Targets for applying every file patch in parallel, or `None` if the patch
/// set is small or any two file patches might touch the same path.
fn parallel_apply_targets(
    root: &Path,
    patch: &PatchSet,
    options: &ApplyOptions,
    allow_absolute: bool,
) -> Result<Option<Vec<PathBuf>>> {
    if patch.files.len() < PARALLEL_APPLY_MIN_FILES
        || options.target_file.is_some()
        || options.remove_empty
    {
        return Ok(None);
    }

    let mut seen_paths = HashSet::with_capacity(patch.files.len());
    let mut targets = Vec::with_capacity(patch.files.len());
    for file_patch in &patch.files {
        if file_patch.git_operation.is_some() || patch_changes_symlink(file_patch, options.reverse)
        {
            return Ok(None);
        }

        let target = selected_file_target(root, file_patch, options, allow_absolute)?;
        let Some(touched) = parallel_candidate_paths(file_patch, options, allow_absolute) else {
            return Ok(None);
        };
        if touched.iter().any(|path| seen_paths.contains(path)) {
            return Ok(None);
        }
        seen_paths.extend(touched);
        targets.push(target);
    }
    Ok(Some(targets))
}

fn parallel_candidate_paths(
    patch: &FilePatch,
    options: &ApplyOptions,
    allow_absolute: bool,
) -> Option<Vec<PathBuf>> {
    let strip = effective_strip(patch, options);
    let mut paths = Vec::with_capacity(2);
    for path in [
        source_path(patch, options.reverse),
        target_path(patch, options.reverse),
    ]
    .into_iter()
    .flatten()
    {
        let stripped = strip_patch_path(path, strip, allow_absolute).ok()?;
        if !paths.contains(&stripped) {
            paths.push(stripped);
        }
    }
    Some(paths)
}

fn snapshot_git_sources(root: &Path, patch: &PatchSet, reverse: bool) -> Result<GitSources> {
    let mut sources = HashMap::new();
    for file_patch in &patch.files {
        if file_patch.git_operation.is_none() {
            continue;
        }
        let source = source_path(file_patch, reverse).ok_or_else(no_usable_path)?;
        let source_path = strip_patch_path(source, 0, false)?;
        if sources.contains_key(&source_path) {
            continue;
        }
        let full_path = root.join(&source_path);
        if full_path.exists() {
            sources.insert(source_path, read_lines(&full_path)?);
        }
    }
    Ok(sources)
}

fn hunk_report(index: usize, status: HunkStatus) -> HunkReport {
    HunkReport {
        hunk: index + 1,
        status,
        reject: None,
        different_line_endings: false,
    }
}

fn failed_hunk_report(
    index: usize,
    patch: &FilePatch,
    hunk: &Hunk,
    requested_line: usize,
    options: &ApplyOptions,
) -> HunkReport {
    HunkReport {
        reject: Some(render_reject(patch, hunk, options.reject_format)),
        ..hunk_report(index, HunkStatus::Failed { requested_line })
    }
}

fn count_applied(hunk_reports: &[HunkReport]) -> usize {
    hunk_reports
        .iter()
        .filter(|hunk| {
            matches!(
                hunk.status,
                HunkStatus::Applied { .. }
                    | HunkStatus::Merged { .. }
                    | HunkStatus::AlreadyApplied { .. }
            )
        })
        .count()
}

/// A report for a file that was not patched at all; every hunk is rejected.
fn refused_report(target: PathBuf, patch: &FilePatch, options: &ApplyOptions) -> FileReport {
    let hunk_reports = patch
        .hunks
        .iter()
        .enumerate()
        .map(|(index, hunk)| failed_hunk_report(index, patch, hunk, hunk.old_start, options))
        .collect();
    FileReport {
        hunk_reports,
        skipped: true,
        ignored_hunks: patch.hunks.len(),
        ..FileReport::new(target)
    }
}

fn apply_file_patch(
    root: &Path,
    patch: &FilePatch,
    options: &ApplyOptions,
    git_sources: &GitSources,
    target: PathBuf,
    canonical_root: &Path,
) -> Result<FileReport> {
    let creates_file = source_path(patch, options.reverse).is_none();
    let deletes_file = target_path(patch, options.reverse).is_none()
        || has_deletion_timestamp(patch, options.reverse);
    let full_path = root.join(&target);

    if path_parent_is_invalid(canonical_root, &target)? {
        return Ok(FileReport {
            invalid_path_refused: true,
            ..refused_report(target, patch, options)
        });
    }

    let status = target_status(&full_path)?;
    let existed = status.existed;
    if !options.follow_symlinks
        && status.is_symlink
        && !patch_changes_symlink(patch, options.reverse)
    {
        return Ok(FileReport {
            not_regular_refused: true,
            ..refused_report(target, patch, options)
        });
    }

    // Keep file contents as one buffer and borrow lines from it. Allocating a
    // String per line dominated apply time on large files; only the slow
    // (offset/fuzz/merge) path materializes owned lines.
    let target_original = match (existed, status.is_symlink) {
        (false, _) => None,
        (true, true) if patch_changes_symlink(patch, options.reverse) => {
            Some(read_symlink_text(&full_path)?)
        }
        (true, _) => Some(read_text(&full_path)?),
    };
    let plan = plan_file_operation(root, patch, options.reverse, existed)?;
    let source_text = match &plan.content_source {
        Some(source) if !existed && !git_sources.contains_key(source) => {
            Some(read_text(&root.join(source))?)
        }
        _ => None,
    };
    let original: Vec<&str> = if let Some(lines) = plan
        .content_source
        .as_ref()
        .and_then(|source| git_sources.get(source))
    {
        lines.iter().map(String::as_str).collect()
    } else if let Some(text) = target_original.as_deref().or(source_text.as_deref()) {
        split_lines(text)
    } else {
        Vec::new()
    };

    let read_only = status
        .permissions
        .as_ref()
        .is_some_and(is_read_only_permissions);
    if read_only && options.read_only == ReadOnlyBehavior::Fail {
        return Ok(FileReport {
            operation: plan.report_operation,
            backup_path: plan.backup_path,
            read_only: true,
            read_only_refused: true,
            ..refused_report(target, patch, options)
        });
    }

    let create_conflict = creates_file && existed;
    let delete_missing = deletes_file && !existed;
    let empties_existing = existed && original.is_empty() && empties_file(patch);
    let assumed_reverse = options.batch && !options.reverse && empties_existing;
    let empty_conflict = options.force && !assumed_reverse && empties_existing;
    let reverse = options.reverse || assumed_reverse;
    if delete_missing && !options.force && !options.batch {
        return Ok(FileReport {
            operation: plan.report_operation,
            backup_path: plan.backup_path,
            delete_missing: true,
            read_only,
            skipped: true,
            ignored_hunks: patch.hunks.len(),
            ..FileReport::new(target)
        });
    }

    let (output, hunk_reports) =
        match apply_ordered_hunks_fast(patch, reverse, &original, options, create_conflict) {
            Some(applied) => (applied.output, applied.hunk_reports),
            None => {
                let (output, hunk_reports) =
                    apply_hunks_with_search(patch, reverse, &original, options, create_conflict);
                (output.into_iter().map(Cow::Owned).collect(), hunk_reports)
            }
        };

    let timestamp_mode = TimestampMode::from_options(options);
    let source_timestamp = source_timestamp(patch, reverse, timestamp_mode);
    let target_timestamp = target_timestamp(patch, reverse, timestamp_mode);
    let effective_deletes_file = deletes_file && !assumed_reverse;
    let any_failed = hunk_reports.iter().any(|hunk| hunk.status.is_failure());
    let time_mismatch = target_timestamp.is_some()
        && (any_failed || !source_timestamp_matches(&full_path, existed, source_timestamp)?);
    let delete_conflict = effective_deletes_file && !output.is_empty() && !any_failed;
    let deleted =
        ((effective_deletes_file && !options.posix) || options.remove_empty) && output.is_empty();
    let writes_symlink = patch_changes_symlink(patch, reverse);

    if !options.dry_run {
        if deleted {
            if existed {
                fs::remove_file(&full_path).map_err(io_error(&full_path))?;
                if options.remove_empty {
                    remove_empty_parent_dirs(root, &full_path)?;
                }
            }
        } else if writes_symlink {
            write_symlink_output(&full_path, &concat_lines(&output))?;
        } else {
            let unchanged = target_original.as_deref().is_some_and(|text| {
                text.split_inclusive('\n')
                    .eq(output.iter().map(|line| line.as_ref()))
            });
            if !unchanged {
                if !existed {
                    create_parent_dirs(&full_path)?;
                }
                write_output_file(&full_path, concat_lines(&output), status.permissions)?;
            }
            let mode = if reverse {
                patch.old_file_mode
            } else {
                patch.new_file_mode
            };
            if let Some(mode) = mode {
                set_file_mode(&full_path, mode & 0o777)?;
            }
        }
        if plan.remove_source_after_apply
            && let Some(source) = &plan.content_source
        {
            let full_source = root.join(source);
            if full_source.exists() && full_source != full_path {
                fs::remove_file(&full_source).map_err(io_error(&full_source))?;
            }
        }
        if !deleted
            && !time_mismatch
            && let Some(timestamp) = target_timestamp
        {
            set_file_mtime(&full_path, timestamp).map_err(io_error(&full_path))?;
        }
    }

    Ok(FileReport {
        operation: plan.report_operation,
        backup_path: plan.backup_path,
        hunks_applied: count_applied(&hunk_reports),
        hunk_reports,
        deleted,
        delete_conflict,
        create_conflict,
        delete_missing,
        empty_conflict,
        read_only,
        assumed_reverse,
        created: !existed && !deletes_file,
        time_mismatch,
        symbolic_link: writes_symlink,
        ..FileReport::new(target)
    })
}

/// The general apply path: searches for each hunk around its expected
/// position with offsets and fuzz, and handles ed scripts and merges.
fn apply_hunks_with_search(
    patch: &FilePatch,
    reverse: bool,
    original: &[&str],
    options: &ApplyOptions,
    create_conflict: bool,
) -> (Vec<String>, Vec<HunkReport>) {
    let mut output: Vec<String> = original.iter().map(|line| (*line).to_owned()).collect();
    let merging = options.merge != MergeMode::None;
    let mut delta: isize = 0;
    let mut lowest_allowed_start = 0;
    let mut hunk_reports = Vec::with_capacity(patch.hunks.len());

    for (index, hunk) in patch.hunks.iter().enumerate() {
        let failed =
            |requested_line| failed_hunk_report(index, patch, hunk, requested_line, options);
        let old_start = hunk_start(hunk, reverse);
        let Ok(base) = usize::try_from(old_start.saturating_sub(1) as isize + delta) else {
            hunk_reports.push(failed(old_start));
            continue;
        };
        let requested_line = base + 1;
        if create_conflict {
            hunk_reports.push(failed(requested_line));
            continue;
        }
        if merging
            && let Some(already_applied) = already_applied_merge_hunk(
                hunk,
                reverse,
                &output,
                base,
                requested_line,
                options.ignore_whitespace,
            )
        {
            lowest_allowed_start = already_applied.next_allowed_start;
            hunk_reports.push(hunk_report(index, already_applied.status));
            continue;
        }
        if patch.format == PatchFormat::Ed {
            hunk_reports.push(match apply_ed_hunk(hunk, reverse, &mut output) {
                Some(hunk_delta) => {
                    delta += hunk_delta;
                    hunk_report(
                        index,
                        HunkStatus::Applied {
                            requested_line,
                            applied_line: requested_line,
                            fuzz: 0,
                        },
                    )
                }
                None => failed(requested_line),
            });
            continue;
        }

        let match_options = HunkMatchOptions {
            ignore_whitespace: options.ignore_whitespace,
            max_fuzz: if merging { 0 } else { options.max_fuzz },
            ifdef_symbol: options.ifdef_symbol.as_deref(),
        };
        let Some(matched) = find_hunk_match(
            hunk,
            reverse,
            &output,
            base,
            lowest_allowed_start,
            match_options,
        ) else {
            if merging {
                let merged = apply_merge_hunk(
                    hunk,
                    reverse,
                    &mut output,
                    base,
                    requested_line,
                    options.merge,
                    options.ignore_whitespace,
                );
                delta += merged.replacement_len as isize - merged.expected_len as isize;
                lowest_allowed_start = merged.next_allowed_start;
                hunk_reports.push(hunk_report(index, merged.status));
            } else {
                hunk_reports.push(HunkReport {
                    different_line_endings: hunk_matches_after_crlf_normalization(
                        hunk,
                        reverse,
                        &output,
                        base,
                        options.ignore_whitespace,
                    ),
                    ..failed(requested_line)
                });
            }
            continue;
        };

        let start = matched.start;
        let replacement_len = matched.replacement.len();
        output.splice(
            start..start.saturating_add(matched.expected_len),
            matched.replacement,
        );
        lowest_allowed_start = start + replacement_len;
        delta += replacement_len as isize - matched.expected_len as isize;
        let applied_line = if old_start == 0 {
            requested_line
        } else {
            matched.reported_line
        };
        let fuzz = matched.fuzz.max(estimate_fuzz(
            matched.prefix_context,
            matched.suffix_context,
            requested_line,
            applied_line,
        ));
        hunk_reports.push(hunk_report(
            index,
            HunkStatus::Applied {
                // Merge mode reports where the hunk landed, not where it was expected.
                requested_line: if merging {
                    applied_line
                } else {
                    requested_line
                },
                applied_line,
                fuzz,
            },
        ));
    }
    (output, hunk_reports)
}

/// Applies an ed-script hunk at its exact position, returning the line-count
/// change.
fn apply_ed_hunk(hunk: &Hunk, reverse: bool, output: &mut Vec<String>) -> Option<isize> {
    if reverse {
        return None;
    }
    let expected_len = hunk.old_count;
    let start = if expected_len == 0 {
        hunk.old_start
    } else {
        hunk.old_start.checked_sub(1)?
    };
    if start + expected_len > output.len() {
        return None;
    }
    let replacement = hunk
        .lines
        .iter()
        .filter_map(|line| match line {
            HunkLine::Add(value) => Some(value.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let replacement_len = replacement.len();
    output.splice(start..start + expected_len, replacement);
    Some(replacement_len as isize - expected_len as isize)
}

fn ordered_apply_supported(patch: &FilePatch, options: &ApplyOptions) -> bool {
    patch.format == PatchFormat::Unified
        && options.merge == MergeMode::None
        && !options.ignore_whitespace
        && options.ifdef_symbol.is_none()
        && !patch.hunks.is_empty()
}

fn ordered_hunk_report(
    index: usize,
    matched: &OrderedMatch,
    old_start: usize,
    requested_line: usize,
) -> HunkReport {
    let reported_line = matched.start + 1 - matched.prefix_stripped.min(matched.start);
    let applied_line = if old_start == 0 {
        requested_line
    } else {
        reported_line
    };
    let fuzz = matched.fuzz.max(estimate_fuzz(
        matched.prefix_context,
        matched.suffix_context,
        requested_line,
        applied_line,
    ));
    hunk_report(
        index,
        HunkStatus::Applied {
            requested_line,
            applied_line,
            fuzz,
        },
    )
}

pub(crate) struct FastAppliedPatch<'a> {
    pub(crate) output: Vec<Cow<'a, str>>,
    pub(crate) hunk_reports: Vec<HunkReport>,
}

/// Fast path for hunks that apply in order at their stated positions:
/// builds the output by borrowing unchanged lines from `original`. Returns
/// `None` if any hunk would need searching, so the caller can fall back.
pub(crate) fn apply_ordered_hunks_fast<'a>(
    patch: &'a FilePatch,
    reverse: bool,
    original: &[&'a str],
    options: &ApplyOptions,
    create_conflict: bool,
) -> Option<FastAppliedPatch<'a>> {
    if create_conflict || !ordered_apply_supported(patch, options) {
        return None;
    }

    let mut output =
        Vec::with_capacity(estimated_output_line_count(patch, reverse, original.len()));
    let mut hunk_reports = Vec::with_capacity(patch.hunks.len());
    let mut cursor = 0;
    let mut delta: isize = 0;

    for (index, hunk) in patch.hunks.iter().enumerate() {
        let old_start = hunk_start(hunk, reverse);
        let original_start = old_start.saturating_sub(1);
        if original_start < cursor {
            return None;
        }
        let matched =
            match_ordered_hunk(hunk, reverse, original, original_start, options.max_fuzz)?;

        let requested_base = original_start as isize + delta;
        let requested_line = if requested_base < 0 {
            old_start
        } else {
            requested_base as usize + 1
        };
        hunk_reports.push(ordered_hunk_report(
            index,
            &matched,
            old_start,
            requested_line,
        ));
        delta += matched.replacement_len as isize - (matched.end - matched.start) as isize;

        output.extend(
            original[cursor..matched.start]
                .iter()
                .map(|line| Cow::Borrowed(*line)),
        );
        match matched.fuzzed_replacement {
            None => output.extend(
                unfuzzed_replacement_lines(hunk, reverse).map(|line| Cow::Borrowed(line.as_str())),
            ),
            Some(replacement) => output.extend(replacement.into_iter().map(Cow::Owned)),
        }
        cursor = matched.end;
    }

    output.extend(original[cursor..].iter().map(|line| Cow::Borrowed(*line)));
    Some(FastAppliedPatch {
        output,
        hunk_reports,
    })
}

struct InPlaceAppliedPatch {
    hunk_reports: Vec<HunkReport>,
    changed: bool,
}

/// Like `apply_ordered_hunks_fast`, but edits `output` in place so a series
/// of patches can be applied to one buffer (see `grouped_apply_patch`).
fn apply_ordered_hunks_in_place(
    patch: &FilePatch,
    reverse: bool,
    output: &mut Vec<String>,
    options: &ApplyOptions,
) -> Option<InPlaceAppliedPatch> {
    if !ordered_apply_supported(patch, options) {
        return None;
    }

    let mut hunk_reports = Vec::with_capacity(patch.hunks.len());
    let mut changed = false;
    let mut cursor = 0;
    let mut delta: isize = 0;

    for (index, hunk) in patch.hunks.iter().enumerate() {
        let old_start = hunk_start(hunk, reverse);
        let start = usize::try_from(old_start.saturating_sub(1) as isize + delta).ok()?;
        if start < cursor {
            return None;
        }
        let matched = match_ordered_hunk(hunk, reverse, output, start, options.max_fuzz)?;
        hunk_reports.push(ordered_hunk_report(index, &matched, old_start, start + 1));

        let replacement = matched
            .fuzzed_replacement
            .unwrap_or_else(|| unfuzzed_replacement(hunk, reverse));
        let replacement_len = replacement.len();
        changed |= replacement[..] != output[matched.start..matched.end];
        output.splice(matched.start..matched.end, replacement);
        cursor = matched.start + replacement_len;
        delta += replacement_len as isize - (matched.end - matched.start) as isize;
    }

    Some(InPlaceAppliedPatch {
        hunk_reports,
        changed,
    })
}

fn estimated_output_line_count(patch: &FilePatch, reverse: bool, original_len: usize) -> usize {
    patch.hunks.iter().fold(original_len, |len, hunk| {
        let (expected, replacement) = if reverse {
            (hunk.new_count, hunk.old_count)
        } else {
            (hunk.old_count, hunk.new_count)
        };
        len.saturating_add(replacement.saturating_sub(expected))
    })
}

fn concat_lines(lines: &[Cow<'_, str>]) -> String {
    let mut out = String::with_capacity(lines.iter().map(|line| line.len()).sum());
    for line in lines {
        out.push_str(line);
    }
    out
}

/// How a git copy or rename is carried out for one file patch.
struct FileOperationPlan {
    report_operation: ReportOperation,
    /// The file whose contents the patch applies to, when it is not the target.
    content_source: Option<PathBuf>,
    remove_source_after_apply: bool,
    backup_path: Option<PathBuf>,
}

fn plan_file_operation(
    root: &Path,
    patch: &FilePatch,
    reverse: bool,
    target_existed: bool,
) -> Result<FileOperationPlan> {
    let Some(operation) = patch.git_operation else {
        return Ok(FileOperationPlan {
            report_operation: ReportOperation::Patch,
            content_source: None,
            remove_source_after_apply: false,
            backup_path: None,
        });
    };

    let source = source_path(patch, reverse).ok_or_else(no_usable_path)?;
    let source = strip_patch_path(source, 0, false)?;
    let source_exists = root.join(&source).exists();
    let remove_source = operation == GitOperation::Rename && source_exists && !target_existed;
    let report_operation = match operation {
        GitOperation::Copy => ReportOperation::Copy {
            from: source.clone(),
        },
        GitOperation::Rename => ReportOperation::Rename {
            from: source.clone(),
            already_done: !source_exists && target_existed,
        },
    };
    Ok(FileOperationPlan {
        report_operation,
        content_source: source_exists.then(|| source.clone()),
        remove_source_after_apply: remove_source,
        backup_path: remove_source.then_some(source),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_patch;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tempdir(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        for attempt in 0..100 {
            let dir = std::env::temp_dir().join(format!(
                "p2-lib-{name}-{stamp}-{}-{attempt}",
                std::process::id()
            ));
            match fs::create_dir(&dir) {
                Ok(()) => return dir,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("failed to create {}: {error}", dir.display()),
            }
        }
        panic!("failed to create unique tempdir for {name}");
    }

    fn dependent_series(count: usize) -> PatchSet {
        let mut input = String::new();
        for index in 0..count {
            input.push_str(&format!(
                "--- file\n+++ file\n@@ -1,3 +1,3 @@\n prefix\n-state {index}\n+state {}\n suffix\n",
                index + 1
            ));
        }
        parse_patch(&input).unwrap()
    }

    #[test]
    fn applies_patch_and_returns_structured_report() {
        let root = tempdir("apply-text");
        fs::write(root.join("file"), "one\ntwo\n").unwrap();
        let patch = parse_patch("--- file\n+++ file\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n").unwrap();

        let report = apply_patch(&root, &patch, &ApplyOptions::default()).unwrap();

        assert_eq!(fs::read_to_string(root.join("file")).unwrap(), "one\nTWO\n");
        assert_eq!(report.files.len(), 1);
        assert_eq!(report.files[0].path, PathBuf::from("file"));
        assert_eq!(report.files[0].hunks_applied, 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ordered_hunks_use_fast_apply_path() {
        let input = "--- big\n+++ big\n@@ -2,3 +2,4 @@\n line 2\n line 3\n+patched 1\n line 4\n@@ -6,3 +7,4 @@\n line 6\n line 7\n+patched 2\n line 8\n";
        let patch = parse_patch(input).unwrap();
        let original = (1..=10)
            .map(|line| format!("line {line}\n"))
            .collect::<Vec<_>>();
        let original = original.iter().map(String::as_str).collect::<Vec<_>>();

        let applied = apply_ordered_hunks_fast(
            &patch.files[0],
            false,
            &original,
            &ApplyOptions::default(),
            false,
        )
        .unwrap();

        assert_eq!(applied.hunk_reports.len(), 2);
        assert_eq!(applied.output[3], "patched 1\n");
        assert_eq!(applied.output[8], "patched 2\n");
    }

    #[test]
    fn affected_paths_uses_same_target_selection_as_apply() {
        let root = tempdir("affected-paths");
        fs::write(root.join("old"), "one\n").unwrap();
        let patch = parse_patch("--- old\n+++ new\n@@ -1 +1 @@\n-one\n+two\n").unwrap();

        let paths = crate::affected_paths(&root, &patch, &ApplyOptions::default()).unwrap();

        assert_eq!(paths, vec![PathBuf::from("old")]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn oversized_fuzz_is_bounded_by_hunk_context() {
        let root = tempdir("oversized-fuzz");
        fs::write(root.join("file"), "one\nactual\nthree\n").unwrap();
        let patch =
            parse_patch("--- file\n+++ file\n@@ -1,3 +1,3 @@\n one\n-expected\n+changed\n three\n")
                .unwrap();
        let options = ApplyOptions {
            max_fuzz: usize::MAX,
            dry_run: true,
            ..ApplyOptions::default()
        };

        let report = apply_patch(&root, &patch, &options).unwrap();

        assert_eq!(report.failed_hunks(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn large_disjoint_patch_set_is_parallel_candidate() {
        let root = tempdir("parallel-candidate");
        let mut input = String::new();
        for index in 0..PARALLEL_APPLY_MIN_FILES {
            fs::write(root.join(format!("file-{index}")), "old\n").unwrap();
            input.push_str(&format!(
                "--- file-{index}\n+++ file-{index}\n@@ -1 +1 @@\n-old\n+new\n"
            ));
        }
        let patch = parse_patch(&input).unwrap();

        let targets = parallel_apply_targets(&root, &patch, &ApplyOptions::default(), false)
            .unwrap()
            .unwrap();

        assert_eq!(targets.len(), PARALLEL_APPLY_MIN_FILES);
        assert_eq!(targets[0], PathBuf::from("file-0"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn repeated_file_patch_set_is_not_parallel_candidate() {
        let root = tempdir("parallel-repeat");
        fs::write(root.join("file"), "old\nmiddle\n").unwrap();
        let mut input = String::new();
        for _ in 0..PARALLEL_APPLY_MIN_FILES {
            input.push_str("--- file\n+++ file\n@@ -1,2 +1,2 @@\n-old\n+new\n middle\n");
        }
        let patch = parse_patch(&input).unwrap();

        let targets =
            parallel_apply_targets(&root, &patch, &ApplyOptions::default(), false).unwrap();

        assert!(targets.is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn repeated_file_patch_set_is_grouped_candidate() {
        let root = tempdir("grouped-candidate");
        fs::write(root.join("file"), "prefix\nstate 0\nsuffix\n").unwrap();
        let patch = dependent_series(GROUPED_APPLY_MIN_FILES);
        let policy = ApplyPathPolicy::new(&root).unwrap();

        let report = grouped_apply_patch(&root, &patch, &ApplyOptions::default(), &policy)
            .unwrap()
            .unwrap();

        assert_eq!(report.files.len(), GROUPED_APPLY_MIN_FILES);
        assert_eq!(report.files[0].path, PathBuf::from("file"));
        assert_eq!(
            fs::read_to_string(root.join("file")).unwrap(),
            format!("prefix\nstate {GROUPED_APPLY_MIN_FILES}\nsuffix\n")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn grouped_apply_does_not_run_for_dry_run() {
        let root = tempdir("grouped-dry-run");
        fs::write(root.join("file"), "prefix\nstate 0\nsuffix\n").unwrap();
        let patch = dependent_series(GROUPED_APPLY_MIN_FILES);
        let options = ApplyOptions {
            dry_run: true,
            ..ApplyOptions::default()
        };
        let policy = ApplyPathPolicy::new(&root).unwrap();

        let report = grouped_apply_patch(&root, &patch, &options, &policy).unwrap();

        assert!(report.is_none());
        assert_eq!(
            fs::read_to_string(root.join("file")).unwrap(),
            "prefix\nstate 0\nsuffix\n"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
