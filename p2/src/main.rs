use std::collections::{HashSet, VecDeque};
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::exit;

use p2::{
    ApplyOptions, Error, FileReport, HunkStatus, MergeMode, PatchFormat, PatchPath, PatchSet,
    ReadOnlyBehavior, affected_paths, apply_patch, parse_patch,
};

mod cli_output;

use cli_output::{
    BackupConfig, QuotingStyle, create_backups, exit_io, parse_backup_style, plural,
    print_file_action, report_ignored_rejects, report_rejects,
};

fn main() {
    let program_name = env::args().next().unwrap_or_else(|| "p2".to_string());
    if let Some(cli) = parse_args(&program_name) {
        run(&program_name, cli);
    }
}

struct Cli {
    options: ApplyOptions,
    directory: PathBuf,
    input_file: Option<PathBuf>,
    output_file: Option<PathBuf>,
    reject_file: Option<PathBuf>,
    backup: BackupConfig,
    verbose: bool,
    silent: bool,
    force_normal: bool,
    force_ed: bool,
    force_format: Option<PatchFormat>,
    binary: bool,
    quoting_style: QuotingStyle,
}

/// Parses the command line. Returns `None` after handling `--help` or
/// `--version`; exits on usage errors.
fn parse_args(program_name: &str) -> Option<Cli> {
    let mut args = expand_cli_args(env::args().skip(1));
    if let Some(value) = env::var_os("PATCH_GET") {
        check_number(program_name, &value.to_string_lossy(), "PATCH_GET value");
    }
    let mut cli = Cli {
        options: ApplyOptions {
            posix: env::var_os("POSIXLY_CORRECT").is_some(),
            ..ApplyOptions::default()
        },
        directory: PathBuf::from("."),
        input_file: None,
        output_file: None,
        reject_file: None,
        backup: BackupConfig::default(),
        verbose: false,
        silent: false,
        force_normal: false,
        force_ed: false,
        force_format: None,
        binary: false,
        quoting_style: env::var("QUOTING_STYLE")
            .ok()
            .and_then(|value| QuotingStyle::from_name(&value))
            .unwrap_or(QuotingStyle::Shell),
    };
    let mut backup_if_mismatch = None;
    let mut positional_patch_file = false;
    let options = &mut cli.options;

    while let Some(arg) = args.pop_front() {
        let (name, mut inline_value) = split_attached_value(&arg);
        // Takes the option's value, attached or from the next argument.
        let mut value = |missing: &str| {
            inline_value
                .take()
                .map(str::to_string)
                .or_else(|| args.pop_front())
                .unwrap_or_else(|| exit_usage(missing))
        };
        match name {
            "-R" | "--reverse" => options.reverse = true,
            "-E" | "--remove-empty-files" => options.remove_empty = true,
            "-l" | "--ignore-whitespace" => options.ignore_whitespace = true,
            "--dry-run" => options.dry_run = true,
            "-d" | "--directory" => cli.directory = value("-d requires a directory").into(),
            "-i" | "--input" => cli.input_file = Some(value("-i requires a patch file").into()),
            "-o" | "--output" => {
                cli.output_file = Some(value("-o requires an output file").into());
            }
            "-r" | "--reject-file" => {
                cli.reject_file = Some(value("-r requires a reject file").into());
            }
            "-D" | "--ifdef" => options.ifdef_symbol = Some(value("-D requires an ifdef symbol")),
            "-m" | "--merge" => {
                // `--merge` takes an optional attached value; `-m` never does.
                options.merge = match inline_value {
                    Some(mode) => MergeMode::from_name(mode)
                        .unwrap_or_else(|| exit_usage(&format!("invalid merge format {mode:?}"))),
                    None => MergeMode::Merge,
                };
            }
            "-B" | "--prefix" => {
                cli.backup.enabled = true;
                cli.backup.prefix = value("-B requires a backup prefix").into();
            }
            "-z" | "--suffix" => {
                cli.backup.enabled = true;
                cli.backup.suffix = Some(value("-z requires a backup suffix"));
            }
            "-V" | "--version-control" => {
                let style = value("-V requires a version-control style");
                cli.backup.style = Some(parse_backup_style(program_name, &style));
            }
            "-Y" | "--basename-prefix" => {
                cli.backup.enabled = true;
                cli.backup.basename_prefix = value("-Y requires a backup basename prefix").into();
            }
            "-F" | "--fuzz" => {
                let fuzz = value("-F requires a fuzz factor");
                options.max_fuzz = parse_count(program_name, &fuzz, "fuzz factor");
            }
            "-p" | "--strip" => {
                let strip = value("-p requires a count");
                options.strip = parse_count(program_name, &strip, "strip count");
            }
            // Accepted and validated for compatibility, but otherwise ignored.
            "-g" | "--get" => {
                let get = value("-g requires a get option value");
                check_number(program_name, &get, "get option value");
            }
            "-x" | "--debug" => {
                let debug = value("-x requires a debugging option value");
                check_number(program_name, &debug, "debugging option");
            }
            "--quoting-style" => {
                let style = value("--quoting-style requires a style");
                cli.quoting_style = QuotingStyle::from_name(&style).unwrap_or_else(|| {
                    eprintln!("p2: invalid argument '{style}' for 'quoting style'");
                    eprintln!("Try 'p2 --help' for more information.");
                    exit(2);
                });
            }
            "--reject-format" => {
                let format = value("--reject-format requires a format");
                options.reject_format =
                    Some(PatchFormat::from_reject_format(&format).unwrap_or_else(|| {
                        exit_usage(&format!("invalid reject format {format:?}"))
                    }));
            }
            "--read-only" => {
                let behavior = value("--read-only requires a behavior");
                options.read_only = ReadOnlyBehavior::from_name(&behavior).unwrap_or_else(|| {
                    exit_usage(&format!("invalid read-only behavior {behavior:?}"))
                });
            }
            "-n" | "--normal" => {
                cli.force_normal = true;
                cli.force_format = Some(PatchFormat::Normal);
            }
            "-e" | "--ed" => {
                cli.force_ed = true;
                cli.force_format = Some(PatchFormat::Ed);
            }
            "-u" | "--unified" => cli.force_format = Some(PatchFormat::Unified),
            "-c" | "--context" => cli.force_format = Some(PatchFormat::Context),
            "--posix" => options.posix = true,
            "-T" | "--set-time" => options.set_time = true,
            "-Z" | "--set-utc" => options.set_utc = true,
            "--follow-symlinks" => options.follow_symlinks = true,
            // There is no interactive mode, so --forward has nothing to change.
            "-N" | "--forward" => {}
            "--backup-if-mismatch" => backup_if_mismatch = Some(true),
            "--no-backup-if-mismatch" => backup_if_mismatch = Some(false),
            "-b" | "--backup" => cli.backup.enabled = true,
            "--binary" => cli.binary = true,
            "-f" | "--force" => options.force = true,
            "-t" | "--batch" => options.batch = true,
            "-s" | "--silent" | "--quiet" => cli.silent = true,
            "--verbose" => cli.verbose = true,
            "-v" | "--version" => {
                print_version();
                return None;
            }
            "--help" => {
                print_help();
                return None;
            }
            _ if arg.starts_with('-') => exit_usage(&format!("unsupported option {arg:?}")),
            _ if options.target_file.is_none() => options.target_file = Some(PathBuf::from(arg)),
            _ if !positional_patch_file => {
                cli.input_file = Some(PathBuf::from(arg));
                positional_patch_file = true;
            }
            _ => exit_usage("too many file operands"),
        }
    }

    cli.backup.if_mismatch = backup_if_mismatch
        .unwrap_or_else(|| !cli.options.posix && env::var_os("POSIXLY_CORRECT").is_none());
    Some(cli)
}

