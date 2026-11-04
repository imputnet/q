//! Diff header timestamps, used for `--set-time`/`--set-utc` and for
//! recognizing epoch-dated deletions.

use std::fs;
use std::path::Path;
use std::str::FromStr;

use filetime::FileTime;

use crate::{ApplyOptions, FilePatch, Result, io_error};

#[derive(Clone, Copy)]
pub(crate) enum TimestampMode {
    Utc,
    Local,
}

impl TimestampMode {
    pub(crate) fn from_options(options: &ApplyOptions) -> Option<Self> {
        if options.set_utc {
            Some(Self::Utc)
        } else if options.set_time {
            Some(Self::Local)
        } else {
            None
        }
    }
}

fn source_label(patch: &FilePatch, reverse: bool) -> Option<&str> {
    if reverse {
        patch.new_label.as_deref()
    } else {
        patch.old_label.as_deref()
    }
}

fn target_label(patch: &FilePatch, reverse: bool) -> Option<&str> {
    if reverse {
        patch.old_label.as_deref()
    } else {
        patch.new_label.as_deref()
    }
}

/// GNU diff marks a deleted file with an epoch timestamp on the new side.
pub(crate) fn has_deletion_timestamp(patch: &FilePatch, reverse: bool) -> bool {
    target_label(patch, reverse)
        .and_then(|label| parse_header_timestamp(label, TimestampMode::Local))
        .is_some_and(|timestamp| timestamp.seconds() == 0)
}

pub(crate) fn source_timestamp(
    patch: &FilePatch,
    reverse: bool,
    mode: Option<TimestampMode>,
) -> Option<FileTime> {
    parse_header_timestamp(source_label(patch, reverse)?, mode?)
}

pub(crate) fn target_timestamp(
    patch: &FilePatch,
    reverse: bool,
    mode: Option<TimestampMode>,
) -> Option<FileTime> {
    parse_header_timestamp(target_label(patch, reverse)?, mode?)
}

pub(crate) fn source_timestamp_matches(
    path: &Path,
    existed: bool,
    timestamp: Option<FileTime>,
) -> Result<bool> {
    let Some(timestamp) = timestamp.filter(|_| existed) else {
        return Ok(true);
    };
    let metadata = fs::metadata(path).map_err(io_error(path))?;
    let actual = FileTime::from_last_modification_time(&metadata);
    Ok(actual.seconds() == timestamp.seconds() && actual.nanoseconds() == timestamp.nanoseconds())
}

struct DateTime {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
}

impl DateTime {
    fn utc_seconds(&self) -> Option<i64> {
        days_from_civil(self.year, self.month, self.day)
            .checked_mul(86_400)?
            .checked_add(
                i64::from(self.hour) * 3_600 + i64::from(self.minute) * 60 + i64::from(self.second),
            )
    }

    fn local_seconds(&self) -> Option<i64> {
        // SAFETY: `tm` is plain integers plus, on some platforms, a zone
        // pointer, for all of which zero is a valid value.
        let mut time: libc::tm = unsafe { std::mem::zeroed() };
        time.tm_sec = i32::try_from(self.second).ok()?;
        time.tm_min = i32::try_from(self.minute).ok()?;
        time.tm_hour = i32::try_from(self.hour).ok()?;
        time.tm_mday = i32::try_from(self.day).ok()?;
        time.tm_mon = i32::try_from(self.month).ok()? - 1;
        time.tm_year = self.year.checked_sub(1900)?;
        time.tm_isdst = -1;
        // SAFETY: `time` is a fully initialized `tm` that mktime may normalize in place.
        let seconds = unsafe { mktime(&mut time) };
        (seconds != -1).then_some(seconds as i64)
    }
}

#[cfg(unix)]
use libc::mktime;

#[cfg(windows)]
unsafe extern "C" {
    #[link_name = "_mktime64"]
    fn mktime(time: *mut libc::tm) -> i64;
}

