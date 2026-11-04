use crate::cli::usage;
use anyhow::{Context, Result, bail};
use lexopt::prelude::*;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::Component;
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) fn cmd_setup(args: Vec<OsString>) -> Result<()> {
    let mut dest_dir = PathBuf::from(".");
    let mut source_dir = PathBuf::from(".");
    let mut source = None::<PathBuf>;
    let mut parser = lexopt::Parser::from_args(args);
    while let Some(arg) = parser.next()? {
        match arg {
            Short('d') => dest_dir = PathBuf::from(parser.value()?),
            Long("sourcedir") => source_dir = PathBuf::from(parser.value()?),
            Value(v) => source = Some(PathBuf::from(v)),
            _ => {}
        }
    }
    let source = source.with_context(|| usage("setup"))?;
    let text = fs::read_to_string(&source)?;
    let archive = setup_header_value(&text, "Source");
    let unpack_dir = setup_header_value(&text, "Sourcedir");
    let patchdir = setup_header_value(&text, "Patchdir");
    fs::create_dir_all(&dest_dir)?;
    if let Some(unpack_dir) = &unpack_dir
        && unpack_dir != "."
        && dest_dir.join(unpack_dir).exists()
    {
        bail!("Directory {unpack_dir} exists");
    }
    if let Some(patchdir) = &patchdir
        && dest_dir.join(patchdir).exists()
    {
        bail!("Directory {patchdir} exists");
    }
    if let Some(archive) = &archive {
        let archive_path = source_dir.join(archive);
        if !archive_path.exists() {
            bail!("File {archive} not found");
        }
        let archive_display = if source_dir == Path::new(".") {
            archive.to_string()
        } else {
            source_dir.join(archive).display().to_string()
        };
        println!("Unpacking archive {archive_display}");
        let archive_path = fs::canonicalize(archive_path)?;
        let unpack_dest = unpack_dir.as_deref().unwrap_or(".");
        fs::create_dir_all(dest_dir.join(unpack_dest))?;
        let status = setup_unpack_command(&archive_path, &dest_dir, unpack_dest)?.status()?;
        if !status.success() {
            bail!("Failed to unpack archive {archive}");
        }
    }
    if let Some(patchdir) = patchdir {
        let dir = dest_dir.join(patchdir);
        if dir.is_dir() {
            link_setup_patchdir(&dir, &source_dir, &source)?;
        }
    }
    Ok(())
}

/// Point `dir`'s patches and series at the setup sources and record the names
/// in its quilt db, falling back to quilt_patches/quilt_series when the
/// unpacked tree already has its own.
fn link_setup_patchdir(dir: &Path, source_dir: &Path, series: &Path) -> Result<()> {
    let display = |name: &str| {
        let path = dir.join(name);
        path.strip_prefix(".")
            .unwrap_or(&path)
            .display()
            .to_string()
    };
    let mut names = ("patches", "series");
    let mut taken = false;
    if dir.join(names.0).exists() {
        println!("Directory {} exists", display(names.0));
        taken = true;
    }
    if dir.join(names.1).exists() {
        println!("File {} exists", display(names.1));
        taken = true;
    }
    if taken {
        println!("Trying alternative patches and series names...");
        names = ("quilt_patches", "quilt_series");
        if dir.join(names.0).exists() || dir.join(names.1).exists() {
            bail!("Directory {} exists", display(names.0));
        }
    }
    let (patches, series_name) = names;
    create_relative_symlink(source_dir, &dir.join(patches))?;
    create_relative_symlink(series, &dir.join(series_name))?;
    let pc = dir.join(env::var("QUILT_PC").unwrap_or_else(|_| ".pc".to_string()));
    fs::create_dir_all(&pc)?;
    for (file, value) in [
        (".version", "2"),
        (".quilt_patches", patches),
        (".quilt_series", series_name),
    ] {
        if !pc.join(file).exists() {
            fs::write(pc.join(file), format!("{value}\n"))?;
        }
    }
    Ok(())
}

/// `ln -s` with a target relative to the link, like quilt's `create_symlink`.
fn create_relative_symlink(target: &Path, link: &Path) -> Result<()> {
    let cwd = env::current_dir()?;
    let target = if target.is_absolute() || link.is_absolute() {
        cwd.join(target)
    } else {
        let target = normalize_lexically(&cwd.join(target));
        let link = normalize_lexically(&cwd.join(link));
        let link_dir = link.parent().unwrap_or(Path::new("/"));
        let common = target
            .components()
            .zip(link_dir.components())
            .take_while(|(a, b)| a == b)
            .count();
        let mut relative: PathBuf = link_dir.components().skip(common).map(|_| "..").collect();
        relative.extend(target.components().skip(common));
        if relative.as_os_str().is_empty() {
            relative.push(".");
        }
        relative
    };
    std::os::unix::fs::symlink(&target, link)
        .with_context(|| format!("failed to create symlink {}", link.display()))
}

fn normalize_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn setup_header_value(text: &str, key: &str) -> Option<String> {
    let prefix = format!("# {key}:");
    text.lines().find_map(|line| {
        line.strip_prefix(&prefix)
            .map(|value| value.trim().to_string())
    })
}

fn setup_unpack_command(
    archive_path: &Path,
    dest_dir: &Path,
    unpack_dest: &str,
) -> Result<Command> {
    let mut cmd = if archive_path.extension().is_some_and(|ext| ext == "zip") {
        let mut cmd = Command::new("unzip");
        cmd.arg("-qqo")
            .arg(archive_path)
            .arg("-d")
            .arg(dest_dir.join(unpack_dest));
        cmd
    } else if archive_path.extension().is_some_and(|ext| ext == "7z") {
        let mut cmd = Command::new("7z");
        cmd.arg("x")
            .arg("-bd")
            .arg(archive_path)
            .arg(format!("-o{}", dest_dir.join(unpack_dest).display()));
        cmd
    } else {
        let mut cmd = Command::new("tar");
        cmd.current_dir(dest_dir.join(unpack_dest))
            .arg("xf")
            .arg(archive_path);
        cmd
    };
    if archive_path
        .extension()
        .is_some_and(|ext| ext == "zip" || ext == "7z")
    {
        cmd.current_dir(dest_dir);
    }
    Ok(cmd)
}
