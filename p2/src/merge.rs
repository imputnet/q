//! `--merge` support: three-way merging of hunks that do not apply cleanly.

use crate::hunk::{MaterializedHunk, find_hunk_start, materialize_hunk};
use crate::{Hunk, HunkStatus, MergeMode};

pub(crate) struct AppliedMergeHunk {
    pub(crate) status: HunkStatus,
    pub(crate) expected_len: usize,
    pub(crate) replacement_len: usize,
    pub(crate) next_allowed_start: usize,
}

/// Merges `hunk` into `output` near `preferred`, writing conflict markers
/// where the file and the hunk disagree.
pub(crate) fn apply_merge_hunk(
    hunk: &Hunk,
    reverse: bool,
    output: &mut Vec<String>,
    preferred: usize,
    requested_line: usize,
    mode: MergeMode,
    ignore_whitespace: bool,
) -> AppliedMergeHunk {
    let materialized = materialize_hunk(hunk, reverse, 0, None);
    if let Some(already_applied) = already_applied_materialized_hunk(
        &materialized,
        output,
        preferred,
        requested_line,
        ignore_whitespace,
    ) {
        return already_applied;
    }

    let start = preferred.min(output.len());
    let end = merge_region_end(output, &materialized, start);
    let merge = merge_lines(
        &materialized.expected,
        &output[start..end],
        &materialized.replacement,
        mode,
    );
    let replacement_len = merge.lines.len();
    output.splice(start..end, merge.lines);
    let offset_ranges = |ranges: &[(usize, usize)]| {
        ranges
            .iter()
            .map(|(range_start, range_end)| (start + range_start + 1, start + range_end))
            .collect::<Vec<_>>()
    };
    let status = if merge.conflict {
        HunkStatus::Conflict {
            requested_line,
            start_line: start + merge.conflict_start + 1,
            end_line: start + merge.conflict_end,
            merged_ranges: offset_ranges(&merge.merged_ranges),
        }
    } else {
        let ranges = if merge.merged_ranges.is_empty() {
            vec![changed_output_range(start, &materialized)]
        } else {
            offset_ranges(&merge.merged_ranges)
        };
        HunkStatus::Merged {
            requested_line,
            ranges,
        }
    };
    AppliedMergeHunk {
        status,
        expected_len: end - start,
        replacement_len,
        next_allowed_start: start + replacement_len,
    }
}

/// Detects a hunk whose changes are already present in `output`.
pub(crate) fn already_applied_merge_hunk(
    hunk: &Hunk,
    reverse: bool,
    output: &[String],
    preferred: usize,
    requested_line: usize,
    ignore_whitespace: bool,
) -> Option<AppliedMergeHunk> {
    let materialized = materialize_hunk(hunk, reverse, 0, None);
    already_applied_materialized_hunk(
        &materialized,
        output,
        preferred,
        requested_line,
        ignore_whitespace,
    )
}

fn already_applied_materialized_hunk(
    materialized: &MaterializedHunk,
    output: &[String],
    preferred: usize,
    requested_line: usize,
    ignore_whitespace: bool,
) -> Option<AppliedMergeHunk> {
    if materialized.replacement.is_empty() {
        return None;
    }
    let preferred = preferred.min(output.len());
    let start = find_hunk_start(
        output,
        &materialized.replacement,
        preferred,
        0,
        ignore_whitespace,
    )?;
    if start != preferred {
        // Found elsewhere: only trust it if merging at the requested position
        // would not conflict.
        let merge_end = merge_region_end(output, materialized, preferred);
        let merge = merge_lines(
            &materialized.expected,
            &output[preferred..merge_end],
            &materialized.replacement,
            MergeMode::Merge,
        );
        if merge.conflict {
            return None;
        }
    }
    let (applied_line, end_line) = changed_output_range(start, materialized);
    Some(AppliedMergeHunk {
        status: HunkStatus::AlreadyApplied {
            requested_line,
            applied_line,
            end_line,
        },
        expected_len: 0,
        replacement_len: 0,
        next_allowed_start: start + materialized.replacement.len(),
    })
}

