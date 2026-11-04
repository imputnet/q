//! Locating hunks in file contents, with fuzz, offsets and whitespace rules.

use crate::{Hunk, HunkLine};

/// The line a hunk expects to start at on the side being patched.
pub(crate) fn hunk_start(hunk: &Hunk, reverse: bool) -> usize {
    if reverse {
        hunk.new_start
    } else {
        hunk.old_start
    }
}

/// Number of context lines before the first change and after the last change.
fn context_counts(hunk: &Hunk) -> (usize, usize) {
    let is_change = |line: &HunkLine| !matches!(line, HunkLine::Context(_));
    match (
        hunk.lines.iter().position(is_change),
        hunk.lines.iter().rposition(is_change),
    ) {
        (Some(first), Some(last)) => (first, hunk.lines.len() - last - 1),
        _ => (0, 0),
    }
}

/// The most fuzz that can be applied before a hunk runs out of context.
pub(crate) fn hunk_fuzz_limit(hunk: &Hunk) -> usize {
    let (prefix, suffix) = context_counts(hunk);
    prefix.max(suffix)
}

/// The fuzz GNU patch reports for a hunk with uneven context
/// (`locate_hunk` in GNU patch). Less trailing than leading context means the
/// hunk belongs at the end of the file; less leading context means it belongs
/// at the start, but only if it claims to start at line 1. Matching anywhere
/// else counts the difference as fuzz.
pub(crate) fn estimate_fuzz(
    prefix_context: usize,
    suffix_context: usize,
    claims_start: bool,
    at_start: bool,
    at_end: bool,
) -> usize {
    if prefix_context == 0 || suffix_context == 0 {
        return 0;
    }
    if prefix_context > suffix_context && !at_end {
        prefix_context - suffix_context
    } else if prefix_context < suffix_context && claims_start && !at_start {
        suffix_context - prefix_context
    } else {
        0
    }
}

/// A hunk's expected and replacement lines after dropping `fuzz` context lines
/// from each end.
pub(crate) struct MaterializedHunk {
    pub(crate) expected: Vec<String>,
    pub(crate) replacement: Vec<String>,
    pub(crate) prefix_context: usize,
    pub(crate) suffix_context: usize,
    pub(crate) prefix_stripped: usize,
}

pub(crate) fn materialize_hunk(
    hunk: &Hunk,
    reverse: bool,
    fuzz: usize,
    ifdef_symbol: Option<&str>,
) -> MaterializedHunk {
    let (prefix_context, suffix_context) = context_counts(hunk);
    let prefix_stripped = fuzz.min(prefix_context);
    let kept = &hunk.lines[prefix_stripped..hunk.lines.len() - fuzz.min(suffix_context)];

    let (expected_capacity, replacement_capacity) = if reverse {
        (hunk.new_count, hunk.old_count)
    } else {
        (hunk.old_count, hunk.new_count)
    };
    let mut expected = Vec::with_capacity(expected_capacity);
    let mut replacement = Vec::with_capacity(replacement_capacity);
    for line in kept {
        match (line, reverse) {
            (HunkLine::Context(value), _) => {
                expected.push(value.clone());
                replacement.push(value.clone());
            }
            (HunkLine::Remove(value), false) | (HunkLine::Add(value), true) => {
                expected.push(value.clone());
            }
            (HunkLine::Add(value), false) | (HunkLine::Remove(value), true) => {
                replacement.push(value.clone());
            }
        }
    }
    if let Some(symbol) = ifdef_symbol {
        replacement = render_ifdef_replacement(kept, reverse, symbol);
    }

    MaterializedHunk {
        expected,
        replacement,
        prefix_context,
        suffix_context,
        prefix_stripped,
    }
}

