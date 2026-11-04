//! Shell completion: the `q completion <shell>` scripts and the hidden
//! `q __complete` helper they call.

use crate::cli::{COMMANDS, CommandSpec, resolve_command};
use crate::state::State;
use anyhow::{Result, bail};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

pub(crate) fn cmd_completion(args: Vec<OsString>) -> Result<()> {
    let shell = args
        .first()
        .map(|arg| arg.to_string_lossy())
        .unwrap_or_default();
    match shell.as_ref() {
        "bash" => {
            let script = r#"_q_complete() {
    local cur
    cur="${COMP_WORDS[COMP_CWORD]}"
    COMPREPLY=( $(COMP_CWORD="$COMP_CWORD" q __complete "${COMP_WORDS[@]:1}" -- "$cur") )
    local reply
    for reply in "${COMPREPLY[@]}"; do
        if [[ "$reply" == */ ]]; then
            compopt -o nospace 2>/dev/null || true
            break
        fi
    done
}
complete -o default -F _q_complete q quilt
"#;
            print!("{script}");
        }
        "zsh" => {
            let script = r#"_q_complete() {
    local -a completions
    local completion
    local no_space
    completions=()
    no_space=0
    while IFS= read -r completion; do
        completions+=("$completion")
        if [[ "$completion" == */ ]]; then
            no_space=1
        fi
    done < <(q __complete "${words[@]}" -- "$words[CURRENT]")
    if (( ${#completions[@]} )); then
        if (( no_space )); then
            compadd -S '' -- "${completions[@]}"
        else
            compadd -- "${completions[@]}"
        fi
    else
        _files
    fi
}
if ! whence -w compdef >/dev/null; then
    autoload -Uz compinit
    compinit
fi
compdef _q_complete q quilt
"#;
            print!("{script}");
        }
        "fish" => {
            let script = r#"function __q_complete
    set -l tokens (commandline -opc)
    set -l cur (commandline -ct)
    q __complete $tokens -- $cur
end
complete -c q -a '(__q_complete)'
complete -c quilt -a '(__q_complete)'
"#;
            print!("{script}");
        }
        _ => bail!("Usage: quilt completion {{bash|zsh|fish}}"),
    }
    Ok(())
}

pub(crate) fn cmd_complete(args: Vec<OsString>) -> Result<()> {
    let (mut words, current) = split_completion_words(args);
    strip_completion_executable(&mut words);
    if let Some(prefix) = patch_option_completion_prefix(&words, &current) {
        print_patch_completions(&prefix)?;
        return Ok(());
    }
    if words.is_empty()
        || (words.len() == 1
            && !current.is_empty()
            && words
                .first()
                .map(|word| word.to_string_lossy() == current)
                .unwrap_or(false))
    {
        print_command_completions(&current);
        return Ok(());
    }
    let Some((command_index, command)) = find_completion_command(&words) else {
        print_command_completions(&current);
        return Ok(());
    };
    let mut command_args = words[command_index + 1..].to_vec();
    if command_args
        .last()
        .map(|arg| arg.to_string_lossy() == current)
        .unwrap_or(false)
    {
        command_args.pop();
    }
    if command.name == "new" {
        print_new_directory_completions(&current)?;
    } else if wants_patch_completion(command, &command_args) {
        print_patch_completions(&current)?;
    }
    Ok(())
}

fn find_completion_command(words: &[OsString]) -> Option<(usize, &'static CommandSpec)> {
    words.iter().enumerate().find_map(|(idx, word)| {
        let command = resolve_command(&word.to_string_lossy()).ok().flatten()?;
        Some((idx, command))
    })
}

fn patch_option_completion_prefix(words: &[OsString], current: &str) -> Option<String> {
    if matches!(current, "-P" | "--combine") {
        return Some(String::new());
    }
    let mut words = words.to_vec();
    if words
        .last()
        .map(|word| word.to_string_lossy() == current)
        .unwrap_or(false)
    {
        words.pop();
    }
    words.last().and_then(|word| match word.to_str() {
        Some("-P" | "--combine") => Some(current.to_string()),
        _ => None,
    })
}

fn print_patch_completions(current: &str) -> Result<()> {
    for candidate in patch_completion_candidates()? {
        if candidate.starts_with(current) {
            println!("{candidate}");
        }
    }
    Ok(())
}

fn print_new_directory_completions(current: &str) -> Result<()> {
    for candidate in new_directory_completion_candidates(current)? {
        println!("{candidate}");
    }
    Ok(())
}

fn new_directory_completion_candidates(current: &str) -> Result<Vec<String>> {
    let state = State::discover(false)?;
    let (base, leaf) = match current.rsplit_once('/') {
        Some((parent, leaf)) => (format!("{parent}/"), leaf),
        None => (String::new(), current),
    };
    if !is_normal_relative_path(&base) {
        return Ok(Vec::new());
    }
    let dir = state.patches_dir.join(Path::new(&base));
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(Vec::new());
    };
    let mut candidates = Vec::new();
    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with(leaf) {
            candidates.push(format!("{base}{name}/"));
        }
    }
    candidates.sort();
    Ok(candidates)
}

fn is_normal_relative_path(path: &str) -> bool {
    let path = Path::new(path);
    !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn split_completion_words(args: Vec<OsString>) -> (Vec<OsString>, String) {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut after_delimiter = false;
    for arg in args {
        if after_delimiter {
            current = arg.to_string_lossy().into_owned();
        } else if arg == "--" {
            after_delimiter = true;
        } else {
            words.push(arg);
        }
    }
    if !after_delimiter {
        current = words
            .pop()
            .map(|arg| arg.to_string_lossy().into_owned())
            .unwrap_or_default();
    }
    (words, current)
}

fn strip_completion_executable(words: &mut Vec<OsString>) {
    let Some(first) = words.first() else {
        return;
    };
    let path = PathBuf::from(first);
    let name = path.file_name().unwrap_or(first.as_os_str());
    if name == "q" || name == "quilt" {
        words.remove(0);
    }
}

fn print_command_completions(current: &str) {
    for command in COMMANDS {
        if command.name.starts_with(current) {
            println!("{}", command.name);
        }
    }
}

fn wants_patch_completion(command: &CommandSpec, args: &[OsString]) -> bool {
    command.completes_patches
        || args
            .last()
            .is_some_and(|arg| arg == "-P" || arg == "--combine")
}

fn patch_completion_candidates() -> Result<Vec<String>> {
    let state = State::discover(false)?;
    let entries = state.entries()?;
    let mut counts = HashMap::<String, usize>::new();
    for entry in &entries {
        for alias in patch_aliases(&entry.name) {
            *counts.entry(alias).or_default() += 1;
        }
    }
    let mut candidates = Vec::new();
    for entry in entries {
        candidates.push(entry.name.clone());
        for alias in patch_aliases(&entry.name) {
            if counts.get(&alias) == Some(&1) {
                candidates.push(alias);
            }
        }
    }
    candidates.sort();
    candidates.dedup();
    Ok(candidates)
}

fn patch_aliases(name: &str) -> Vec<String> {
    let Some(base) = PathBuf::from(name)
        .file_name()
        .and_then(|file| file.to_str())
        .map(str::to_string)
    else {
        return Vec::new();
    };
    let mut aliases = vec![base.clone()];
    for suffix in [".patch", ".diff"] {
        if let Some(stem) = base.strip_suffix(suffix) {
            aliases.push(stem.to_string());
        }
    }
    aliases
}