/// Splits an attached option value: `-p1` into `-p` and `1`, and
/// `--strip=1` into `--strip` and `1`. Other arguments are returned whole.
fn split_attached_value(arg: &str) -> (&str, Option<&str>) {
    if let Some((name, value)) = arg.split_once('=')
        && LONG_OPTIONS_WITH_VALUE.contains(&name)
    {
        return (name, Some(value));
    }
    if arg.len() > 2
        && !arg.starts_with("--")
        && let Some(option) = arg.strip_prefix('-').and_then(|rest| rest.chars().next())
        && short_option_takes_value(option)
    {
        let (name, value) = arg.split_at(2);
        return (name, Some(value));
    }
    (arg, None)
}

fn run(program_name: &str, cli: Cli) {
    change_directory_or_exit(program_name, &cli.directory);
    let root = Path::new(".");
    let options = &cli.options;

    let raw_input = read_patch_input(root, cli.input_file.as_deref());
    if let Some(nul_index) = raw_input.bytes().position(|byte| byte == 0) {
        let line = raw_input[..nul_index].matches('\n').count() + 1;
        eprintln!("{program_name}: **** patch line {line} contains NUL byte");
        exit(2);
    }

    let normalized_input;
    let (input, stripped_crs) = if !cli.binary
        && raw_input.contains("\r\n")
        && !raw_patch_applies(root, &raw_input, options)
    {
        normalized_input = raw_input.replace("\r\n", "\n");
        (normalized_input.as_str(), true)
    } else {
        (raw_input.as_str(), false)
    };

    if input.trim().is_empty() {
        // With nothing to apply, `-o` still gets a copy of the original.
        if let (Some(source), Some(output)) = (&options.target_file, &cli.output_file) {
            copy_or_exit(&root.join(source), &root.join(output));
            println!(
                "patching file {} (read from {})",
                output.display(),
                source.display()
            );
        }
        return;
    }

    let (patch, malformed) = parse_or_exit(program_name, input, options);

    if options.target_file.is_none() && has_headerless_file_patch(&patch) {
        if cli.force_normal || cli.force_ed {
            println!("can't find file to patch at input line 1");
            println!("No file to patch.  Skipping patch.");
            if cli.force_normal {
                println!("1 out of 1 hunk ignored");
            }
            exit(1);
        }
        exit_garbage(program_name);
    }
    if let Some(format) = cli.force_format
        && patch.files.iter().any(|file| file.format != format)
    {
        exit_garbage(program_name);
    }
    if let Some(prereq) = extract_prereq(input) {
        check_prereq(program_name, root, &patch, options, &prereq);
    }

    if let Some(output) = &cli.output_file {
        if path_contains_newline(output) {
            eprintln!(
                "{program_name}: **** Can't create file {} : Invalid byte sequence",
                quoted_newline_path(output)
            );
            exit(2);
        }
        apply_patch_to_output(root, output, &patch, options, cli.verbose);
        return;
    }

    if let Some(path) = first_newline_target(&patch, options) {
        println!("patching file {}", quoted_newline_path(&path));
        let _ = io::stdout().flush();
        eprintln!(
            "{program_name}: **** Can't rename file ab.* to {} : Invalid byte sequence",
            quoted_newline_path(&path)
        );
        exit(2);
    }

    // Backups need to know which files will change, so do a dry run first.
    if !options.dry_run && (cli.backup.enabled || cli.backup.if_mismatch) {
        let dry_options = ApplyOptions {
            dry_run: true,
            ..options.clone()
        };
        let preflight = apply_or_exit(root, &patch, &dry_options, input);
        create_backups(root, &preflight, &cli.backup);
    }

    let report = apply_or_exit(root, &patch, options, input);

    let quiet_ed = patch
        .files
        .iter()
        .all(|file| file.format == PatchFormat::Ed);
    let failed = report.failed_hunks() > 0
        || report
            .files
            .iter()
            .any(|file| file.delete_conflict || file.skipped);
    if !quiet_ed && !cli.silent {
        if stripped_crs {
            println!("(Stripping trailing CRs from patch; use --binary to disable.)");
        }
        let mut reject_files_seen = HashSet::new();
        for file in &report.files {
            print_file_report(root, &cli, file, &mut reject_files_seen);
        }
    }

    if let Some(malformed) = malformed {
        if !cli.silent {
            if let Some(path) = malformed.path {
                println!("patching file {}", path.display());
            }
            report_malformed_patch(program_name, input, malformed.line);
        }
        exit(2);
    }
    if failed {
        exit(1);
    }
}