/// Renders a hunk as `#ifdef`/`#ifndef` blocks that keep both sides (`-D`).
fn render_ifdef_replacement(lines: &[HunkLine], reverse: bool, symbol: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut old = Vec::new();
    let mut new = Vec::new();

    for line in lines {
        match (line, reverse) {
            (HunkLine::Context(value), _) => {
                flush_ifdef_block(&mut out, &mut old, &mut new, symbol);
                out.push(value.clone());
            }
            (HunkLine::Remove(value), false) | (HunkLine::Add(value), true) => {
                old.push(value.clone());
            }
            (HunkLine::Add(value), false) | (HunkLine::Remove(value), true) => {
                new.push(value.clone());
            }
        }
    }
    flush_ifdef_block(&mut out, &mut old, &mut new, symbol);
    out
}

fn flush_ifdef_block(
    out: &mut Vec<String>,
    old: &mut Vec<String>,
    new: &mut Vec<String>,
    symbol: &str,
) {
    match (old.is_empty(), new.is_empty()) {
        (true, true) => return,
        (true, false) => {
            out.push(format!("#ifdef {symbol}\n"));
            out.append(new);
        }
        (false, true) => {
            out.push(format!("#ifndef {symbol}\n"));
            out.append(old);
        }
        (false, false) => {
            out.push(format!("#ifndef {symbol}\n"));
            out.append(old);
            out.push("#else\n".to_string());
            out.append(new);
        }
    }
    out.push("#endif\n".to_string());
}

/// The lines a hunk produces when applied without fuzz.
pub(crate) fn unfuzzed_replacement_lines(
    hunk: &Hunk,
    reverse: bool,
) -> impl Iterator<Item = &String> {
    hunk.lines
        .iter()
        .filter_map(move |line| match (line, reverse) {
            (HunkLine::Context(value), _)
            | (HunkLine::Add(value), false)
            | (HunkLine::Remove(value), true) => Some(value),
            (HunkLine::Remove(_), false) | (HunkLine::Add(_), true) => None,
        })
}

pub(crate) fn unfuzzed_replacement(hunk: &Hunk, reverse: bool) -> Vec<String> {
    let mut replacement = Vec::with_capacity(if reverse {
        hunk.old_count
    } else {
        hunk.new_count
    });
    replacement.extend(unfuzzed_replacement_lines(hunk, reverse).cloned());
    replacement
}

/// A match found at a hunk's exact requested position (see `match_ordered_hunk`).
pub(crate) struct OrderedMatch {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) fuzz: usize,
    pub(crate) prefix_stripped: usize,
    pub(crate) prefix_context: usize,
    pub(crate) suffix_context: usize,
    pub(crate) replacement_len: usize,
    /// The replacement lines when fuzz was needed; `None` means the hunk's own
    /// lines (see `unfuzzed_replacement_lines`) apply as-is.
    pub(crate) fuzzed_replacement: Option<Vec<String>>,
}

/// Matches a hunk at exactly `start`, trying increasing fuzz but never
/// searching other offsets. This is the fast path for well-formed patch series.
pub(crate) fn match_ordered_hunk<L: AsRef<str>>(
    hunk: &Hunk,
    reverse: bool,
    lines: &[L],
    start: usize,
    max_fuzz: usize,
) -> Option<OrderedMatch> {
    if let Some(matched) = match_unfuzzed_hunk(hunk, reverse, lines, start) {
        return Some(matched);
    }
    for fuzz in 1..=max_fuzz.min(hunk_fuzz_limit(hunk)) {
        let materialized = materialize_hunk(hunk, reverse, fuzz, None);
        if materialized.expected.is_empty() {
            continue;
        }
        let match_start = start.checked_add(materialized.prefix_stripped)?;
        let end = match_start.checked_add(materialized.expected.len())?;
        if end <= lines.len()
            && lines[match_start..end]
                .iter()
                .map(|line| line.as_ref())
                .eq(materialized.expected.iter().map(String::as_str))
        {
            return Some(OrderedMatch {
                start: match_start,
                end,
                fuzz,
                prefix_stripped: materialized.prefix_stripped,
                prefix_context: materialized.prefix_context,
                suffix_context: materialized.suffix_context,
                replacement_len: materialized.replacement.len(),
                fuzzed_replacement: Some(materialized.replacement),
            });
        }
    }
    None
}

