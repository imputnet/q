//! Rendering of rejected hunks for `.rej` files.

use crate::diff_format::{
    CONTEXT_HUNK_SEPARATOR, CONTEXT_NEW_RANGE_PREFIX, CONTEXT_NEW_RANGE_SUFFIX,
    CONTEXT_OLD_RANGE_PREFIX, CONTEXT_OLD_RANGE_SUFFIX, push_context_range, push_unified_range,
};
use crate::{FilePatch, Hunk, HunkLine, PatchFormat};

/// Renders `hunk` as a reject in `format`, defaulting to the patch's own format.
pub(crate) fn render_reject(patch: &FilePatch, hunk: &Hunk, format: Option<PatchFormat>) -> String {
    let mut out = String::new();
    if let Some(index) = &patch.index {
        out.push_str(index);
    }
    if format.unwrap_or(patch.format) == PatchFormat::Context {
        push_file_headers(&mut out, patch, "*** ", "--- ");
        render_context_hunk(&mut out, hunk);
    } else {
        push_file_headers(&mut out, patch, "--- ", "+++ ");
        render_unified_hunk(&mut out, hunk);
    }
    out
}

fn push_file_headers(out: &mut String, patch: &FilePatch, old_prefix: &str, new_prefix: &str) {
    for (prefix, path, label) in [
        (old_prefix, &patch.old_path, &patch.old_label),
        (new_prefix, &patch.new_path, &patch.new_label),
    ] {
        out.push_str(prefix);
        out.push_str(path.label());
        if let Some(label) = label {
            out.push_str(label);
        }
        out.push('\n');
    }
}

fn render_unified_hunk(out: &mut String, hunk: &Hunk) {
    out.push_str("@@ -");
    push_unified_range(out, hunk.old_start, hunk.old_count);
    out.push_str(" +");
    push_unified_range(out, hunk.new_start, hunk.new_count);
    out.push_str(" @@");
    if let Some(section) = &hunk.section {
        out.push_str(section);
    }
    out.push('\n');
    for line in &hunk.lines {
        let (tag, value) = match line {
            HunkLine::Context(value) => (' ', value),
            HunkLine::Remove(value) => ('-', value),
            HunkLine::Add(value) => ('+', value),
        };
        out.push(tag);
        out.push_str(value);
    }
}

fn render_context_hunk(out: &mut String, hunk: &Hunk) {
    out.push_str(CONTEXT_HUNK_SEPARATOR);
    if let Some(section) = &hunk.section {
        out.push_str(section);
    }
    out.push('\n');

    // Lines on a side that also has changes on the other side are "changed" (`!`).
    let has_add = hunk
        .lines
        .iter()
        .any(|line| matches!(line, HunkLine::Add(_)));
    let has_remove = hunk
        .lines
        .iter()
        .any(|line| matches!(line, HunkLine::Remove(_)));

    out.push_str(CONTEXT_OLD_RANGE_PREFIX);
    push_context_range(out, hunk.old_start, hunk.old_count);
    out.push_str(CONTEXT_OLD_RANGE_SUFFIX);
    out.push('\n');
    for line in &hunk.lines {
        let (tag, value) = match line {
            HunkLine::Context(value) => ("  ", value),
            HunkLine::Remove(value) => (if has_add { "! " } else { "- " }, value),
            HunkLine::Add(_) => continue,
        };
        out.push_str(tag);
        out.push_str(value);
    }

    out.push_str(CONTEXT_NEW_RANGE_PREFIX);
    push_context_range(out, hunk.new_start, hunk.new_count);
    out.push_str(CONTEXT_NEW_RANGE_SUFFIX);
    out.push('\n');
    for line in &hunk.lines {
        let (tag, value) = match line {
            HunkLine::Context(value) => ("  ", value),
            HunkLine::Add(value) => (if has_remove { "! " } else { "+ " }, value),
            HunkLine::Remove(_) => continue,
        };
        out.push_str(tag);
        out.push_str(value);
    }
}