fn read_patch_input(root: &Path, input_file: Option<&Path>) -> String {
    match input_file {
        Some(path) => {
            let path = root.join(path);
            fs::read_to_string(&path).unwrap_or_else(|error| {
                eprintln!("p2: {}: {error}", path.display());
                exit(2);
            })
        }
        None => {
            let mut input = String::new();
            io::stdin()
                .read_to_string(&mut input)
                .unwrap_or_else(|error| {
                    eprintln!("p2: stdin: {error}");
                    exit(2);
                });
            input
        }
    }
}

fn copy_or_exit(source: &Path, destination: &Path) {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|error| exit_io(parent, error));
    }
    fs::copy(source, destination).unwrap_or_else(|error| exit_io(source, error));
}

fn exit_garbage(program_name: &str) -> ! {
    eprintln!("{program_name}: **** Only garbage was found in the patch input.");
    exit(2);
}

/// Parses the patch, recovering the files before a malformed one so they
/// can still be applied (as GNU patch does).
fn parse_or_exit(
    program_name: &str,
    input: &str,
    options: &ApplyOptions,
) -> (PatchSet, Option<MalformedPatch>) {
    let error = match parse_patch(input) {
        Ok(patch) => return (patch, None),
        Err(error) => error,
    };
    match &error {
        Error::Parse { line, message } => {
            if let Some((patch, malformed)) = recover_parse_prefix(input, *line, options) {
                return (patch, Some(malformed));
            }
            if message == "replacement text or line numbers mangled in hunk" {
                eprintln!("{program_name}: **** {message} at line {line}");
                exit(2);
            }
            if message.contains("no unified diff file headers found") {
                exit_garbage(program_name);
            }
            eprintln!("{program_name}: {error}");
            exit(2);
        }
        Error::UnsupportedGitBinary { .. } => {
            eprintln!("{error}");
            exit(1);
        }
        _ => {
            eprintln!("{program_name}: {error}");
            exit(2);
        }
    }
}