/// Compares hunk lines against `lines` in place, without materializing them.
fn match_unfuzzed_hunk<L: AsRef<str>>(
    hunk: &Hunk,
    reverse: bool,
    lines: &[L],
    start: usize,
) -> Option<OrderedMatch> {
    let mut expected_len = 0;
    let mut replacement_len = 0;
    for line in &hunk.lines {
        match (line, reverse) {
            (HunkLine::Context(value), _) => {
                if lines.get(start + expected_len)?.as_ref() != value.as_str() {
                    return None;
                }
                expected_len += 1;
                replacement_len += 1;
            }
            (HunkLine::Remove(value), false) | (HunkLine::Add(value), true) => {
                if lines.get(start + expected_len)?.as_ref() != value.as_str() {
                    return None;
                }
                expected_len += 1;
            }
            (HunkLine::Add(_), false) | (HunkLine::Remove(_), true) => replacement_len += 1,
        }
    }

    let (prefix_context, suffix_context) = context_counts(hunk);
    Some(OrderedMatch {
        start,
        end: start + expected_len,
        fuzz: 0,
        prefix_stripped: 0,
        prefix_context,
        suffix_context,
        replacement_len,
        fuzzed_replacement: None,
    })
}

/// A match found by searching around the requested position.
pub(crate) struct MatchedHunk {
    pub(crate) start: usize,
    pub(crate) reported_line: usize,
    pub(crate) expected_len: usize,
    pub(crate) replacement: Vec<String>,
    pub(crate) fuzz: usize,
    pub(crate) prefix_context: usize,
    pub(crate) suffix_context: usize,
}

pub(crate) struct HunkMatchOptions<'a> {
    pub(crate) ignore_whitespace: bool,
    pub(crate) max_fuzz: usize,
    pub(crate) ifdef_symbol: Option<&'a str>,
}

/// Finds the nearest position where the hunk applies, preferring less fuzz.
pub(crate) fn find_hunk_match(
    hunk: &Hunk,
    reverse: bool,
    lines: &[String],
    preferred: usize,
    lowest_allowed_start: usize,
    options: HunkMatchOptions<'_>,
) -> Option<MatchedHunk> {
    for fuzz in 0..=options.max_fuzz.min(hunk_fuzz_limit(hunk)) {
        let materialized = materialize_hunk(hunk, reverse, fuzz, options.ifdef_symbol);
        if materialized.expected.is_empty() && fuzz > 0 {
            continue;
        }
        if let Some(start) = find_hunk_start(
            lines,
            &materialized.expected,
            preferred,
            lowest_allowed_start,
            options.ignore_whitespace,
        ) {
            return Some(MatchedHunk {
                start,
                reported_line: start + 1 - materialized.prefix_stripped.min(start),
                expected_len: materialized.expected.len(),
                replacement: materialized.replacement,
                fuzz,
                prefix_context: materialized.prefix_context,
                suffix_context: materialized.suffix_context,
            });
        }
    }
    None
}

/// Searches outward from `preferred`, alternating forward and backward, for
/// where `expected` occurs in `lines` at or after `lowest_allowed_start`.
pub(crate) fn find_hunk_start(
    lines: &[String],
    expected: &[String],
    preferred: usize,
    lowest_allowed_start: usize,
    ignore_whitespace: bool,
) -> Option<usize> {
    if expected.is_empty() {
        return Some(preferred.min(lines.len()).max(lowest_allowed_start));
    }
    let max_start = lines.len().checked_sub(expected.len())?;
    if lowest_allowed_start > max_start {
        return None;
    }
    let matches_at = |start: usize| {
        start <= max_start
            && lines_match(
                &lines[start..start + expected.len()],
                expected,
                ignore_whitespace,
            )
    };

    if preferred >= lowest_allowed_start && matches_at(preferred) {
        return Some(preferred);
    }
    for offset in 1..=max_start.max(preferred) {
        // Only the backward search is bounded by `lowest_allowed_start`.
        if let Some(forward) = preferred.checked_add(offset)
            && matches_at(forward)
        {
            return Some(forward);
        }
        if let Some(backward) = preferred.checked_sub(offset)
            && backward >= lowest_allowed_start
            && matches_at(backward)
        {
            return Some(backward);
        }
    }
    None
}