/// The 1-based line range of the changed (non-context) part of a hunk placed
/// at `start`.
fn changed_output_range(start: usize, materialized: &MaterializedHunk) -> (usize, usize) {
    let first = start + materialized.prefix_context + 1;
    let changed_len = materialized
        .replacement
        .len()
        .saturating_sub(materialized.prefix_context + materialized.suffix_context)
        .max(1);
    (first, first + changed_len - 1)
}

/// The end of the region of `output` starting at `start` that a merge should
/// replace: through the next occurrence of the hunk's last expected line.
fn merge_region_end(output: &[String], materialized: &MaterializedHunk, start: usize) -> usize {
    if materialized.prefix_context == 0 && materialized.suffix_context == 0 {
        return start;
    }
    let expected = &materialized.expected;
    let Some(last_expected) = expected.last() else {
        return start;
    };
    let min_end = start.saturating_add(expected.len()).min(output.len());
    output
        .iter()
        .enumerate()
        .skip(min_end.saturating_sub(1))
        .find(|(_, line)| *line == last_expected)
        .map_or(min_end, |(index, _)| index + 1)
}

struct LineMerge {
    lines: Vec<String>,
    conflict: bool,
    conflict_start: usize,
    conflict_end: usize,
    merged_ranges: Vec<(usize, usize)>,
}

/// Replacement of `base[start..end]`.
struct LineChange {
    start: usize,
    end: usize,
    replacement: Vec<String>,
}

/// Three-way merge of `ours` (the file) and `theirs` (the hunk's result)
/// against their common `base` (the hunk's expected lines).
fn merge_lines(base: &[String], ours: &[String], theirs: &[String], mode: MergeMode) -> LineMerge {
    let ours_changes = diff_changes(base, ours);
    let theirs_changes = diff_changes(base, theirs);
    let mut out = Vec::new();
    let mut cursor = 0;
    let mut ours_index = 0;
    let mut theirs_index = 0;
    let mut conflict = false;
    let mut conflict_report_start = None;
    let mut conflict_report_end = 0;
    let mut merged_ranges = Vec::new();

    loop {
        match (
            ours_changes.get(ours_index),
            theirs_changes.get(theirs_index),
        ) {
            (Some(ours_change), Some(theirs_change))
                if changes_overlap_or_share_insertion(ours_change, theirs_change) =>
            {
                let overlap_start = ours_change.start.min(theirs_change.start);
                let overlap_end = ours_change.end.max(theirs_change.end);
                append_base_until(&mut out, base, &mut cursor, overlap_start);
                let ours_side =
                    apply_change_to_base_range(base, overlap_start, overlap_end, ours_change);
                let theirs_side =
                    apply_change_to_base_range(base, overlap_start, overlap_end, theirs_change);
                // Plain merge mode moves lines both sides agree on out of the
                // conflict markers; diff3 mode shows the sides verbatim.
                let (prefix, ours_side, theirs_side, suffix) = if mode == MergeMode::Merge {
                    trim_common_lines(&ours_side, &theirs_side)
                } else {
                    (&[][..], &ours_side[..], &theirs_side[..], &[][..])
                };
                conflict = true;
                out.extend_from_slice(prefix);
                conflict_report_start.get_or_insert(out.len());
                out.extend(render_merge_conflict(
                    ours_side,
                    &base[overlap_start..overlap_end],
                    theirs_side,
                    mode,
                ));
                conflict_report_end = out.len();
                out.extend_from_slice(suffix);
                cursor = overlap_end;
                ours_index += 1;
                theirs_index += 1;
            }
            // Otherwise take whichever change comes first; ties go to theirs.
            (ours_change, Some(theirs_change))
                if ours_change.is_none_or(|ours| ours.start >= theirs_change.start) =>
            {
                let range_start = out.len() + theirs_change.start.saturating_sub(cursor);
                append_change(&mut out, base, &mut cursor, theirs_change);
                if range_start < out.len() {
                    merged_ranges.push((range_start, out.len()));
                }
                theirs_index += 1;
            }
            (Some(ours_change), _) => {
                append_change(&mut out, base, &mut cursor, ours_change);
                ours_index += 1;
            }
            (None, _) => break,
        }
    }
    append_base_until(&mut out, base, &mut cursor, base.len());
    let conflict_start = conflict_report_start.unwrap_or(0);
    let conflict_end = conflict_report_end.max(conflict_start + 1);
    LineMerge {
        lines: out,
        conflict,
        conflict_start,
        conflict_end,
        merged_ranges,
    }
}