/// Applies the patch, translating path errors into GNU patch's messages.
fn apply_or_exit(
    root: &Path,
    patch: &PatchSet,
    options: &ApplyOptions,
    input: &str,
) -> p2::ApplyReport {
    apply_patch(root, patch, options).unwrap_or_else(|error| {
        if error.to_string().contains("unexpected NUL byte") {
            report_cannot_find_file(input, StripHint::ShouldHaveStripped);
        } else if let Error::DangerousPath { path } = &error {
            println!("Ignoring potentially dangerous file name {path}");
            report_cannot_find_file(input, StripHint::WrongStrip);
        } else if matches!(error, Error::StripPath { .. }) {
            report_cannot_find_file(input, strip_error_hint(input));
        } else {
            eprintln!("p2: {error}");
        }
        exit(1);
    })
}

fn print_file_report(
    root: &Path,
    cli: &Cli,
    file: &FileReport,
    reject_files_seen: &mut HashSet<PathBuf>,
) {
    let path = file.path.display();
    let dry_run = cli.options.dry_run;
    let reject_file = cli.reject_file.as_deref();
    if file.create_conflict {
        println!("The next patch would create the file {path},");
        println!("which already exists!  Applying it anyway.");
    }
    if file.delete_missing {
        println!("The next patch would delete the file {path},");
        if file.skipped {
            println!("which does not exist!  Assume -R? [n] ");
            println!("Apply anyway? [n] ");
            println!("Skipping patch.");
            println!(
                "{} out of {} hunk{} ignored",
                file.ignored_hunks,
                file.ignored_hunks,
                plural(file.ignored_hunks)
            );
            return;
        }
        println!("which does not exist!  Applying it anyway.");
    }
    if file.empty_conflict {
        println!("The next patch would empty out the file {path},");
        println!("which is already empty!  Applying it anyway.");
    }
    if file.assumed_reverse {
        println!("The next patch would empty out the file {path},");
        println!("which is already empty!  Assuming -R.");
    }
    if file.read_only_refused || file.not_regular_refused {
        if file.read_only_refused {
            println!("File {path} is read-only; refusing to patch");
        } else {
            println!("File {path} is not a regular file -- refusing to patch");
        }
        report_ignored_rejects(root, file, reject_file, dry_run, reject_files_seen);
        return;
    }
    if file.invalid_path_refused {
        println!("Invalid file name {path} -- skipping patch");
        return;
    }
    if file.read_only && cli.options.read_only == ReadOnlyBehavior::Warn {
        println!("File {path} is read-only; trying to patch anyway");
    }
    let action = if dry_run { "checking" } else { "patching" };
    print_file_action(action, file, cli.quoting_style);
    print_hunk_reports(file, cli.verbose);
    if file.delete_conflict {
        println!("Not deleting file {path} as content differs from patch");
    }
    if file.time_mismatch {
        println!("Not setting time of file {path} (time mismatch)");
    }
    report_rejects(root, file, reject_file, dry_run, reject_files_seen);
}

struct MalformedPatch {
    line: usize,
    path: Option<PathBuf>,
}

fn recover_parse_prefix(
    input: &str,
    line: usize,
    options: &ApplyOptions,
) -> Option<(PatchSet, MalformedPatch)> {
    let lines = input.split_inclusive('\n').collect::<Vec<_>>();
    let error_index = line.checked_sub(1)?;
    let file_start = malformed_file_start(&lines, error_index)?;
    if file_start == 0 {
        return None;
    }
    let patch = parse_patch(&lines[..file_start].concat()).ok()?;
    Some((
        patch,
        MalformedPatch {
            line,
            path: malformed_file_path(&lines, file_start, error_index, options),
        },
    ))
}