/// Parses a `YYYY-MM-DD HH:MM:SS[.fraction][ zone]` header label.
pub(crate) fn parse_header_timestamp(label: &str, mode: TimestampMode) -> Option<FileTime> {
    let timestamp = label.trim_start();
    let datetime = timestamp.get(..19)?;
    let bytes = datetime.as_bytes();
    if bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b' '
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return None;
    }
    let datetime = DateTime {
        year: parse_digits(&datetime[0..4])?,
        month: parse_digits(&datetime[5..7])?,
        day: parse_digits(&datetime[8..10])?,
        hour: parse_digits(&datetime[11..13])?,
        minute: parse_digits(&datetime[14..16])?,
        second: parse_digits(&datetime[17..19])?,
    };
    if !(1..=12).contains(&datetime.month)
        || !(1..=31).contains(&datetime.day)
        || datetime.hour > 23
        || datetime.minute > 59
        || datetime.second > 60
    {
        return None;
    }
    let (nanoseconds, timezone_offset) = parse_timestamp_suffix(&timestamp[19..])?;
    let seconds = match (timezone_offset, mode) {
        (Some(offset), _) => datetime.utc_seconds()?.checked_sub(offset)?,
        (None, TimestampMode::Utc) => datetime.utc_seconds()?,
        (None, TimestampMode::Local) => datetime.local_seconds()?,
    };
    Some(FileTime::from_unix_time(seconds, nanoseconds))
}

/// Parses the optional `.fraction` and timezone that follow the seconds.
fn parse_timestamp_suffix(suffix: &str) -> Option<(u32, Option<i64>)> {
    let mut rest = suffix;
    let mut nanoseconds = 0;
    if let Some(fraction) = rest.strip_prefix('.') {
        let digit_count = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if digit_count == 0 {
            return None;
        }
        let parsed_digits = digit_count.min(9);
        nanoseconds = fraction
            .bytes()
            .take(parsed_digits)
            .fold(0, |value, byte| value * 10 + u32::from(byte - b'0'))
            * 10u32.pow((9 - parsed_digits) as u32);
        rest = &fraction[digit_count..];
    }

    let rest = rest.trim_start();
    let timezone_offset = if rest.is_empty() {
        None
    } else {
        Some(parse_timezone_offset(rest)?)
    };
    Some((nanoseconds, timezone_offset))
}

fn parse_timezone_offset(value: &str) -> Option<i64> {
    if ["UTC", "GMT", "UT", "Z"]
        .iter()
        .any(|zone| value.eq_ignore_ascii_case(zone))
    {
        return Some(0);
    }

    let sign = match value.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let rest = &value[1..];
    let digits = |range: std::ops::Range<usize>| {
        rest.as_bytes()
            .get(range)
            .is_some_and(|bytes| bytes.iter().all(u8::is_ascii_digit))
    };
    let (hours, minutes) = if digits(0..4) {
        (&rest[0..2], &rest[2..4])
    } else if digits(0..2) && rest.as_bytes().get(2) == Some(&b':') && digits(3..5) {
        (&rest[0..2], &rest[3..5])
    } else {
        return None;
    };
    let hours: i64 = parse_digits(hours)?;
    let minutes: i64 = parse_digits(minutes)?;
    if hours > 25 || minutes > 59 {
        return None;
    }
    Some(sign * (hours * 3_600 + minutes * 60))
}

/// Parses a field made only of ASCII digits (no sign).
fn parse_digits<T: FromStr>(value: &str) -> Option<T> {
    if value.bytes().all(|byte| byte.is_ascii_digit()) {
        value.parse().ok()
    } else {
        None
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = year - i32::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = month as i32;
    let day = day as i32;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    i64::from(era * 146_097 + day_of_era - 719_468)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_patch;

    #[test]
    fn epoch_timestamp_marks_deletion_with_timezone() {
        let patch = parse_patch(
            "--- a/deleted\n+++ b/deleted\t1970-01-01 01:00:00.000000000 +0100\n@@ -1 +0,0 @@\n-data\n",
        )
        .unwrap();
        assert!(has_deletion_timestamp(&patch.files[0], false));
    }

    #[test]
    fn parses_fractional_timestamp_with_timezone_offset() {
        let timestamp =
            parse_header_timestamp("2009-03-14 01:00:00.123456789 +0100", TimestampMode::Utc)
                .unwrap();
        assert_eq!(timestamp.seconds(), 1_236_988_800);
        assert_eq!(timestamp.nanoseconds(), 123_456_789);
    }
}
