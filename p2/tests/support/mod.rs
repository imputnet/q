use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) fn tempdir(name: &str) -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    for attempt in 0..100 {
        let dir = std::env::temp_dir().join(format!(
            "p2-{name}-{stamp}-{}-{attempt}",
            std::process::id()
        ));
        match fs::create_dir(&dir) {
            Ok(()) => return dir,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => panic!("failed to create {}: {error}", dir.display()),
        }
    }
    panic!("failed to create unique tempdir for {name}");
}

pub(crate) fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let source = entry.path();
        let target = to.join(entry.file_name());
        if source.is_dir() {
            copy_dir(&source, &target);
        } else {
            fs::copy(&source, &target).unwrap();
        }
    }
}

pub(crate) fn run_patch(
    binary: &Path,
    dir: &Path,
    patch: &str,
    args: &[&str],
) -> std::process::Output {
    run_patch_with_env(binary, dir, patch, args, &[])
}

pub(crate) fn run_patch_with_env(
    binary: &Path,
    dir: &Path,
    patch: &str,
    args: &[&str],
    envs: &[(&str, &str)],
) -> std::process::Output {
    let mut child = Command::new(binary)
        .args(args)
        .current_dir(dir)
        .env_remove("PATCH_VERSION_CONTROL")
        .env_remove("VERSION_CONTROL")
        .env_remove("SIMPLE_BACKUP_SUFFIX")
        .env_remove("POSIXLY_CORRECT")
        .env_remove("PATCH_GET")
        .env_remove("QUOTING_STYLE")
        .envs(envs.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    match std::io::Write::write_all(child.stdin.as_mut().unwrap(), patch.as_bytes()) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {}
        Err(error) => panic!("failed to write patch to stdin: {error}"),
    }
    child.wait_with_output().unwrap()
}

#[cfg(unix)]
pub(crate) fn run_patch_with_umask(
    binary: &Path,
    dir: &Path,
    patch: &str,
    args: &[&str],
    umask: &str,
) -> std::process::Output {
    let command = format!("umask {umask}; exec \"$1\" {}", args.join(" "));
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .arg("sh")
        .arg(binary)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::io::Write::write_all(child.stdin.as_mut().unwrap(), patch.as_bytes()).unwrap();
    child.wait_with_output().unwrap()
}

pub(crate) fn reference_patch() -> PathBuf {
    std::env::var_os("P2_REFERENCE_PATCH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("patch"))
}

pub(crate) fn patch_under_test() -> PathBuf {
    std::env::var_os("P2_UNDER_TEST")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_p2")))
}

pub(crate) fn run_patch_no_input(binary: &Path, args: &[&str]) -> std::process::Output {
    Command::new(binary)
        .args(args)
        .env_remove("PATCH_GET")
        .env_remove("POSIXLY_CORRECT")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

pub(crate) fn assert_matches_gnu_patch(
    seed: &[(&str, &str)],
    patch: &str,
    args: &[&str],
    expected_files: &[&str],
) {
    assert_matches_gnu_patch_with_env(seed, patch, args, &[], expected_files);
}

pub(crate) fn assert_matches_gnu_patch_with_env(
    seed: &[(&str, &str)],
    patch: &str,
    args: &[&str],
    envs: &[(&str, &str)],
    expected_files: &[&str],
) {
    let root = tempdir("parity");
    let gnu_dir = root.join("gnu");
    let p2_dir = root.join("p2");
    fs::create_dir(&gnu_dir).unwrap();
    for (path, content) in seed {
        let path = gnu_dir.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    copy_dir(&gnu_dir, &p2_dir);

    let reference = reference_patch();
    let under_test = patch_under_test();
    let gnu = run_patch_with_env(&reference, &gnu_dir, patch, args, envs);
    let ours = run_patch_with_env(&under_test, &p2_dir, patch, args, envs);
    assert_eq!(
        gnu.status.success(),
        ours.status.success(),
        "gnu stderr: {}\np2 stderr: {}",
        String::from_utf8_lossy(&gnu.stderr),
        String::from_utf8_lossy(&ours.stderr)
    );

    for file in expected_files {
        let gnu_path = gnu_dir.join(file);
        let p2_path = p2_dir.join(file);
        assert_eq!(
            gnu_path.exists(),
            p2_path.exists(),
            "{file} existence differs"
        );
        if gnu_path.exists() {
            assert_eq!(
                fs::read(&gnu_path).unwrap(),
                fs::read(&p2_path).unwrap(),
                "{file} content differs"
            );
        }
    }

    fs::remove_dir_all(root).unwrap();
}