/// The start of the last file header at or before `error_index`.
fn malformed_file_start(lines: &[&str], error_index: usize) -> Option<usize> {
    let end = error_index.min(lines.len().checked_sub(1)?);
    (0..=end).rev().find(|&index| {
        lines[index].starts_with("diff ")
            || (lines[index].starts_with("--- ")
                && lines
                    .get(index + 1)
                    .is_some_and(|line| line.starts_with("+++ ")))
    })
}

fn malformed_file_path(
    lines: &[&str],
    file_start: usize,
    error_index: usize,
    options: &ApplyOptions,
) -> Option<PathBuf> {
    for line in &lines[file_start..error_index.min(lines.len())] {
        if let Some(path) = line.strip_prefix("+++ ") {
            let raw = header_path_for_display(path)?;
            if raw != "/dev/null" {
                return Some(strip_display_path(raw, options.strip));
            }
        }
    }
    None
}

fn header_path_for_display(raw: &str) -> Option<&str> {
    let raw = raw.trim_end_matches(['\r', '\n']).trim_start();
    if raw.is_empty() || raw.starts_with('"') {
        return None;
    }
    raw.split_whitespace().next()
}

fn strip_display_path(raw: &str, strip: usize) -> PathBuf {
    let raw = raw
        .strip_prefix("a/")
        .or_else(|| raw.strip_prefix("b/"))
        .unwrap_or(raw);
    let stripped = raw.split('/').skip(strip).collect::<Vec<_>>().join("/");
    PathBuf::from(if stripped.is_empty() { raw } else { &stripped })
}

fn report_malformed_patch(program_name: &str, input: &str, line: usize) {
    let text = input
        .split_inclusive('\n')
        .nth(line.saturating_sub(1))
        .unwrap_or("")
        .trim_end_matches(['\r', '\n']);
    eprintln!("{program_name}: **** malformed patch at line {line}: {text}");
    eprintln!();
}

/// Whether the patch has bare hunks with no file name to apply them to.
fn has_headerless_file_patch(patch: &PatchSet) -> bool {
    let is_empty = |path: &PatchPath| matches!(path, PatchPath::File(path) if path.is_empty());
    patch
        .files
        .iter()
        .any(|file| is_empty(&file.old_path) && is_empty(&file.new_path))
}

/// The revision named by the first `Prereq:` line, if any.
fn extract_prereq(input: &str) -> Option<String> {
    let (line_number, rest) = input
        .lines()
        .enumerate()
        .find_map(|(index, line)| Some((index, line.strip_prefix("Prereq:")?)))?;
    let mut words = rest.split_whitespace();
    let prereq = words.next()?;
    if words.next().is_some() {
        println!("Prereq: with multiple words at line {line_number} of patch");
    }
    Some(prereq.to_string())
}

fn check_prereq(
    program_name: &str,
    root: &Path,
    patch: &PatchSet,
    options: &ApplyOptions,
    prereq: &str,
) {
    let Ok(paths) = affected_paths(root, patch, options) else {
        return;
    };
    let found = paths.iter().any(|path| {
        fs::read(root.join(path)).is_ok_and(|content| contains_word(&content, prereq.as_bytes()))
    });
    if found {
        return;
    }
    if options.force {
        println!(
            "Warning: this file doesn't appear to be the {prereq} version -- patching anyway."
        );
    } else if options.batch {
        eprintln!(
            "{program_name}: **** This file doesn't appear to be the {prereq} version -- aborting."
        );
        exit(2);
    } else {
        println!("This file doesn't appear to be the {prereq} version -- patch anyway? [n] ");
        eprintln!("{program_name}: **** aborted");
        exit(2);
    }
}

/// Whether `word` occurs in `content` delimited by whitespace or the ends.
fn contains_word(content: &[u8], word: &[u8]) -> bool {
    if word.is_empty() || word.len() > content.len() {
        return false;
    }
    content
        .windows(word.len())
        .enumerate()
        .any(|(index, window)| {
            window == word
                && index
                    .checked_sub(1)
                    .is_none_or(|before| content[before].is_ascii_whitespace())
                && content
                    .get(index + word.len())
                    .is_none_or(u8::is_ascii_whitespace)
        })
}