fn lines_match(lines: &[String], expected: &[String], ignore_whitespace: bool) -> bool {
    if !ignore_whitespace {
        return lines == expected;
    }
    lines
        .iter()
        .zip(expected)
        .all(|(line, expected)| line_matches_ignoring_whitespace(line, expected))
}

/// `-l` matching: runs of blanks compare equal and trailing blanks are ignored,
/// but line endings must agree.
fn line_matches_ignoring_whitespace(line: &str, expected: &str) -> bool {
    let (line_body, line_ending) = split_line_ending(line);
    let (expected_body, expected_ending) = split_line_ending(expected);
    if line_ending != expected_ending {
        return false;
    }
    let is_blank = |byte: &u8| matches!(byte, b' ' | b'\t');
    let mut line_bytes = trim_trailing_blanks(line_body).bytes().peekable();
    let mut expected_bytes = trim_trailing_blanks(expected_body).bytes().peekable();

    loop {
        match (line_bytes.peek().copied(), expected_bytes.peek().copied()) {
            (None, None) => return true,
            (Some(line), Some(expected)) if is_blank(&line) && is_blank(&expected) => {
                while line_bytes.next_if(is_blank).is_some() {}
                while expected_bytes.next_if(is_blank).is_some() {}
            }
            (Some(line), Some(expected)) if line == expected => {
                line_bytes.next();
                expected_bytes.next();
            }
            _ => return false,
        }
    }
}

fn split_line_ending(line: &str) -> (&str, &str) {
    if let Some(body) = line.strip_suffix("\r\n") {
        (body, "\r\n")
    } else if let Some(body) = line.strip_suffix('\n') {
        (body, "\n")
    } else {
        (line, "")
    }
}

fn trim_trailing_blanks(line: &str) -> &str {
    line.trim_end_matches([' ', '\t'])
}

/// Whether a failed hunk would have matched if CRLF line endings were ignored,
/// so the failure can be reported as "different line endings".
pub(crate) fn hunk_matches_after_crlf_normalization(
    hunk: &Hunk,
    reverse: bool,
    output: &[String],
    preferred: usize,
    ignore_whitespace: bool,
) -> bool {
    let materialized = materialize_hunk(hunk, reverse, 0, None);
    let expected = materialized.expected;
    if expected.is_empty() || expected.len() > output.len() {
        return false;
    }
    if let Some(start) = find_hunk_start(output, &expected, preferred, 0, ignore_whitespace) {
        return output[start..start + expected.len()] != expected;
    }
    let normalize = |lines: &[String]| {
        lines
            .iter()
            .map(|line| line.replace("\r\n", "\n"))
            .collect::<Vec<_>>()
    };
    find_hunk_start(
        &normalize(output),
        &normalize(&expected),
        preferred,
        0,
        ignore_whitespace,
    )
    .is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn finds_shifted_hunk_after_preferred_location() {
        let lines_ = lines(&["x\n", "a\n", "b\n", "c\n"]);
        let expected = lines(&["a\n", "b\n"]);
        assert_eq!(find_hunk_start(&lines_, &expected, 0, 0, false), Some(1));
    }

    #[test]
    fn finds_hunk_with_matching_blank_runs_when_ignoring_whitespace() {
        let lines_ = lines(&["alpha\t\tbeta\n", "gamma\n"]);
        let expected = lines(&["alpha beta  \n", "gamma\n"]);
        assert_eq!(find_hunk_start(&lines_, &expected, 0, 0, true), Some(0));
    }

    #[test]
    fn ignore_whitespace_does_not_match_missing_blank_runs() {
        let lines_ = lines(&["alphabeta\n"]);
        let expected = lines(&["alpha beta\n"]);
        assert_eq!(find_hunk_start(&lines_, &expected, 0, 0, true), None);
    }
}