fn append_base_until(out: &mut Vec<String>, base: &[String], cursor: &mut usize, end: usize) {
    out.extend_from_slice(&base[*cursor..end]);
    *cursor = end;
}

fn append_change(out: &mut Vec<String>, base: &[String], cursor: &mut usize, change: &LineChange) {
    append_base_until(out, base, cursor, change.start);
    out.extend_from_slice(&change.replacement);
    *cursor = change.end;
}

fn apply_change_to_base_range(
    base: &[String],
    start: usize,
    end: usize,
    change: &LineChange,
) -> Vec<String> {
    [
        &base[start..change.start],
        &change.replacement[..],
        &base[change.end..end],
    ]
    .concat()
}

fn changes_overlap_or_share_insertion(left: &LineChange, right: &LineChange) -> bool {
    let left_consumes_base = left.start < left.end;
    let right_consumes_base = right.start < right.end;
    if !left_consumes_base && !right_consumes_base {
        return left.start == right.start;
    }
    if left.start < right.end && right.start < left.end {
        return true;
    }
    left.end == right.start || right.end == left.start
}

/// The changes that turn `base` into `target`, from a longest common
/// subsequence alignment.
fn diff_changes(base: &[String], target: &[String]) -> Vec<LineChange> {
    let table = lcs_table(base, target);
    let mut changes = Vec::new();
    let mut pending: Option<LineChange> = None;
    let mut i = 0;
    let mut j = 0;
    while i < base.len() || j < target.len() {
        if i < base.len() && j < target.len() && base[i] == target[j] {
            changes.extend(pending.take());
            i += 1;
            j += 1;
            continue;
        }
        let change = pending.get_or_insert_with(|| LineChange {
            start: i,
            end: i,
            replacement: Vec::new(),
        });
        if j < target.len() && (i == base.len() || table[i][j + 1] >= table[i + 1][j]) {
            change.replacement.push(target[j].clone());
            j += 1;
        } else {
            change.end = i + 1;
            i += 1;
        }
    }
    changes.extend(pending);
    changes
}

fn lcs_table(left: &[String], right: &[String]) -> Vec<Vec<usize>> {
    let mut table = vec![vec![0; right.len() + 1]; left.len() + 1];
    for i in (0..left.len()).rev() {
        for j in (0..right.len()).rev() {
            table[i][j] = if left[i] == right[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    table
}

fn render_merge_conflict(
    current: &[String],
    original: &[String],
    replacement: &[String],
    mode: MergeMode,
) -> Vec<String> {
    let mut out = vec!["<<<<<<<\n".to_string()];
    out.extend_from_slice(current);
    if mode == MergeMode::Diff3 {
        out.push("|||||||\n".to_string());
        out.extend_from_slice(original);
    }
    out.push("=======\n".to_string());
    out.extend_from_slice(replacement);
    out.push(">>>>>>>\n".to_string());
    out
}

/// Splits two conflict sides into (common prefix, ours, theirs, common suffix).
fn trim_common_lines<'a>(
    ours: &'a [String],
    theirs: &'a [String],
) -> (&'a [String], &'a [String], &'a [String], &'a [String]) {
    let prefix_len = ours
        .iter()
        .zip(theirs)
        .take_while(|(ours, theirs)| ours == theirs)
        .count();
    let suffix_len = ours[prefix_len..]
        .iter()
        .rev()
        .zip(theirs[prefix_len..].iter().rev())
        .take_while(|(ours, theirs)| ours == theirs)
        .count();
    (
        &ours[..prefix_len],
        &ours[prefix_len..ours.len() - suffix_len],
        &theirs[prefix_len..theirs.len() - suffix_len],
        &ours[ours.len() - suffix_len..],
    )
}