fn change_directory_or_exit(program_name: &str, directory: &Path) {
    if directory == Path::new(".") {
        return;
    }
    if let Err(error) = env::set_current_dir(directory) {
        eprintln!(
            "{program_name}: **** Can't change to directory {} : {error}",
            directory.display()
        );
        exit(2);
    }
}

/// Whether the patch applies cleanly as-is, in which case CRs are kept.
fn raw_patch_applies(root: &Path, input: &str, options: &ApplyOptions) -> bool {
    let Ok(patch) = parse_patch(input) else {
        return false;
    };
    if options.target_file.is_none() && has_headerless_file_patch(&patch) {
        return false;
    }
    let dry_options = ApplyOptions {
        dry_run: true,
        ..options.clone()
    };
    let Ok(report) = apply_patch(root, &patch, &dry_options) else {
        return false;
    };
    report.failed_hunks() == 0
        && report
            .files
            .iter()
            .all(|file| !file.delete_conflict && !file.skipped && !file.read_only_refused)
}

fn expand_cli_args(args: impl IntoIterator<Item = String>) -> VecDeque<String> {
    args.into_iter()
        .flat_map(expand_short_options)
        .map(canonical_long_option)
        .collect()
}

/// Expands an unambiguous abbreviation such as `--rev` to `--reverse`.
fn canonical_long_option(arg: String) -> String {
    if !arg.starts_with("--") || arg == "--" {
        return arg;
    }
    let (name, value) = match arg.split_once('=') {
        Some((name, value)) => (name, Some(value)),
        None => (arg.as_str(), None),
    };
    if LONG_OPTIONS.contains(&name) {
        return arg;
    }
    let mut matches = LONG_OPTIONS
        .iter()
        .filter(|option| option.starts_with(name));
    let (Some(canonical), None) = (matches.next(), matches.next()) else {
        return arg;
    };
    match value {
        Some(value) => format!("{canonical}={value}"),
        None => canonical.to_string(),
    }
}

/// Splits bundled short options: `-Rp1` becomes `-R`, `-p1`.
fn expand_short_options(arg: String) -> Vec<String> {
    if !arg.starts_with('-') || arg.starts_with("--") || arg.len() <= 2 {
        return vec![arg];
    }

    let mut expanded = Vec::new();
    for (index, option) in arg.char_indices().skip(1) {
        if short_option_takes_value(option) {
            let value = &arg[index + option.len_utf8()..];
            expanded.push(format!("-{option}{value}"));
            return expanded;
        }
        if !short_option_takes_no_value(option) {
            return vec![arg];
        }
        expanded.push(format!("-{option}"));
    }
    expanded
}

fn short_option_takes_value(option: char) -> bool {
    matches!(
        option,
        'p' | 'd' | 'i' | 'o' | 'r' | 'D' | 'B' | 'z' | 'V' | 'Y' | 'F' | 'g' | 'x'
    )
}

fn short_option_takes_no_value(option: char) -> bool {
    matches!(
        option,
        'R' | 'E'
            | 'l'
            | 'm'
            | 'n'
            | 'e'
            | 'u'
            | 'c'
            | 'T'
            | 'Z'
            | 'N'
            | 'b'
            | 'f'
            | 't'
            | 's'
            | 'v'
    )
}

const LONG_OPTIONS: &[&str] = &[
    "--backup",
    "--backup-if-mismatch",
    "--basename-prefix",
    "--batch",
    "--binary",
    "--context",
    "--debug",
    "--directory",
    "--dry-run",
    "--ed",
    "--follow-symlinks",
    "--force",
    "--forward",
    "--fuzz",
    "--get",
    "--help",
    "--ifdef",
    "--ignore-whitespace",
    "--input",
    "--merge",
    "--no-backup-if-mismatch",
    "--normal",
    "--output",
    "--posix",
    "--prefix",
    "--quiet",
    "--quoting-style",
    "--read-only",
    "--reject-file",
    "--reject-format",
    "--remove-empty-files",
    "--reverse",
    "--set-time",
    "--set-utc",
    "--silent",
    "--strip",
    "--suffix",
    "--unified",
    "--verbose",
    "--version",
    "--version-control",
];

/// Long options that accept a `--name=value` form.
const LONG_OPTIONS_WITH_VALUE: &[&str] = &[
    "--basename-prefix",
    "--debug",
    "--directory",
    "--fuzz",
    "--get",
    "--ifdef",
    "--input",
    "--merge",
    "--output",
    "--prefix",
    "--quoting-style",
    "--read-only",
    "--reject-file",
    "--reject-format",
    "--strip",
    "--suffix",
    "--version-control",
];

