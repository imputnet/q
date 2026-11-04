use anyhow::{Context, Result, bail};
use std::collections::HashMap;
use std::env;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};

#[derive(Default)]
struct Opts {
    prefix: String,
    config: String,
    default_selected: bool,
    check: bool,
    list: bool,
    invert: bool,
    with_guards: bool,
    paths: Vec<String>,
    symbols: Vec<String>,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut opts = parse_args(env::args().skip(1).collect())?;
    if opts.config.is_empty() {
        opts.config = "-".to_string();
    }
    if opts.paths.is_empty() {
        opts.paths.push(".".to_string());
    }
    opts.prefix = slashme(&opts.prefix);
    let records = parse_config(&opts.config)?;
    if opts.check {
        check_records(&opts, &records)
    } else if opts.list {
        for record in records {
            if opts.with_guards && !record.guards.is_empty() {
                print!("{} ", record.guards.join(" "));
            }
            println!("{}{}", opts.prefix, record.patch);
        }
        Ok(())
    } else {
        let symbols: HashMap<_, _> = opts.symbols.iter().map(|s| (s.as_str(), true)).collect();
        for record in records {
            let selected = record_selected(&record, &symbols, opts.default_selected);
            if selected ^ opts.invert {
                println!("{}{}", opts.prefix, record.patch);
            }
        }
        Ok(())
    }
}

fn parse_args(args: Vec<String>) -> Result<Opts> {
    let mut opts = Opts {
        config: "-".to_string(),
        default_selected: true,
        ..Opts::default()
    };
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        let (name, inline) = arg.split_once('=').unwrap_or((&arg, ""));
        match name {
            "-h" | "--help" => {
                println!("guards - select from a list of files guarded by conditions");
                std::process::exit(0);
            }
            "-d" | "--prefix" => opts.prefix = value(inline, &mut iter, name)?,
            "-c" | "--config" => opts.config = value(inline, &mut iter, name)?,
            "-p" | "--path" => {
                opts.paths.extend(
                    value(inline, &mut iter, name)?
                        .split(':')
                        .map(str::to_string),
                );
            }
            "-D" | "--default" => {
                opts.default_selected = value(inline, &mut iter, name)? != "0";
            }
            "-C" | "--check" => opts.check = true,
            "-l" | "--list" => opts.list = true,
            "-w" | "--with-guards" => opts.with_guards = true,
            "-v" | "--invert-match" => opts.invert = true,
            _ if arg.starts_with('-') => bail!("unknown option {arg}"),
            _ => opts.symbols.push(arg),
        }
    }
    Ok(opts)
}

fn value(name_value: &str, iter: &mut impl Iterator<Item = String>, name: &str) -> Result<String> {
    if !name_value.is_empty() {
        Ok(name_value.to_string())
    } else {
        iter.next()
            .with_context(|| format!("{name}: option requires an argument"))
    }
}

fn slashme(dir: &str) -> String {
    if dir.is_empty() || dir == "./" {
        String::new()
    } else if dir.ends_with('/') {
        dir.to_string()
    } else {
        format!("{dir}/")
    }
}

#[derive(Clone)]
struct Record {
    patch: String,
    guards: Vec<String>,
}

fn parse_config(config: &str) -> Result<Vec<Record>> {
    let input: Box<dyn BufRead> = if config == "-" {
        Box::new(BufReader::new(io::stdin()))
    } else {
        Box::new(BufReader::new(
            File::open(config).with_context(|| format!("{config}: open failed"))?,
        ))
    };
    let mut records = Vec::new();
    let mut logical = String::new();
    for line in input.lines() {
        let mut line = line?;
        strip_comment(&mut line);
        if line.ends_with('\\') {
            line.pop();
            logical.push_str(&line);
            logical.push(' ');
            continue;
        }
        logical.push_str(&line);
        let mut guards = Vec::new();
        for token in logical.split_whitespace() {
            if token.starts_with('+') || token.starts_with('-') {
                guards.push(token.to_string());
            } else {
                records.push(Record {
                    patch: token.to_string(),
                    guards: guards.clone(),
                });
            }
        }
        logical.clear();
    }
    Ok(records)
}

fn strip_comment(line: &mut String) {
    let bytes = line.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i] == b'#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()) {
            line.truncate(i);
            break;
        }
    }
}

fn record_selected(record: &Record, symbols: &HashMap<&str, bool>, default_selected: bool) -> bool {
    if record.guards.is_empty() {
        return default_selected;
    }
    let mut selected = record.guards[0].starts_with('-');
    for guard in &record.guards {
        let Some(action) = guard.chars().next() else {
            continue;
        };
        let rest = &guard[1..];
        let (negated, symbol) = rest
            .strip_prefix('!')
            .map(|symbol| (true, symbol))
            .unwrap_or((false, rest));
        let matches = if negated {
            !symbols.contains_key(symbol)
        } else {
            symbol.is_empty() || symbols.contains_key(symbol)
        };
        if matches {
            selected = action == '+';
        }
    }
    selected
}

fn check_records(opts: &Opts, records: &[Record]) -> Result<()> {
    let mut files = HashMap::<String, usize>::new();
    for path in &opts.paths {
        for file in files_in(&opts.prefix, path)? {
            files.insert(file, 0);
        }
    }
    let mut problems = 0;
    for record in records {
        if let Some(count) = files.get_mut(&record.patch) {
            *count += 1;
        } else {
            if opts.config == "-" {
                println!("Not found: {}{}", opts.prefix, record.patch);
            } else {
                println!(
                    "In {} but not found: {}{}",
                    opts.config, opts.prefix, record.patch
                );
            }
            problems += 1;
        }
    }
    for (file, count) in files {
        if count == 0 {
            if opts.config == "-" {
                println!("Unused: {file}");
            } else {
                println!("Not in {}: {file}", opts.config);
            }
            problems += 1;
        } else if count > 1 {
            if opts.config == "-" {
                println!("Warning: multiple uses: {file}");
            } else {
                println!("Warning: multiple uses in {}: {file}", opts.config);
            }
        }
    }
    if problems == 0 {
        Ok(())
    } else {
        std::process::exit(1);
    }
}

fn files_in(prefix: &str, path: &str) -> Result<Vec<String>> {
    if let Some(list) = path.strip_prefix('@') {
        let text = fs::read_to_string(list)?;
        return Ok(text
            .lines()
            .map(|line| line.strip_prefix(prefix).unwrap_or(line).to_string())
            .collect());
    }
    let mut out = Vec::new();
    let rel = if path == "." {
        Path::new("")
    } else {
        Path::new(path)
    };
    let base = if path == "." {
        PathBuf::from(if prefix.is_empty() { "." } else { prefix })
    } else {
        Path::new(prefix).join(path)
    };
    collect_files(prefix, &base, rel, &mut out)?;
    Ok(out)
}

fn collect_files(prefix: &str, path: &Path, rel: &Path, out: &mut Vec<String>) -> Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "."
            || name == ".."
            || name.starts_with(".#")
            || name == "CVS"
            || name.ends_with('~')
        {
            continue;
        }
        let rel_path: PathBuf = rel.join(&name);
        if entry.file_type()?.is_dir() {
            collect_files(prefix, &Path::new(prefix).join(&rel_path), &rel_path, out)?;
        } else {
            out.push(rel_path.display().to_string());
        }
    }
    Ok(())
}
