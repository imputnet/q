use crate::cli::usage;
use crate::commands::applied_patch;
use crate::config::diff_command;
use crate::state::State;
use anyhow::{Context, Result, bail};
use lexopt::prelude::*;
use std::collections::HashMap;
use std::collections::{BTreeSet, HashSet};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::process::Stdio;

pub(crate) fn cmd_graph(args: Vec<OsString>) -> Result<()> {
    let mut all = false;
    let mut edge_labels = false;
    let mut reduce = false;
    let mut output_ps = false;
    let mut lines = None::<usize>;
    let mut patch_arg = None::<String>;
    let mut parser = lexopt::Parser::from_args(args);
    while let Some(arg) = parser.next()? {
        match arg {
            Long("all") => all = true,
            Long("reduce") => reduce = true,
            Long("lines") => {
                lines = match parser.optional_value() {
                    Some(value) => Some(value.string()?.parse()?),
                    None => Some(2),
                };
            }
            Long("edge-labels") => {
                let value = parser.value()?.string()?;
                if value != "files" {
                    bail!("{}", usage("graph"));
                }
                edge_labels = true;
            }
            Short('T') => {
                let value = parser.value()?.string()?;
                if value != "ps" {
                    bail!("{}", usage("graph"));
                }
                output_ps = true;
            }
            Value(v) => patch_arg = Some(v.to_string_lossy().into_owned()),
            _ => {}
        }
    }
    let state = State::discover(false)?;
    state.require_series()?;
    let applied = state.applied()?;
    let selected = if all {
        applied
    } else {
        let patch = patch_arg
            .as_deref()
            .map(|patch| applied_patch(&state, Some(patch)))
            .transpose()?
            .unwrap_or(state.top()?);
        related_graph_patches(&state, &applied, &patch, lines)?
    };
    let graph = graph_dot(&state, &selected, edge_labels, reduce, lines)?;
    if output_ps {
        let mut child = Command::new("dot")
            .arg("-T")
            .arg("ps")
            .stdin(Stdio::piped())
            .spawn()
            .context("failed to run dot")?;
        if let Some(stdin) = child.stdin.as_mut() {
            stdin.write_all(graph.as_bytes())?;
        }
        let status = child.wait()?;
        if !status.success() {
            bail!("dot failed");
        }
    } else {
        print!("{graph}");
    }
    Ok(())
}

fn related_graph_patches(
    state: &State,
    applied: &[String],
    patch: &str,
    lines: Option<usize>,
) -> Result<Vec<String>> {
    let all_edges = graph_edges(state, applied, lines)?;
    let mut stack = vec![patch.to_string()];
    let mut seen = HashSet::new();
    while let Some(node) = stack.pop() {
        if !seen.insert(node.clone()) {
            continue;
        }
        for Edge { from, to, .. } in &all_edges {
            if from == &node && !seen.contains(to) {
                stack.push(to.clone());
            }
            if to == &node && !seen.contains(from) {
                stack.push(from.clone());
            }
        }
    }
    Ok(applied
        .iter()
        .filter(|candidate| seen.contains(*candidate))
        .cloned()
        .collect())
}

fn graph_dot(
    state: &State,
    selected: &[String],
    edge_labels: bool,
    reduce: bool,
    lines: Option<usize>,
) -> Result<String> {
    let mut edges = graph_edges(state, selected, lines)?;
    if reduce {
        edges = reduce_graph_edges(&edges);
    }
    let mut out = String::new();
    out.push_str("digraph dependencies {\n");
    for patch in selected {
        out.push_str(&format!("  \"{patch}\";\n"));
    }
    for Edge { from, to, files } in edges {
        if edge_labels {
            let label = files
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join("\\n");
            out.push_str(&format!("  \"{from}\" -> \"{to}\" [label=\"{label}\"];\n"));
        } else {
            out.push_str(&format!("  \"{from}\" -> \"{to}\";\n"));
        }
    }
    out.push_str("}\n");
    Ok(out)
}

fn graph_edges(state: &State, patches: &[String], lines: Option<usize>) -> Result<Vec<Edge>> {
    if let Some(lines) = lines {
        return graph_line_edges(state, patches, lines);
    }
    let patch_files: Vec<BTreeSet<PathBuf>> = patches
        .iter()
        .map(|patch| {
            state
                .tracked_files(patch)
                .map(|files| files.into_iter().collect())
        })
        .collect::<Result<_>>()?;
    let mut edges = Vec::new();
    for (i, left) in patches.iter().enumerate() {
        let left_files = &patch_files[i];
        for (j, right) in patches.iter().enumerate().skip(i + 1) {
            let shared: Vec<_> = left_files.intersection(&patch_files[j]).cloned().collect();
            if !shared.is_empty() {
                edges.push(Edge {
                    from: right.clone(),
                    to: left.clone(),
                    files: shared,
                });
            }
        }
    }
    Ok(edges)
}