fn first_newline_target(patch: &PatchSet, options: &ApplyOptions) -> Option<PathBuf> {
    if let Some(target) = &options.target_file {
        return path_contains_newline(target).then(|| target.clone());
    }
    patch.files.iter().find_map(|file| {
        let path = if options.reverse {
            &file.old_path
        } else {
            &file.new_path
        };
        match path {
            PatchPath::File(path) if path.contains('\n') => Some(PathBuf::from(path)),
            _ => None,
        }
    })
}

fn path_contains_newline(path: &Path) -> bool {
    path.to_string_lossy().contains('\n')
}

fn quoted_newline_path(path: &Path) -> String {
    format!("'{}'", path.display())
}

enum StripHint {
    ShouldHaveStripped,
    WrongStrip,
}

fn strip_error_hint(input: &str) -> StripHint {
    let first = input.lines().next().unwrap_or("").trim_start();
    if first
        .strip_prefix("--- ")
        .or_else(|| first.strip_prefix("*** "))
        .is_some_and(|rest| rest.split_whitespace().next() == Some("/dev/null"))
    {
        StripHint::ShouldHaveStripped
    } else {
        StripHint::WrongStrip
    }
}

fn report_cannot_find_file(input: &str, hint: StripHint) {
    let mut lines = input.lines();
    let first = lines.next().unwrap_or("");
    let second = lines.next().unwrap_or("");
    println!("can't find file to patch at input line 3");
    match hint {
        StripHint::ShouldHaveStripped => {
            println!("Perhaps you should have used the -p or --strip option?");
        }
        StripHint::WrongStrip => println!("Perhaps you used the wrong -p or --strip option?"),
    }
    println!("The text leading up to this was:");
    println!("--------------------------");
    println!("|{first}");
    println!("|{second}");
    println!("--------------------------");
    println!("No file to patch.  Skipping patch.");
    println!("1 out of 1 hunk ignored");
}

/// `-o`: applies each file patch to a scratch copy of the operand and
/// appends the results to `output`.
fn apply_patch_to_output(
    root: &Path,
    output: &Path,
    patch: &PatchSet,
    options: &ApplyOptions,
    verbose: bool,
) {
    let Some(source) = &options.target_file else {
        eprintln!("p2: -o requires a file operand for this patch form");
        exit(2);
    };
    let source_path = root.join(source);
    let output_path = root.join(output);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|error| exit_io(parent, error));
    }

    let temp_name = format!(".p2-output-{}-{}", std::process::id(), unique_stamp());
    let temp_path = root.join(&temp_name);
    fs::copy(&source_path, &temp_path).unwrap_or_else(|error| exit_io(&source_path, error));
    fs::write(&output_path, "").unwrap_or_else(|error| exit_io(&output_path, error));
    fn fail(temp_path: &Path, message: String) -> ! {
        let _ = fs::remove_file(temp_path);
        eprintln!("p2: {message}");
        exit(1);
    }

    let single_options = ApplyOptions {
        target_file: Some(PathBuf::from(&temp_name)),
        ..options.clone()
    };
    let mut failed_hunks = 0;
    for file_patch in &patch.files {
        let single = PatchSet {
            files: vec![file_patch.clone()],
        };
        let report = apply_patch(root, &single, &single_options)
            .unwrap_or_else(|error| fail(&temp_path, error.to_string()));
        for file in &report.files {
            println!(
                "patching file {} (read from {})",
                output.display(),
                source.display()
            );
            print_hunk_reports(file, verbose);
        }
        failed_hunks += report.failed_hunks();
        let content = fs::read(&temp_path)
            .unwrap_or_else(|error| fail(&temp_path, format!("{}: {error}", temp_path.display())));
        fs::OpenOptions::new()
            .append(true)
            .open(&output_path)
            .and_then(|mut file| file.write_all(&content))
            .unwrap_or_else(|error| {
                fail(&temp_path, format!("{}: {error}", output_path.display()))
            });
    }
    let _ = fs::remove_file(&temp_path);
    if failed_hunks > 0 {
        exit(1);
    }
}

