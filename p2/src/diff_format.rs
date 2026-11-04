//! Context diff syntax shared by the parser and the reject writer.

use std::fmt::Write;

pub(crate) const CONTEXT_HUNK_SEPARATOR: &str = "***************";
pub(crate) const CONTEXT_OLD_RANGE_PREFIX: &str = "*** ";
pub(crate) const CONTEXT_OLD_RANGE_SUFFIX: &str = " ****";
pub(crate) const CONTEXT_NEW_RANGE_PREFIX: &str = "--- ";
pub(crate) const CONTEXT_NEW_RANGE_SUFFIX: &str = " ----";

/// Writes a context range: `start` or `start,end`.
pub(crate) fn push_context_range(out: &mut String, start: usize, count: usize) {
    if count == 1 {
        let _ = write!(out, "{start}");
    } else {
        let _ = write!(out, "{start},{}", start + count - 1);
    }
}

/// Writes a unified range: `start` or `start,count`.
pub(crate) fn push_unified_range(out: &mut String, start: usize, count: usize) {
    if count == 1 {
        let _ = write!(out, "{start}");
    } else {
        let _ = write!(out, "{start},{count}");
    }
}
