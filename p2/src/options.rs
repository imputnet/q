//! Parsing of the GNU patch options that affect how a patch applies, for
//! callers that embed p2 instead of running the binary.

use std::fmt;

use crate::{ApplyOptions, MergeMode, PatchFormat, ReadOnlyBehavior};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedApplyOptions {
    pub options: ApplyOptions,
    pub force_format: Option<PatchFormat>,
    pub binary: bool,
    pub quiet: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOptionParseError {
    MissingValue { option: String },
    InvalidValue { option: String, value: String },
    Unsupported { option: String },
}

impl fmt::Display for ApplyOptionParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingValue { option } => write!(f, "{option} requires a value"),
            Self::InvalidValue { option, value } => {
                write!(f, "invalid value {value:?} for {option}")
            }
            Self::Unsupported { option } => write!(f, "unsupported patch option {option}"),
        }
    }
}

impl std::error::Error for ApplyOptionParseError {}

type ParseResult<T> = std::result::Result<T, ApplyOptionParseError>;

pub fn parse_apply_options<I, S>(args: I) -> ParseResult<ParsedApplyOptions>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut parsed = ParsedApplyOptions::default();
    let options = &mut parsed.options;
    let mut args = args.into_iter().map(Into::into).peekable();
    while let Some(arg) = args.next() {
        let mut next_value = || {
            args.next()
                .ok_or_else(|| ApplyOptionParseError::MissingValue {
                    option: arg.clone(),
                })
        };
        match arg.as_str() {
            "-R" | "--reverse" => options.reverse = true,
            "-E" | "--remove-empty-files" => options.remove_empty = true,
            "-l" | "--ignore-whitespace" => options.ignore_whitespace = true,
            "--dry-run" => options.dry_run = true,
            "-m" | "--merge" => {
                // The mode is optional; only consume the next argument if it is one.
                options.merge = match args.next_if(|next| MergeMode::from_name(next).is_some()) {
                    Some(mode) => parse_value(&arg, &mode, MergeMode::from_name)?,
                    None => MergeMode::Merge,
                };
            }
            "-N" | "--forward" => {}
            "-n" | "--normal" => parsed.force_format = Some(PatchFormat::Normal),
            "-e" | "--ed" => parsed.force_format = Some(PatchFormat::Ed),
            "-u" | "--unified" => parsed.force_format = Some(PatchFormat::Unified),
            "-c" | "--context" => parsed.force_format = Some(PatchFormat::Context),
            "-T" | "--set-time" => options.set_time = true,
            "-Z" | "--set-utc" => options.set_utc = true,
            "--follow-symlinks" => options.follow_symlinks = true,
            "--binary" => parsed.binary = true,
            "-f" | "--force" => options.force = true,
            "-t" | "--batch" => options.batch = true,
            "-s" | "--quiet" | "--silent" => parsed.quiet = true,
            "--posix" => options.posix = true,
            "-p" | "--strip" => options.strip = parse_count(&arg, &next_value()?)?,
            "-F" | "--fuzz" => options.max_fuzz = parse_count(&arg, &next_value()?)?,
            "--reject-format" => {
                let value = next_value()?;
                options.reject_format =
                    Some(parse_value(&arg, &value, PatchFormat::from_reject_format)?);
            }
            "--read-only" => {
                let value = next_value()?;
                options.read_only = parse_value(&arg, &value, ReadOnlyBehavior::from_name)?;
            }
            _ if let Some(value) = short_value(&arg, "-p") => {
                options.strip = parse_count("-p", value)?;
            }
            _ if let Some(value) = short_value(&arg, "-F") => {
                options.max_fuzz = parse_count("-F", value)?;
            }
            _ if let Some(value) = arg.strip_prefix("--strip=") => {
                options.strip = parse_count("--strip", value)?;
            }
            _ if let Some(value) = arg.strip_prefix("--fuzz=") => {
                options.max_fuzz = parse_count("--fuzz", value)?;
            }
            _ if let Some(value) = arg.strip_prefix("--merge=") => {
                options.merge = parse_value("--merge", value, MergeMode::from_name)?;
            }
            _ if let Some(value) = arg.strip_prefix("--reject-format=") => {
                options.reject_format = Some(parse_value(
                    "--reject-format",
                    value,
                    PatchFormat::from_reject_format,
                )?);
            }
            _ if let Some(value) = arg.strip_prefix("--read-only=") => {
                options.read_only = parse_value("--read-only", value, ReadOnlyBehavior::from_name)?;
            }
            _ => return Err(ApplyOptionParseError::Unsupported { option: arg }),
        }
    }
    Ok(parsed)
}

/// The value of an attached short option such as `-p1`.
fn short_value<'a>(arg: &'a str, option: &str) -> Option<&'a str> {
    arg.strip_prefix(option).filter(|value| !value.is_empty())
}

fn invalid_value(option: &str, value: &str) -> ApplyOptionParseError {
    ApplyOptionParseError::InvalidValue {
        option: option.to_string(),
        value: value.to_string(),
    }
}

fn parse_value<T>(
    option: &str,
    value: &str,
    parse: impl FnOnce(&str) -> Option<T>,
) -> ParseResult<T> {
    parse(value).ok_or_else(|| invalid_value(option, value))
}

/// Parses a non-negative count, saturating values too large for `usize`.
fn parse_count(option: &str, value: &str) -> ParseResult<usize> {
    let digits = value.strip_prefix('+').unwrap_or(value);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid_value(option, value));
    }
    Ok(digits.parse().unwrap_or(usize::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_apply_options() {
        let parsed = parse_apply_options([
            "-p1",
            "--ignore-whitespace",
            "--merge",
            "diff3",
            "--fuzz=0",
            "--reject-format=context",
            "--forward",
        ])
        .unwrap();

        assert_eq!(parsed.options.strip, 1);
        assert!(parsed.options.ignore_whitespace);
        assert_eq!(parsed.options.merge, MergeMode::Diff3);
        assert_eq!(parsed.options.max_fuzz, 0);
        assert_eq!(parsed.options.reject_format, Some(PatchFormat::Context));
    }

    #[test]
    fn rejects_unsupported_apply_option() {
        assert!(matches!(
            parse_apply_options(["--backup"]),
            Err(ApplyOptionParseError::Unsupported { option }) if option == "--backup"
        ));
    }
}