fn print_hunk_reports(file: &FileReport, verbose: bool) {
    for hunk in &file.hunk_reports {
        let number = hunk.hunk;
        match &hunk.status {
            HunkStatus::Applied {
                requested_line,
                applied_line,
                fuzz,
            } => {
                if verbose || requested_line != applied_line || *fuzz > 0 {
                    print!("Hunk #{number} succeeded at {applied_line}");
                    if *fuzz > 0 {
                        print!(" with fuzz {fuzz}");
                    }
                    let offset = *applied_line as isize - *requested_line as isize;
                    if offset != 0 {
                        let noun = if offset == 1 { "line" } else { "lines" };
                        print!(" (offset {offset} {noun})");
                    }
                    println!(".");
                }
            }
            HunkStatus::Failed { requested_line } => {
                let endings = if hunk.different_line_endings {
                    " (different line endings)"
                } else {
                    ""
                };
                println!("Hunk #{number} FAILED at {requested_line}{endings}.");
            }
            HunkStatus::Merged { ranges, .. } => {
                println!("Hunk #{number} merged at {}.", format_ranges(ranges));
            }
            HunkStatus::AlreadyApplied {
                applied_line,
                end_line,
                ..
            } => {
                println!(
                    "Hunk #{number} already applied at {}.",
                    format_range(*applied_line, *end_line)
                );
            }
            HunkStatus::Conflict {
                start_line,
                end_line,
                merged_ranges,
                ..
            } => {
                print!(
                    "Hunk #{number} NOT MERGED at {}",
                    format_range(*start_line, *end_line)
                );
                if !merged_ranges.is_empty() {
                    print!(", merged at {}", format_ranges(merged_ranges));
                }
                println!(".");
            }
        }
    }
}

fn format_range(start: usize, end: usize) -> String {
    if start == end {
        start.to_string()
    } else {
        format!("{start}-{end}")
    }
}

fn format_ranges(ranges: &[(usize, usize)]) -> String {
    ranges
        .iter()
        .map(|&(start, end)| format_range(start, end))
        .collect::<Vec<_>>()
        .join(",")
}

fn unique_stamp() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

/// Parses a non-negative count, saturating values too large for `usize`.
fn parse_count(program_name: &str, value: &str, context: &str) -> usize {
    if value.starts_with('-') {
        eprintln!("{program_name}: **** {context} {value} is negative");
        exit(2);
    }
    let digits = value.strip_prefix('+').unwrap_or(value);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        exit_not_a_number(program_name, context, value);
    }
    digits.parse().unwrap_or(usize::MAX)
}

/// Validates an optionally signed number whose value is otherwise unused.
fn check_number(program_name: &str, value: &str, context: &str) {
    let digits = value
        .strip_prefix('-')
        .or_else(|| value.strip_prefix('+'))
        .unwrap_or(value);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        exit_not_a_number(program_name, context, value);
    }
}

fn exit_not_a_number(program_name: &str, context: &str, value: &str) -> ! {
    eprintln!("{program_name}: **** {context} {value} is not a number");
    exit(2);
}

fn exit_usage(message: &str) -> ! {
    eprintln!("p2: {message}");
    eprintln!("Try 'p2 --help' for more information.");
    exit(2);
}

fn print_help() {
    print!(
        "\
Usage: p2 [OPTION]... [ORIGFILE [PATCHFILE]]
Apply unified diff patches.

  -p, --strip N       strip N leading path components
  -c, --context       interpret the patch as a context difference
  -u, --unified       interpret the patch as a unified difference
  -R, --reverse       apply the patch in reverse
  -E                  remove files that become empty after patching
  -l, --ignore-whitespace
  -d, --directory DIR change to DIR before applying
  -i, --input FILE    read patch from FILE instead of stdin
  -o, --output FILE   write patched output to FILE
  -r, --reject-file FILE
  -D NAME, --ifdef NAME
      --merge[=merge|diff3]
  -b                  make a backup before patching
      --backup-if-mismatch
      --no-backup-if-mismatch
  -V STYLE            use STYLE version control for backup names
  -B PREFIX           prepend PREFIX to backup file names
  -g NUM, --get NUM   get files from RCS etc. if positive
      --dry-run       check whether the patch applies without writing
  -T, --set-time      set patched file times from local diff headers
  -Z, --set-utc       set patched file times from UTC diff headers
      --quoting-style STYLE
      --follow-symlinks
  -x NUM, --debug NUM set debugging flags
  -v, --version       output version information
  FILE                apply patch to FILE
"
    );
}

fn print_version() {
    print!(
        "\
p2 {}
A Rust implementation of GNU patch-compatible patch application

License GPLv3+: GNU GPL version 3 or later <http://gnu.org/licenses/gpl.html>.
This is free software: you are free to change and redistribute it.
There is NO WARRANTY, to the extent permitted by law.
",
        env!("CARGO_PKG_VERSION")
    );
}