fn graph_line_edges(state: &State, patches: &[String], context_lines: usize) -> Result<Vec<Edge>> {
    let patch_files: Vec<BTreeSet<PathBuf>> = patches
        .iter()
        .map(|patch| {
            state
                .tracked_files(patch)
                .map(|files| files.into_iter().collect())
        })
        .collect::<Result<_>>()?;
    let mut edges = Vec::new();
    let mut files_seen = HashMap::<PathBuf, Vec<usize>>::new();
    let mut ranges = GraphRangeCache::new();
    for (idx, patch) in patches.iter().enumerate() {
        for file in &patch_files[idx] {
            if let Some(previous) = files_seen.get(file) {
                let mut conflict = None;
                for prev_idx in previous.iter().rev() {
                    let current_ranges = graph_patch_ranges(
                        state,
                        patches,
                        &patch_files,
                        &mut ranges,
                        idx,
                        file,
                        context_lines,
                    )?;
                    let previous_ranges = graph_patch_ranges(
                        state,
                        patches,
                        &patch_files,
                        &mut ranges,
                        *prev_idx,
                        file,
                        context_lines,
                    )?;
                    // Does this patch change lines the earlier one produced?
                    if line_ranges_overlap(&current_ranges.old, &previous_ranges.new) {
                        conflict = Some(*prev_idx);
                        break;
                    }
                }
                if let Some(prev_idx) = conflict {
                    edges.push(Edge {
                        from: patch.clone(),
                        to: patches[prev_idx].clone(),
                        files: vec![file.clone()],
                    });
                }
            }
            files_seen.entry(file.clone()).or_default().push(idx);
        }
    }
    Ok(edges)
}

#[derive(Clone)]
struct LineRange {
    start: usize,
    end: usize,
}

/// A patch's hunks for one file, as line ranges in the old and new file.
#[derive(Clone)]
struct HunkRanges {
    old: Vec<LineRange>,
    new: Vec<LineRange>,
}

type GraphRangeCache = HashMap<(usize, PathBuf), HunkRanges>;

/// `from` depends on `to` because both change `files`.
#[derive(Clone)]
struct Edge {
    from: String,
    to: String,
    files: Vec<PathBuf>,
}

fn graph_patch_ranges(
    state: &State,
    patches: &[String],
    patch_files: &[BTreeSet<PathBuf>],
    cache: &mut GraphRangeCache,
    idx: usize,
    file: &PathBuf,
    context_lines: usize,
) -> Result<HunkRanges> {
    let key = (idx, file.clone());
    if let Some(ranges) = cache.get(&key) {
        return Ok(ranges.clone());
    }
    let old = state.backup_dir(&patches[idx]).join(file);
    let next = patch_files
        .iter()
        .enumerate()
        .skip(idx + 1)
        .find(|(_, files)| files.contains(file))
        .map(|(next_idx, _)| state.backup_dir(&patches[next_idx]).join(file))
        .unwrap_or_else(|| state.root.join(file));
    let old_arg = if old.metadata().map(|m| m.len() == 0).unwrap_or(true) {
        Path::new("/dev/null")
    } else {
        old.as_path()
    };
    let new_arg = if next.metadata().map(|m| m.len() == 0).unwrap_or(true) {
        Path::new("/dev/null")
    } else {
        next.as_path()
    };
    let output = diff_command()
        .arg(format!("-U{context_lines}"))
        .arg(old_arg)
        .arg(new_arg)
        .output()
        .context("failed to run diff")?;
    let parsed = parse_diff_ranges(&String::from_utf8_lossy(&output.stdout));
    cache.insert(key, parsed.clone());
    Ok(parsed)
}

fn parse_diff_ranges(diff: &str) -> HunkRanges {
    let mut new_ranges = Vec::new();
    let mut old_ranges = Vec::new();
    for line in diff.lines() {
        if let Some((old_start, old_len, new_start, new_len)) = parse_hunk_header(line) {
            old_ranges.push(LineRange {
                start: old_start,
                end: old_start + old_len,
            });
            new_ranges.push(LineRange {
                start: new_start,
                end: new_start + new_len,
            });
        }
    }
    HunkRanges {
        old: old_ranges,
        new: new_ranges,
    }
}

fn parse_hunk_header(line: &str) -> Option<(usize, usize, usize, usize)> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(" +")?;
    let (new, _) = rest.split_once(" @@")?;
    let (old_start, old_len) = parse_hunk_range(old)?;
    let (new_start, new_len) = parse_hunk_range(new)?;
    Some((old_start, old_len, new_start, new_len))
}

fn parse_hunk_range(range: &str) -> Option<(usize, usize)> {
    if let Some((start, len)) = range.split_once(',') {
        Some((start.parse().ok()?, len.parse().ok()?))
    } else {
        Some((range.parse().ok()?, 1))
    }
}

fn line_ranges_overlap(left: &[LineRange], right: &[LineRange]) -> bool {
    left.iter()
        .any(|a| right.iter().any(|b| a.start < b.end && b.start < a.end))
}

/// Drop edges implied by a longer path (`--reduce`).
fn reduce_graph_edges(edges: &[Edge]) -> Vec<Edge> {
    edges
        .iter()
        .filter(|edge| !has_alternate_path(edges, edge))
        .cloned()
        .collect()
}

/// Whether `edge.to` is reachable from `edge.from` without using `edge`.
fn has_alternate_path(edges: &[Edge], edge: &Edge) -> bool {
    let mut stack: Vec<&str> = edges
        .iter()
        .filter(|other| other.from == edge.from && other.to != edge.to)
        .map(|other| other.to.as_str())
        .collect();
    let mut seen = HashSet::new();
    while let Some(node) = stack.pop() {
        if node == edge.to {
            return true;
        }
        if seen.insert(node) {
            stack.extend(
                edges
                    .iter()
                    .filter(|next| next.from == node)
                    .map(|next| next.to.as_str()),
            );
        }
    }
    false
}
