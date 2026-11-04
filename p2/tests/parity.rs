use std::fs;
use std::path::Path;

mod support;

use support::*;

use filetime::{FileTime, set_file_mtime};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::fs::symlink;

#[test]
fn version_options_exit_successfully_like_gnu_patch() {
    for args in [["-v"], ["--version"]] {
        let reference = reference_patch();
        let under_test = patch_under_test();
        let gnu = run_patch_no_input(&reference, &args);
        let ours = run_patch_no_input(&under_test, &args);
        assert_eq!(
            gnu.status.success(),
            ours.status.success(),
            "gnu stderr: {}\np2 stderr: {}",
            String::from_utf8_lossy(&gnu.stderr),
            String::from_utf8_lossy(&ours.stderr)
        );
        for output in [gnu.stdout, ours.stdout] {
            let text = String::from_utf8_lossy(&output);
            assert!(text.contains("patch"), "{text}");
            assert!(text.contains("GPL"), "{text}");
            assert!(text.contains("NO WARRANTY"), "{text}");
        }
    }
}

#[test]
fn help_option_exits_successfully_like_gnu_patch() {
    let reference = reference_patch();
    let under_test = patch_under_test();
    let gnu = run_patch_no_input(&reference, &["--help"]);
    let ours = run_patch_no_input(&under_test, &["--help"]);
    assert_eq!(gnu.status.success(), ours.status.success());
    assert!(String::from_utf8_lossy(&ours.stdout).contains("Usage:"));
}

#[test]
fn short_help_option_rejects_like_gnu_patch() {
    let reference = reference_patch();
    let under_test = patch_under_test();
    let gnu = run_patch_no_input(&reference, &["-h"]);
    let ours = run_patch_no_input(&under_test, &["-h"]);
    assert_eq!(gnu.status.code(), ours.status.code());
}

#[test]
fn quoting_style_changes_file_action_output_like_gnu_patch() {
    let root = tempdir("quoting-style");
    let gnu_dir = root.join("gnu");
    let p2_dir = root.join("p2");
    fs::create_dir(&gnu_dir).unwrap();
    fs::write(gnu_dir.join("file with space"), "one\n").unwrap();
    copy_dir(&gnu_dir, &p2_dir);
    let patch = "--- \"file with space\"\n+++ \"file with space\"\n@@ -1 +1 @@\n-one\n+two\n";

    let reference = reference_patch();
    let under_test = patch_under_test();
    for args in [
        ["--quoting-style=shell-always", "-p0"],
        ["--quoting-style=shell-a", "-p0"],
        ["--quoting-style=locale", "-p0"],
        ["--quoting-style=c", "-p0"],
        ["--quoting-style=clocale", "-p0"],
        ["--quoting-style=c-m", "-p0"],
        ["--quoting-style=e", "-p0"],
    ] {
        fs::write(gnu_dir.join("file with space"), "one\n").unwrap();
        fs::write(p2_dir.join("file with space"), "one\n").unwrap();
        let gnu = run_patch(&reference, &gnu_dir, patch, &args);
        let ours = run_patch(&under_test, &p2_dir, patch, &args);
        assert_eq!(gnu.status.success(), ours.status.success());
        assert_eq!(
            String::from_utf8_lossy(&gnu.stdout),
            String::from_utf8_lossy(&ours.stdout)
        );
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn quoting_style_env_changes_file_action_output_like_gnu_patch() {
    let root = tempdir("quoting-style-env");
    let gnu_dir = root.join("gnu");
    let p2_dir = root.join("p2");
    fs::create_dir(&gnu_dir).unwrap();
    fs::write(gnu_dir.join("f"), "one\n").unwrap();
    copy_dir(&gnu_dir, &p2_dir);
    let patch = "--- f\n+++ f\n@@ -1 +1 @@\n-one\n+two\n";

    let reference = reference_patch();
    let under_test = patch_under_test();
    let gnu = run_patch_with_env(
        &reference,
        &gnu_dir,
        patch,
        &["-p0"],
        &[("QUOTING_STYLE", "shell-always")],
    );
    let ours = run_patch_with_env(
        &under_test,
        &p2_dir,
        patch,
        &["-p0"],
        &[("QUOTING_STYLE", "shell-always")],
    );
    assert_eq!(gnu.status.success(), ours.status.success());
    assert_eq!(
        String::from_utf8_lossy(&gnu.stdout),
        String::from_utf8_lossy(&ours.stdout)
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_quoting_style_rejects_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("file", "one\n")],
        "--- file\n+++ file\n@@ -1 +1 @@\n-one\n+two\n",
        &["--quoting-style=bad", "-p0"],
        &["file"],
    );
}

#[test]
fn ambiguous_quoting_style_abbreviations_reject_like_gnu_patch() {
    for style in ["l", "sh", "shell-e"] {
        let arg = format!("--quoting-style={style}");
        assert_matches_gnu_patch(
            &[("file", "one\n")],
            "--- file\n+++ file\n@@ -1 +1 @@\n-one\n+two\n",
            &[&arg, "-p0"],
            &["file"],
        );
    }
}

#[test]
fn unambiguous_long_option_abbreviations_match_gnu_patch() {
    assert_matches_gnu_patch(
        &[("file", "one\n")],
        "--- file\n+++ file\n@@ -1 +1 @@\n-one\n+two\n",
        &["--dry-ru", "-p0"],
        &["file"],
    );
    assert_matches_gnu_patch(
        &[("file", "one\n")],
        "--- file\n+++ file\n@@ -1 +1 @@\n-one\n+two\n",
        &["--quoting-st", "shell", "-p0"],
        &["file"],
    );
    assert_matches_gnu_patch(
        &[("file", "one\n")],
        "--- file\n+++ file\n@@ -1 +1 @@\n-one\n+two\n",
        &["--read-o", "ignore", "-p0"],
        &["file"],
    );
}

#[test]
fn ambiguous_long_option_abbreviations_reject_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("file", "one\n")],
        "--- file\n+++ file\n@@ -1 +1 @@\n-one\n+two\n",
        &["--rej", "file.rej", "-p0"],
        &["file"],
    );
}

#[test]
fn applies_simple_unified_change_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("file", "one\ntwo\nthree\n")],
        "--- file\n+++ file\n@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n",
        &["-p0"],
        &["file"],
    );
}

#[test]
fn applies_hunk_with_ignore_whitespace_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("file", "alpha\t\tbeta  \ngamma\n")],
        "--- file\n+++ file\n@@ -1,2 +1,2 @@\n-alpha beta\n+ALPHA BETA\n gamma\n",
        &["-p0", "--ignore-whitespace"],
        &["file"],
    );
}

#[test]
fn short_ignore_whitespace_option_matches_gnu_patch() {
    assert_matches_gnu_patch(
        &[("file", "alpha beta\n")],
        "--- file\n+++ file\n@@ -1 +1 @@\n-alpha  beta\n+ALPHA BETA\n",
        &["-p0", "-l"],
        &["file"],
    );
}

#[test]
fn clustered_short_flag_options_match_gnu_patch() {
    assert_matches_gnu_patch(
        &[("file", "one\n")],
        "--- file\n+++ file\n@@ -1 +1 @@\n-one\n+two\n",
        &["-fs", "-p0"],
        &["file"],
    );
    assert_matches_gnu_patch(
        &[("file", "two\n")],
        "--- file\n+++ file\n@@ -1 +1 @@\n-one\n+two\n",
        &["-Rfs", "-p0"],
        &["file"],
    );
}

#[test]
fn clustered_short_options_stop_at_attached_operand_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("file", "one\n")],
        "--- file\n+++ file\n@@ -1 +1 @@\n-one\n+two\n",
        &["-fsp0"],
        &["file"],
    );
    assert_matches_gnu_patch(
        &[("file", "one\n")],
        "--- file\n+++ file\n@@ -1 +1 @@\n-one\n+two\n",
        &["-fbBbackup/", "-p0"],
        &["file", "backup/file"],
    );
}

#[test]
fn get_option_allows_plain_patch_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("file", "one\n")],
        "--- file\n+++ file\n@@ -1 +1 @@\n-one\n+two\n",
        &["-g0", "-p0"],
        &["file"],
    );
    assert_matches_gnu_patch(
        &[("file", "one\n")],
        "--- file\n+++ file\n@@ -1 +1 @@\n-one\n+two\n",
        &["--get=-1", "-p0"],
        &["file"],
    );
}

#[test]
fn oversized_numeric_options_match_gnu_patch() {
    let huge = "999999999999999999999999999999";
    assert_matches_gnu_patch(
        &[("a/file", "one\n")],
        "--- a/file\n+++ a/file\n@@ -1 +1 @@\n-one\n+two\n",
        &["-p", huge],
        &["a/file"],
    );
    assert_matches_gnu_patch(
        &[("file", "one\n")],
        "--- file\n+++ file\n@@ -1 +1 @@\n-one\n+two\n",
        &["-F", huge, "-p0"],
        &["file"],
    );

    let reference = reference_patch();
    let under_test = patch_under_test();
    for args in [["-g", huge], ["-x", huge]] {
        let gnu = run_patch_no_input(&reference, &args);
        let ours = run_patch_no_input(&under_test, &args);
        assert_eq!(gnu.status.success(), ours.status.success());
    }
}

#[test]
fn patch_get_env_allows_plain_patch_like_gnu_patch() {
    assert_matches_gnu_patch_with_env(
        &[("file", "one\n")],
        "--- file\n+++ file\n@@ -1 +1 @@\n-one\n+two\n",
        &["-p0"],
        &[("PATCH_GET", "1")],
        &["file"],
    );
}

#[test]
fn matching_prereq_allows_patch_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "rev1\none\n")],
        "Prereq: rev1\n--- f\n+++ f\n@@ -2 +2 @@\n-one\n+two\n",
        &["-p0"],
        &["f"],
    );
}

#[test]
fn missing_prereq_aborts_by_default_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "rev0\none\n")],
        "Prereq: rev1\n--- f\n+++ f\n@@ -2 +2 @@\n-one\n+two\n",
        &["-p0"],
        &["f"],
    );
}

#[test]
fn missing_prereq_aborts_in_batch_mode_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "rev0\none\n")],
        "Prereq: rev1\n--- f\n+++ f\n@@ -2 +2 @@\n-one\n+two\n",
        &["-t", "-p0"],
        &["f"],
    );
}

#[test]
fn force_ignores_missing_prereq_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "rev0\none\n")],
        "Prereq: rev1\n--- f\n+++ f\n@@ -2 +2 @@\n-one\n+two\n",
        &["-f", "-p0"],
        &["f"],
    );
}

#[test]
fn invalid_patch_get_env_rejects_like_gnu_patch() {
    assert_matches_gnu_patch_with_env(
        &[("file", "one\n")],
        "--- file\n+++ file\n@@ -1 +1 @@\n-one\n+two\n",
        &["-p0"],
        &[("PATCH_GET", "abc")],
        &["file"],
    );
}

#[test]
fn zero_debug_option_allows_plain_patch_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("file", "one\n")],
        "--- file\n+++ file\n@@ -1 +1 @@\n-one\n+two\n",
        &["-x0", "-p0"],
        &["file"],
    );
    assert_matches_gnu_patch(
        &[("file", "one\n")],
        "--- file\n+++ file\n@@ -1 +1 @@\n-one\n+two\n",
        &["--debug=0", "-p0"],
        &["file"],
    );
}

#[test]
fn ifdef_option_marks_replacements_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("file", "one\ntwo\nthree\n")],
        "--- file\n+++ file\n@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n",
        &["-D", "FEATURE", "-p0"],
        &["file"],
    );
    assert_matches_gnu_patch(
        &[("file", "one\ntwo\nthree\n")],
        "--- file\n+++ file\n@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n",
        &["--ifdef=FEATURE", "-p0"],
        &["file"],
    );
}

#[test]
fn ifdef_option_marks_additions_and_removals_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("file", "one\ntwo\nthree\n")],
        "--- file\n+++ file\n@@ -1,3 +1,4 @@\n one\n+new\n two\n three\n",
        &["-DFEATURE", "-p0"],
        &["file"],
    );
    assert_matches_gnu_patch(
        &[("file", "one\ntwo\nthree\n")],
        "--- file\n+++ file\n@@ -1,3 +1,2 @@\n one\n-two\n three\n",
        &["-DFEATURE", "-p0"],
        &["file"],
    );
}

#[test]
fn merge_option_writes_conflict_markers_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "ours\n")],
        "--- f\n+++ f\n@@ -1 +1 @@\n-base\n+theirs\n",
        &["--merge", "-f", "-p0"],
        &["f"],
    );
}

#[test]
fn short_merge_option_writes_conflict_markers_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "ours\n")],
        "--- f\n+++ f\n@@ -1 +1 @@\n-base\n+theirs\n",
        &["-m", "-f", "-p0"],
        &["f"],
    );
}

#[test]
fn merge_diff3_option_writes_original_conflict_markers_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "ours\n")],
        "--- f\n+++ f\n@@ -1 +1 @@\n-base\n+theirs\n",
        &["--merge=diff3", "-f", "-p0"],
        &["f"],
    );
}

#[test]
fn invalid_debug_option_rejects_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("file", "one\n")],
        "--- file\n+++ file\n@@ -1 +1 @@\n-one\n+two\n",
        &["-xabc", "-p0"],
        &["file"],
    );
}

#[test]
fn ignore_whitespace_rejects_missing_blank_runs_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("file", "alphabeta\n")],
        "--- file\n+++ file\n@@ -1 +1 @@\n-alpha beta\n+ALPHA BETA\n",
        &["-p0", "--ignore-whitespace", "-f"],
        &["file", "file.rej"],
    );
}

#[test]
fn creates_file_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[],
        "--- /dev/null\n+++ new-file\n@@ -0,0 +1,2 @@\n+one\n+two\n",
        &["-p0"],
        &["new-file"],
    );
}

#[test]
fn empty_patch_with_target_operand_exits_successfully_like_gnu_patch() {
    assert_matches_gnu_patch(&[("file", "one\n")], "", &["file"], &["file"]);
}

#[test]
fn forced_creation_against_existing_file_fails_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "one\n")],
        "--- /dev/null\n+++ f\n@@ -0,0 +1 @@\n+one\n",
        &["-f"],
        &["f", "f.rej"],
    );
}

#[test]
fn deletes_file_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("old-file", "one\ntwo\n")],
        "--- old-file\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-one\n-two\n",
        &["-p0"],
        &["old-file"],
    );
}

#[test]
fn batch_reverses_emptying_patch_against_empty_file_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "")],
        "--- f\n+++ /dev/null\n@@ -1 +0,0 @@\n-f\n",
        &["-p0", "-t"],
        &["f"],
    );
}

#[test]
fn skips_missing_deletion_without_force_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[],
        "--- f\n+++ /dev/null\n@@ -1 +0,0 @@\n-one\n",
        &["-p0"],
        &["f", "f.rej"],
    );
}

#[test]
fn forced_missing_deletion_fails_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[],
        "--- f\n+++ /dev/null\n@@ -1 +0,0 @@\n-one\n",
        &["-p0", "-f"],
        &["f", "f.rej"],
    );
}

#[test]
fn forced_emptying_patch_against_empty_file_fails_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f.orig", "f\n"), ("f", "")],
        "--- f.orig\n+++ f\n@@ -1 +0,0 @@\n-f\n",
        &["-f"],
        &["f", "f.rej"],
    );
}

#[test]
fn posix_deletion_patch_empties_file_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("target", "data\n")],
        "diff --git a/target b/target\ndeleted file mode 100644\nindex 1269488..0000000\n--- a/target\n+++ /dev/null\n@@ -1 +0,0 @@\n-data\n",
        &["-p1", "--posix", "-f"],
        &["target"],
    );
}

#[test]
fn posixly_correct_deletion_patch_empties_file_like_gnu_patch() {
    assert_matches_gnu_patch_with_env(
        &[("target", "data\n")],
        "diff --git a/target b/target\ndeleted file mode 100644\nindex 1269488..0000000\n--- a/target\n+++ /dev/null\n@@ -1 +0,0 @@\n-data\n",
        &["-p1", "-f"],
        &[("POSIXLY_CORRECT", "1")],
        &["target"],
    );
}

#[test]
fn refuses_to_delete_when_content_remains_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("target", "1\n2\n")],
        "--- target\n+++ /dev/null\n@@ -1 +0,0 @@\n-1\n",
        &["-p0"],
        &["target"],
    );
}

#[test]
fn refuses_timestamp_marked_deletion_when_content_remains_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("target", "1\n2\n")],
        "--- target\t2012-09-25 03:09:15\n+++ target\t1970-01-01 00:00:00\n@@ -1 +0,0 @@\n-1\n",
        &["-p0"],
        &["target"],
    );
}

#[test]
fn removes_empty_parent_directory_with_remove_empty_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("dir/file", "one\n")],
        "--- dir/file\n+++ /dev/null\n@@ -1 +0,0 @@\n-one\n",
        &["-p0", "-E"],
        &["dir", "dir/file"],
    );
}

#[test]
fn reverses_patch_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("file", "one\nTWO\nthree\n")],
        "--- file\n+++ file\n@@ -1,3 +1,3 @@\n one\n-two\n+TWO\n three\n",
        &["-p0", "-R"],
        &["file"],
    );
}

#[test]
fn strips_git_prefix_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("src/file", "alpha\nbeta\n")],
        "--- a/src/file\n+++ b/src/file\n@@ -1,2 +1,2 @@\n alpha\n-beta\n+BETA\n",
        &["-p1"],
        &["src/file"],
    );
}

#[test]
fn short_unified_option_applies_unified_patch_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "one\ntwo\n")],
        "--- f\n+++ f\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n",
        &["-u", "-p0"],
        &["f"],
    );
}

#[test]
fn long_context_option_applies_context_patch_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "one\ntwo\n")],
        "*** f\n--- f\n***************\n*** 1,2 ****\n  one\n- two\n--- 1,2 ----\n  one\n+ TWO\n",
        &["--context", "-p0"],
        &["f"],
    );
}

#[test]
fn forced_normal_rejects_unified_patch_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "one\n")],
        "--- f\n+++ f\n@@ -1 +1 @@\n-one\n+two\n",
        &["-n", "f"],
        &["f"],
    );
}

#[test]
fn forced_context_rejects_unified_patch_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "one\n")],
        "--- f\n+++ f\n@@ -1 +1 @@\n-one\n+two\n",
        &["-c", "f"],
        &["f"],
    );
}

#[test]
fn forced_ed_rejects_unified_patch_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "one\n")],
        "--- f\n+++ f\n@@ -1 +1 @@\n-one\n+two\n",
        &["-e", "f"],
        &["f"],
    );
}

#[test]
fn forced_unified_rejects_normal_patch_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "one\n")],
        "1c1\n< one\n---\n> two\n",
        &["-u", "f"],
        &["f"],
    );
}

#[test]
fn chooses_existing_old_filename_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("a", "1\n2\n4\n")],
        "--- a\n+++ b\n@@ -1,3 +1,4 @@\n 1\n 2\n+3\n 4\n",
        &["-p0"],
        &["a", "b"],
    );
}

#[test]
fn creates_using_old_filename_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[],
        "--- f\n+++ f.new\n@@ -0,0 +1 @@\n+content\n",
        &["-p0"],
        &["f", "f.new"],
    );
}

#[test]
fn applies_normal_diff_change_with_target_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "one\ntwo\n")],
        "2c2\n< two\n---\n> TWO\n",
        &["f"],
        &["f"],
    );
}

#[test]
fn applies_normal_diff_add_with_target_like_gnu_patch() {
    assert_matches_gnu_patch(&[("f", "")], "0a1\n> one\n", &["f"], &["f"]);
}

#[test]
fn applies_headerless_unified_hunk_with_target_like_gnu_patch() {
    assert_matches_gnu_patch(&[], "@@ -0,0 +1 @@\n+one\n", &["f"], &["f"]);
}

#[test]
fn applies_headerless_context_hunk_with_target_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[],
        "***************\n*** 0 ****\n--- 1 ----\n+ one\n",
        &["f"],
        &["f"],
    );
}

#[test]
fn applies_quoted_filename_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("\t  ", "one\n")],
        "--- \"\\t \\040\"\n+++ \"\\t \\040\"\n@@ -1 +1 @@\n-one\n+two\n",
        &["-p0"],
        &["\t  "],
    );
}

#[test]
fn applies_unquoted_filename_with_spaces_before_tab_timestamp_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("file with space", "one\n")],
        "--- file with space\t2009-03-13 00:00:00\n+++ file with space\t2009-03-14 00:00:00\n@@ -1 +1 @@\n-one\n+two\n",
        &["-p0"],
        &["file with space"],
    );
}

#[test]
fn invalid_quoted_filename_escapes_are_unusable_like_gnu_patch() {
    for escape in ["\\0", "\\00", "\\000", "\\400", "\\777", "\\q"] {
        let patch = format!("--- \"f{escape}g\"\n+++ \"f{escape}g\"\n@@ -1 +1 @@\n-one\n+two\n");
        let root = tempdir("invalid-quoted-filename");
        let gnu_dir = root.join("gnu");
        let p2_dir = root.join("p2");
        fs::create_dir(&gnu_dir).unwrap();
        fs::create_dir(&p2_dir).unwrap();

        let reference = reference_patch();
        let under_test = patch_under_test();
        let gnu = run_patch(&reference, &gnu_dir, &patch, &["-f", "-p0"]);
        let ours = run_patch(&under_test, &p2_dir, &patch, &["-f", "-p0"]);
        assert_eq!(
            gnu.status.success(),
            ours.status.success(),
            "escape {escape}: gnu stderr: {}\np2 stderr: {}",
            String::from_utf8_lossy(&gnu.stderr),
            String::from_utf8_lossy(&ours.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&gnu.stdout),
            String::from_utf8_lossy(&ours.stdout),
            "escape {escape} stdout differs"
        );
        assert!(!gnu_dir.join(format!("f{escape}g")).exists());
        assert!(!p2_dir.join(format!("f{escape}g")).exists());

        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn strips_trailing_crs_from_unified_patch_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("c", "1\n")],
        "--- a\r\n+++ b\r\n@@ -1 +1 @@\r\n-1\r\n+1b\r\n",
        &["c"],
        &["c"],
    );
}

#[test]
fn applies_prior_files_before_malformed_later_patch_like_gnu_patch() {
    let patch = "diff --git a/f b/f\n--- f\n+++ f\n@@ -1 +1 @@\n-1\n+1a\n@@ -5 +5 @@\n-out\n+in\ndiff --git a/g b/g\n--- g\n+++ g\n@@ -1 +1 @@\n-out\n+in\ndiff --git a/h b/h\n--- h\n+++ h\n@@ -1 +1 @@\nBAD PATCH\n";
    let root = tempdir("malformed-cleanup");
    let gnu_dir = root.join("gnu");
    let p2_dir = root.join("p2");
    fs::create_dir(&gnu_dir).unwrap();
    for (path, content) in [("f", "1\n"), ("g", ""), ("h", "")] {
        fs::write(gnu_dir.join(path), content).unwrap();
    }
    fs::write(gnu_dir.join("1.diff"), patch).unwrap();
    copy_dir(&gnu_dir, &p2_dir);

    let reference = reference_patch();
    let under_test = patch_under_test();
    let gnu = run_patch(&reference, &gnu_dir, "", &["-f", "-i", "1.diff"]);
    let ours = run_patch(&under_test, &p2_dir, "", &["-f", "-i", "1.diff"]);
    assert_eq!(
        gnu.status.code(),
        ours.status.code(),
        "gnu stderr: {}\np2 stderr: {}",
        String::from_utf8_lossy(&gnu.stderr),
        String::from_utf8_lossy(&ours.stderr)
    );
    for file in ["f", "g", "f.rej", "g.rej"] {
        let gnu_path = gnu_dir.join(file);
        let p2_path = p2_dir.join(file);
        assert_eq!(gnu_path.exists(), p2_path.exists(), "{file} existence");
        if gnu_path.exists() {
            assert_eq!(
                fs::read(&gnu_path).unwrap(),
                fs::read(&p2_path).unwrap(),
                "{file} content"
            );
        }
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_dangerous_creation_path_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[],
        "--- /dev/null\n+++ ../z\n@@ -0,0 +1 @@\n+x\n",
        &["-f", "-p0", "--dry-run"],
        &["../z"],
    );
}

#[test]
fn rejects_nul_patch_line_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[],
        "--- /dev/null\n+++ a\0b\n@@ -0,0 +1 @@\n+x\n",
        &[],
        &["ab"],
    );
}

#[test]
fn rejects_newline_filename_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[],
        "--- /dev/null\n+++ \"a\\nb\"\n@@ -0,0 +1 @@\n+x\n",
        &[],
        &["a\nb"],
    );
}

#[test]
fn permits_absolute_path_from_root_like_gnu_patch() {
    let root = tempdir("absolute-from-root");
    let target = root.join("target");
    let patch = format!(
        "--- /dev/null\n+++ {}\n@@ -0,0 +1 @@\n+x\n",
        target.display()
    );
    let reference = reference_patch();
    let under_test = patch_under_test();
    let gnu = run_patch(
        &reference,
        Path::new("/"),
        &patch,
        &["-f", "-p0", "--dry-run"],
    );
    let ours = run_patch(
        &under_test,
        Path::new("/"),
        &patch,
        &["-f", "-p0", "--dry-run"],
    );
    assert_eq!(
        gnu.status.success(),
        ours.status.success(),
        "gnu stderr: {}\np2 stderr: {}",
        String::from_utf8_lossy(&gnu.stderr),
        String::from_utf8_lossy(&ours.stderr)
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn uses_existing_source_when_target_path_is_dangerous_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "1\n")],
        "--- f\n+++ ../g\n@@ -1 +1 @@\n-1\n+1\n",
        &["-f", "-p0", "--dry-run"],
        &["f", "../g"],
    );
}

#[test]
fn applies_ed_add_with_target_like_gnu_patch() {
    assert_matches_gnu_patch(&[("f", "")], "0a\none\n.\n", &["f"], &["f"]);
}

#[test]
fn applies_normal_diff_delete_with_target_like_gnu_patch() {
    assert_matches_gnu_patch(&[("f", "one\n")], "1d0\n< one\n", &["f"], &["f"]);
}

#[test]
fn applies_index_named_normal_diff_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "one\n")],
        "Index: f\n1c1\n< one\n---\n> two\n",
        &[],
        &["f"],
    );
}

#[test]
fn applies_index_named_unified_fragment_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "one\n")],
        "Index: f\n@@ -1 +1 @@\n-one\n+two\n",
        &[],
        &["f"],
    );
}

#[test]
fn applies_index_named_context_fragment_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "one\n")],
        "Index: f\n***************\n*** 1 ****\n! one\n--- 1 ----\n! two\n",
        &[],
        &["f"],
    );
}

#[test]
fn applies_index_named_ed_fragment_like_gnu_patch() {
    assert_matches_gnu_patch(&[("f", "one\n")], "Index: f\n1c\ntwo\n.\n", &[], &["f"]);
}

#[test]
fn copies_input_for_empty_patch_with_output_like_gnu_patch() {
    assert_matches_gnu_patch(&[("a", "three\n")], "", &["-o", "b", "a"], &["a", "b"]);
}

#[test]
fn attached_short_input_option_reads_patch_file_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[
            ("f", "one\n"),
            ("p.diff", "--- f\n+++ f\n@@ -1 +1 @@\n-one\n+two\n"),
        ],
        "",
        &["-ip.diff", "-p0"],
        &["f"],
    );
}

#[test]
fn attached_short_directory_option_resolves_input_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[
            ("sub/f", "one\n"),
            ("sub/p.diff", "--- f\n+++ f\n@@ -1 +1 @@\n-one\n+two\n"),
        ],
        "",
        &["-dsub", "-ip.diff", "-p0"],
        &["sub/f"],
    );
}

#[test]
fn attached_short_output_option_writes_output_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[
            ("f", "one\n"),
            ("p.diff", "--- f\n+++ f\n@@ -1 +1 @@\n-one\n+two\n"),
        ],
        "",
        &["-oout", "f", "p.diff"],
        &["f", "out"],
    );
}

#[test]
fn long_input_option_with_equals_reads_patch_file_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[
            ("f", "one\n"),
            ("p.diff", "--- f\n+++ f\n@@ -1 +1 @@\n-one\n+two\n"),
        ],
        "",
        &["--input=p.diff", "-p0"],
        &["f"],
    );
}

#[test]
fn long_directory_option_with_equals_resolves_input_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[
            ("sub/f", "one\n"),
            ("sub/p.diff", "--- f\n+++ f\n@@ -1 +1 @@\n-one\n+two\n"),
        ],
        "",
        &["--directory=sub", "--input=p.diff", "-p0"],
        &["sub/f"],
    );
}

#[test]
fn missing_directory_option_exits_like_gnu_patch() {
    let root = tempdir("missing-directory");
    let gnu_dir = root.join("gnu");
    let p2_dir = root.join("p2");
    fs::create_dir(&gnu_dir).unwrap();
    fs::write(gnu_dir.join("file"), "one\n").unwrap();
    copy_dir(&gnu_dir, &p2_dir);
    let patch = "--- file\n+++ file\n@@ -1 +1 @@\n-one\n+two\n";

    let reference = reference_patch();
    let under_test = patch_under_test();
    let gnu = run_patch(&reference, &gnu_dir, patch, &["-d", "missing", "-p0"]);
    let ours = run_patch(&under_test, &p2_dir, patch, &["-d", "missing", "-p0"]);
    assert_eq!(gnu.status.code(), ours.status.code());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn long_output_option_with_equals_writes_output_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[
            ("f", "one\n"),
            ("p.diff", "--- f\n+++ f\n@@ -1 +1 @@\n-one\n+two\n"),
        ],
        "",
        &["--output=out", "f", "p.diff"],
        &["f", "out"],
    );
}

#[test]
fn creates_backup_for_failed_hunk_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("a", "three\n")],
        "--- a\n+++ a\n@@ -1 +1 @@\n-one\n+two\n",
        &["-b", "-B", "backup/", "a"],
        &["a", "backup/a", "a.rej"],
    );
}

#[test]
fn creates_default_backup_for_successful_hunk_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "one\n")],
        "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-one\n+two\n",
        &["-b", "-p1"],
        &["f", "f.orig"],
    );
}

#[test]
fn creates_prefixed_backup_for_successful_hunk_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "one\n")],
        "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-one\n+two\n",
        &["-b", "-B", "prefix.", "-p1"],
        &["f", "prefix.f"],
    );
}

#[test]
fn creates_custom_suffix_backup_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "one\n")],
        "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-one\n+two\n",
        &["-b", "-z", ".suffix", "-p1"],
        &["f", "f.suffix"],
    );
}

#[test]
fn creates_basename_prefixed_backup_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("d/f", "one\n")],
        "--- a/d/f\n+++ b/d/f\n@@ -1 +1 @@\n-one\n+two\n",
        &["-b", "-Y", ".orig/", "-p1"],
        &["d/f", "d/.orig/f"],
    );
}

#[test]
fn attached_short_backup_affix_options_match_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "one\n")],
        "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-one\n+two\n",
        &["-b", "-Bpre.", "-z.bak", "-Ybase.", "-p1"],
        &["f", "pre.base.f.bak"],
    );
}

#[test]
fn long_backup_affix_options_with_equals_match_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "one\n")],
        "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-one\n+two\n",
        &[
            "--backup",
            "--prefix=pre.",
            "--suffix=.bak",
            "--basename-prefix=base.",
            "-p1",
        ],
        &["f", "pre.base.f.bak"],
    );
}

#[test]
fn version_control_env_creates_numbered_backup_like_gnu_patch() {
    assert_matches_gnu_patch_with_env(
        &[("f", "one\n")],
        "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-one\n+two\n",
        &["-b", "-p1"],
        &[("VERSION_CONTROL", "numbered")],
        &["f", "f.~1~"],
    );
}

#[test]
fn version_control_none_and_off_create_numbered_backups_like_gnu_patch() {
    for style in ["none", "off"] {
        assert_matches_gnu_patch(
            &[("f", "one\n")],
            "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-one\n+two\n",
            &["-b", "-V", style, "-p1"],
            &["f", "f.~1~"],
        );
        assert_matches_gnu_patch_with_env(
            &[("f", "one\n")],
            "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-one\n+two\n",
            &["-b", "-p1"],
            &[("VERSION_CONTROL", style)],
            &["f", "f.~1~"],
        );
        assert_matches_gnu_patch_with_env(
            &[("f", "one\n")],
            "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-one\n+two\n",
            &["-b", "-p1"],
            &[("PATCH_VERSION_CONTROL", style)],
            &["f", "f.~1~"],
        );
    }
}

#[test]
fn abbreviated_version_control_styles_match_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "one\n")],
        "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-one\n+two\n",
        &["-b", "-Vs", "-p1"],
        &["f", "f.orig"],
    );
    assert_matches_gnu_patch(
        &[("f", "one\n")],
        "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-one\n+two\n",
        &["-b", "-Vnu", "-p1"],
        &["f", "f.~1~"],
    );
    assert_matches_gnu_patch(
        &[("f", "one\n"), ("f.~1~", "prior\n")],
        "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-one\n+two\n",
        &["-b", "-Ve", "-p1"],
        &["f", "f.~1~", "f.~2~"],
    );
    assert_matches_gnu_patch_with_env(
        &[("f", "one\n")],
        "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-one\n+two\n",
        &["-b", "-p1"],
        &[("VERSION_CONTROL", "o")],
        &["f", "f.~1~"],
    );
}

#[test]
fn ambiguous_version_control_style_rejects_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "one\n")],
        "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-one\n+two\n",
        &["-b", "-Vn", "-p1"],
        &["f"],
    );
}

#[test]
fn patch_version_control_takes_precedence_like_gnu_patch() {
    assert_matches_gnu_patch_with_env(
        &[("f", "one\n")],
        "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-one\n+two\n",
        &["-b", "-p1"],
        &[
            ("VERSION_CONTROL", "numbered"),
            ("PATCH_VERSION_CONTROL", "simple"),
        ],
        &["f", "f.orig"],
    );
}

#[test]
fn cli_version_control_existing_uses_existing_numbered_backup_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "one\n"), ("f.~1~", "prior\n")],
        "--- a/f\n+++ b/f\n@@ -1 +1 @@\n-one\n+two\n",
        &["-b", "-V", "existing", "-p1"],
        &["f", "f.~1~", "f.~2~"],
    );
}

#[test]
fn posix_disables_default_backup_if_mismatch_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "one\nzero\ntwo\n")],
        "--- f\n+++ f\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n",
        &["--posix", "-p0"],
        &["f", "f.orig"],
    );
}

#[test]
fn posix_backup_if_mismatch_override_creates_backup_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "one\nzero\ntwo\n")],
        "--- f\n+++ f\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n",
        &["--posix", "--backup-if-mismatch", "-p0"],
        &["f", "f.orig"],
    );
}

#[test]
fn posixly_correct_disables_default_backup_if_mismatch_like_gnu_patch() {
    assert_matches_gnu_patch_with_env(
        &[("f", "one\nzero\ntwo\n")],
        "--- f\n+++ f\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n",
        &["-p0"],
        &[("POSIXLY_CORRECT", "1")],
        &["f", "f.orig"],
    );
}

#[test]
fn appends_repeated_rejects_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "one\n")],
        "--- f.orig\n+++ f\n@@ -2 +2 @@\n-two\n+three\n--- f.orig\n+++ f\n@@ -2 +2 @@\n-three\n+four\n",
        &["-p0"],
        &["f", "f.rej"],
    );
}

#[test]
fn writes_global_reject_file_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("a", "one\n"), ("b", "three\n")],
        "--- a\n+++ a\n@@ -1 +1 @@\n-one\n+two\n--- b\n+++ b\n@@ -1 +1 @@\n-three\n+four\n",
        &["-p0", "-r", "ab.rej"],
        &["a", "b", "ab.rej"],
    );
}

#[test]
fn attached_short_reject_file_option_routes_rejects_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "actual\n")],
        "--- f\n+++ f\n@@ -1 +1 @@\n-expected\n+changed\n",
        &["-rcustom.rej", "-p0"],
        &["f", "custom.rej"],
    );
}

#[test]
fn long_reject_file_option_with_equals_routes_rejects_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "actual\n")],
        "--- f\n+++ f\n@@ -1 +1 @@\n-expected\n+changed\n",
        &["--reject-file=custom.rej", "-p0"],
        &["f", "custom.rej"],
    );
}

#[test]
fn writes_forced_context_reject_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "actual\n")],
        "--- f.orig\n+++ f\n@@ -1 +1 @@\n-expected\n+changed\n",
        &["--reject-format=context", "f"],
        &["f", "f.rej"],
    );
}

#[test]
fn separated_long_reject_format_option_matches_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "actual\n")],
        "--- f.orig\n+++ f\n@@ -1 +1 @@\n-expected\n+changed\n",
        &["--reject-format", "context", "f"],
        &["f", "f.rej"],
    );
}

#[test]
fn writes_forced_unified_reject_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "actual\n")],
        "*** f.orig\n--- f\n***************\n*** 1 ****\n! expected\n--- 1 ----\n! changed\n",
        &["--reject-format=unified", "f"],
        &["f", "f.rej"],
    );
}

#[test]
fn preserves_reject_labels_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "actual\n")],
        "Index: f\n--- f.orig\told label\n+++ f\tnew label\n@@ -1 +1 @@ section\n-expected\n+changed\n",
        &["f"],
        &["f", "f.rej"],
    );
}

#[test]
fn writes_delete_only_context_reject_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("a", "1\n2\n")],
        "*** a\n--- b\n***************\n*** 1,2 ****\n  1\n- X\n--- 1 ----\n",
        &["a"],
        &["a", "a.rej"],
    );
}

#[test]
fn applies_shifted_hunk_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("a", "x\nx\n1\n2\n4\n")],
        "--- a\n+++ a\n@@ -1,3 +1,4 @@\n 1\n 2\n+3\n 4\n",
        &["-p0"],
        &["a"],
    );
}

#[test]
fn zero_fuzz_rejects_fuzzy_context_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("file", "1\nx\n3\n4\n5\n")],
        "--- file\n+++ file\n@@ -1,5 +1,5 @@\n 1\n 2\n-3\n+THREE\n 4\n 5\n",
        &["-p0", "--fuzz=0", "-f"],
        &["file", "file.rej"],
    );
}

#[test]
fn explicit_fuzz_allows_fuzzy_context_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("file", "1\nx\n3\n4\n5\n")],
        "--- file\n+++ file\n@@ -1,5 +1,5 @@\n 1\n 2\n-3\n+THREE\n 4\n 5\n",
        &["-p0", "--fuzz=2"],
        &["file"],
    );
}

#[test]
fn accepts_target_file_operand_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("a", "1\n2\n4\n")],
        "--- ignored\n+++ ignored\n@@ -1,3 +1,4 @@\n 1\n 2\n+3\n 4\n",
        &["a"],
        &["a", "ignored"],
    );
}

#[test]
fn reads_second_file_operand_as_patch_file_like_gnu_patch() {
    let patch = "--- f\n+++ f\n@@ -1 +1 @@\n-one\n+two\n";
    assert_matches_gnu_patch(
        &[("f", "one\n"), ("p.diff", patch)],
        "",
        &["f", "p.diff"],
        &["f", "p.diff"],
    );
}

#[test]
fn second_file_operand_overrides_input_option_like_gnu_patch() {
    let ignored = "--- f\n+++ f\n@@ -1 +1 @@\n-one\n+ignored\n";
    let patch = "--- f\n+++ f\n@@ -1 +1 @@\n-one\n+two\n";
    assert_matches_gnu_patch(
        &[("f", "one\n"), ("ignored.diff", ignored), ("p.diff", patch)],
        "",
        &["-i", "ignored.diff", "f", "p.diff"],
        &["f", "ignored.diff", "p.diff"],
    );
}

#[test]
fn applies_context_format_addition_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("a", "1\n2\n4\n5\n")],
        "*** a\n--- b\n***************\n*** 1,4 ****\n--- 1,5 ----\n  1\n  2\n+ 3\n  4\n  5\n",
        &[],
        &["a", "b"],
    );
}

#[test]
fn applies_context_format_removal_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("a", "1\n2\n3\n4\n5\n")],
        "*** a\n--- b\n***************\n*** 1,5 ****\n  1\n  2\n- 3\n  4\n  5\n--- 1,4 ----\n",
        &[],
        &["a", "b"],
    );
}

#[test]
fn aborts_mangled_context_hunk_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("a", "1\n2\n3\n4\n5\n6\n7\n")],
        "*** p\n--- p\n***************\n*** 405,409 ****\n--- 405,407 ----\n  a\n  b\n  c\n+ d\n  x\n  y\n  z\n",
        &["a"],
        &["a", "a.rej"],
    );
}

#[test]
fn applies_git_header_paths_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "one\ntwo\n")],
        "diff --git a/f b/f\nindex 5626abf..f719efd 100644\n--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n",
        &[],
        &["f", "b/f"],
    );
}

#[test]
fn rejects_plain_diff_header_without_file_headers_like_gnu_patch() {
    let root = tempdir("parity");
    let gnu_dir = root.join("gnu");
    let p2_dir = root.join("p2");
    fs::create_dir(&gnu_dir).unwrap();
    fs::write(gnu_dir.join("f"), "one\n").unwrap();
    copy_dir(&gnu_dir, &p2_dir);

    let patch = "diff a/f b/f\n@@ -1 +1 @@\n-one\n+two\n";
    let reference = reference_patch();
    let under_test = patch_under_test();
    let gnu = run_patch(&reference, &gnu_dir, patch, &["-p1"]);
    let ours = run_patch(&under_test, &p2_dir, patch, &["-p1"]);

    assert_eq!(
        gnu.status.code(),
        ours.status.code(),
        "gnu stderr: {}\np2 stderr: {}",
        String::from_utf8_lossy(&gnu.stderr),
        String::from_utf8_lossy(&ours.stderr)
    );
    assert_eq!(
        fs::read(gnu_dir.join("f")).unwrap(),
        fs::read(p2_dir.join("f")).unwrap()
    );
}

#[test]
fn applies_quoted_git_header_paths_with_spaces_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("file with space", "one\n")],
        "diff --git \"a/file with space\" \"b/file with space\"\n--- \"a/file with space\"\n+++ \"b/file with space\"\n@@ -1 +1 @@\n-one\n+two\n",
        &["-p1"],
        &["file with space"],
    );
}

#[test]
fn applies_git_copy_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "old\n")],
        "diff --git a/f b/g\ncopy from f\ncopy to g\nindex 3367afd..3e75765 100644\n--- a/f\n+++ b/g\n@@ -1 +1 @@\n-old\n+new\n",
        &["-p1"],
        &["f", "g"],
    );
}

#[test]
fn applies_git_rename_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "old\n")],
        "diff --git a/f b/h\nrename from f\nrename to h\nindex 3367afd..3e75765 100644\n--- a/f\n+++ b/h\n@@ -1 +1 @@\n-old\n+new\n",
        &["-p1"],
        &["f", "h"],
    );
}

#[test]
fn applies_criss_cross_git_renames_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "1\n"), ("g", "2\n")],
        "diff --git a/g b/f\nsimilarity index 100%\nrename from g\nrename to f\ndiff --git a/f b/g\nsimilarity index 100%\nrename from f\nrename to g\n",
        &["-p1"],
        &["f", "g"],
    );
}

#[test]
fn copies_original_source_after_prior_modification_like_gnu_patch() {
    let source = (1..=1000)
        .map(|number| format!("{number}\n"))
        .collect::<String>();
    let patch = "diff --git a/a b/a\nindex 1179824..912fcbd 100644\n--- a/a\n+++ b/a\n@@ -998,3 +998,4 @@\n 998\n 999\n 1000\n+a\ndiff --git a/a b/b\nsimilarity index 99%\ncopy from a\ncopy to b\nindex 1179824..1c5afa7 100644\n--- a/a\n+++ b/b\n@@ -998,3 +998,4 @@\n 998\n 999\n 1000\n+b\n";
    assert_matches_gnu_patch(&[("a", &source)], patch, &["-p1"], &["a", "b"]);
}

#[test]
fn creates_empty_file_from_git_diff_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[],
        "diff --git a/f b/f\nnew file mode 100644\nindex 0000000..e69de29\n",
        &[],
        &["f"],
    );
}

#[test]
fn rejects_git_binary_patch_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[],
        "diff --git a/zeroes b/zeroes\nnew file mode 100644\nindex 0000000..06d7405\nGIT binary patch\nliteral 1\nAcmZQz\n\nliteral 0\nHcmV?d00001\n",
        &["-p1"],
        &["zeroes"],
    );
}

#[cfg(unix)]
#[test]
fn handles_read_only_modes_like_gnu_patch() {
    let patch = "--- f.orig\n+++ f\n@@ -1 +1 @@\n-one\n+two\n--- f.orig\n+++ f\n@@ -1 +1 @@\n-two\n+three\n";
    for (name, args) in [
        ("read-only-fail", &["-p0", "--read-only=fail"][..]),
        (
            "read-only-fail-separated",
            &["-p0", "--read-only", "fail"][..],
        ),
        ("read-only-warn", &["-f", "-p0", "--read-only=warn"][..]),
        (
            "read-only-warn-separated",
            &["-f", "-p0", "--read-only", "warn"][..],
        ),
        ("read-only-ignore", &["-f", "-p0", "--read-only=ignore"][..]),
        (
            "read-only-ignore-separated",
            &["-f", "-p0", "--read-only", "ignore"][..],
        ),
    ] {
        let root = tempdir(name);
        let gnu_dir = root.join("gnu");
        let p2_dir = root.join("p2");
        fs::create_dir(&gnu_dir).unwrap();
        fs::create_dir(&p2_dir).unwrap();
        fs::write(gnu_dir.join("f"), "one\n").unwrap();
        fs::write(p2_dir.join("f"), "one\n").unwrap();
        fs::set_permissions(gnu_dir.join("f"), fs::Permissions::from_mode(0o444)).unwrap();
        fs::set_permissions(p2_dir.join("f"), fs::Permissions::from_mode(0o444)).unwrap();

        let reference = reference_patch();
        let under_test = patch_under_test();
        let gnu = run_patch(&reference, &gnu_dir, patch, args);
        let ours = run_patch(&under_test, &p2_dir, patch, args);
        assert_eq!(
            gnu.status.success(),
            ours.status.success(),
            "{name}\ngnu stderr: {}\np2 stderr: {}",
            String::from_utf8_lossy(&gnu.stderr),
            String::from_utf8_lossy(&ours.stderr)
        );
        for file in ["f", "f.rej"] {
            let gnu_path = gnu_dir.join(file);
            let p2_path = p2_dir.join(file);
            assert_eq!(
                gnu_path.exists(),
                p2_path.exists(),
                "{name}: {file} existence"
            );
            if gnu_path.exists() {
                assert_eq!(
                    fs::read(&gnu_path).unwrap(),
                    fs::read(&p2_path).unwrap(),
                    "{name}: {file} content"
                );
            }
        }
        fs::set_permissions(gnu_dir.join("f"), fs::Permissions::from_mode(0o644)).ok();
        fs::set_permissions(p2_dir.join("f"), fs::Permissions::from_mode(0o644)).ok();
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn replaces_hardlinked_targets_like_gnu_patch() {
    let patch =
        "--- f.orig\n+++ f\n@@ -2 +2 @@\n-one\n+two\n--- g.orig\n+++ g\n@@ -2 +2 @@\n-one\n+two\n";
    let root = tempdir("hardlinks");
    let gnu_dir = root.join("gnu");
    let p2_dir = root.join("p2");
    fs::create_dir(&gnu_dir).unwrap();
    fs::create_dir(&p2_dir).unwrap();
    fs::write(gnu_dir.join("f"), "one\n").unwrap();
    fs::hard_link(gnu_dir.join("f"), gnu_dir.join("g")).unwrap();
    fs::write(p2_dir.join("f"), "one\n").unwrap();
    fs::hard_link(p2_dir.join("f"), p2_dir.join("g")).unwrap();

    let reference = reference_patch();
    let under_test = patch_under_test();
    let gnu = run_patch(&reference, &gnu_dir, patch, &["-p0"]);
    let ours = run_patch(&under_test, &p2_dir, patch, &["-p0"]);
    assert_eq!(
        gnu.status.success(),
        ours.status.success(),
        "gnu stderr: {}\np2 stderr: {}",
        String::from_utf8_lossy(&gnu.stderr),
        String::from_utf8_lossy(&ours.stderr)
    );
    for file in ["f", "g", "f.orig", "g.orig"] {
        assert_eq!(
            fs::read(gnu_dir.join(file)).unwrap(),
            fs::read(p2_dir.join(file)).unwrap(),
            "{file} content"
        );
    }
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn refuses_regular_patch_against_symlink_like_gnu_patch() {
    let patch = "--- l\n+++ l\n@@ -1 +1 @@\n-one\n+two\n";
    let root = tempdir("symlink-refuse");
    let gnu_dir = root.join("gnu");
    let p2_dir = root.join("p2");
    fs::create_dir(&gnu_dir).unwrap();
    fs::create_dir(&p2_dir).unwrap();
    for dir in [&gnu_dir, &p2_dir] {
        fs::write(dir.join("f"), "one\n").unwrap();
        symlink("f", dir.join("l")).unwrap();
    }

    let reference = reference_patch();
    let under_test = patch_under_test();
    let gnu = run_patch(&reference, &gnu_dir, patch, &[]);
    let ours = run_patch(&under_test, &p2_dir, patch, &[]);
    assert_eq!(gnu.status.code(), ours.status.code());
    for file in ["f", "l.rej"] {
        assert_eq!(
            fs::read(gnu_dir.join(file)).unwrap(),
            fs::read(p2_dir.join(file)).unwrap(),
            "{file} content"
        );
    }
    assert_eq!(
        fs::read_link(gnu_dir.join("l")).unwrap(),
        fs::read_link(p2_dir.join("l")).unwrap()
    );

    let gnu = run_patch(&reference, &gnu_dir, patch, &["--follow-symlinks"]);
    let ours = run_patch(&under_test, &p2_dir, patch, &["--follow-symlinks"]);
    assert_eq!(gnu.status.success(), ours.status.success());
    assert_eq!(
        gnu_dir.join("l").is_symlink(),
        p2_dir.join("l").is_symlink(),
        "l symlink status"
    );
    assert_eq!(
        fs::read(gnu_dir.join("l")).unwrap(),
        fs::read(p2_dir.join("l")).unwrap()
    );

    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn applies_git_symlink_create_modify_delete_like_gnu_patch() {
    let create_patch = "diff --git a/symlink b/symlink\nnew file mode 120000\nindex 0000000..12a8d8a\n--- /dev/null\n+++ b/symlink\n@@ -0,0 +1 @@\n+target1\n\\ No newline at end of file\n";
    let modify_patch = "diff --git a/symlink b/symlink\nindex 12a8d8a..3b7781e 120000\n--- a/symlink\n+++ b/symlink\n@@ -1 +1 @@\n-target1\n\\ No newline at end of file\n+target2\n\\ No newline at end of file\n";
    let delete_patch = "diff --git a/symlink b/symlink\ndeleted file mode 120000\nindex 3b7781e..0000000\n--- a/symlink\n+++ /dev/null\n@@ -1 +0,0 @@\n-target2\n\\ No newline at end of file\n";
    let root = tempdir("git-symlink");
    let gnu_dir = root.join("gnu");
    let p2_dir = root.join("p2");
    fs::create_dir(&gnu_dir).unwrap();
    fs::create_dir(&p2_dir).unwrap();

    let reference = reference_patch();
    let under_test = patch_under_test();
    for patch in [create_patch, modify_patch, delete_patch] {
        let gnu = run_patch(&reference, &gnu_dir, patch, &["-p1"]);
        let ours = run_patch(&under_test, &p2_dir, patch, &["-p1"]);
        assert_eq!(
            gnu.status.success(),
            ours.status.success(),
            "gnu stderr: {}\np2 stderr: {}",
            String::from_utf8_lossy(&gnu.stderr),
            String::from_utf8_lossy(&ours.stderr)
        );
        assert_eq!(
            fs::symlink_metadata(gnu_dir.join("symlink")).is_ok(),
            fs::symlink_metadata(p2_dir.join("symlink")).is_ok(),
            "symlink existence after patch"
        );
        if gnu_dir.join("symlink").is_symlink() {
            assert_eq!(
                fs::read_link(gnu_dir.join("symlink")).unwrap(),
                fs::read_link(p2_dir.join("symlink")).unwrap()
            );
        }
    }

    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn rejects_symlink_parent_escape_like_gnu_patch() {
    let patch = "--- g/bad-rel/x\n+++ g/bad-rel/x\n@@ -0,0 +1 @@\n+relative\n--- g/bad-abs/x\n+++ g/bad-abs/x\n@@ -0,0 +1 @@\n+absolute\n";
    let root = tempdir("symlink-parent-escape");
    let gnu_dir = root.join("gnu");
    let p2_dir = root.join("p2");
    fs::create_dir(&gnu_dir).unwrap();
    fs::create_dir(&p2_dir).unwrap();
    for dir in [&gnu_dir, &p2_dir] {
        fs::create_dir(dir.join("g")).unwrap();
        symlink("../../z", dir.join("g/bad-rel")).unwrap();
        symlink("/z", dir.join("g/bad-abs")).unwrap();
    }

    let reference = reference_patch();
    let under_test = patch_under_test();
    let gnu = run_patch(&reference, &gnu_dir, patch, &["-p0"]);
    let ours = run_patch(&under_test, &p2_dir, patch, &["-p0"]);
    assert_eq!(gnu.status.code(), ours.status.code());
    for file in ["g/bad-rel/x", "g/bad-abs/x"] {
        assert_eq!(
            fs::symlink_metadata(gnu_dir.join(file)).is_ok(),
            fs::symlink_metadata(p2_dir.join(file)).is_ok(),
            "{file} existence"
        );
    }

    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn preserves_symlink_backup_entries_like_gnu_patch() {
    let create_patch = "diff --git a/symlink b/symlink\nnew file mode 120000\nindex 0000000..12a8d8a\n--- /dev/null\n+++ b/symlink\n@@ -0,0 +1 @@\n+target1\n\\ No newline at end of file\n";
    let modify_patch = "diff --git a/symlink b/symlink\nindex 12a8d8a..3b7781e 120000\n--- a/symlink\n+++ b/symlink\n@@ -1 +1 @@\n-target1\n\\ No newline at end of file\n+target2\n\\ No newline at end of file\n";
    let delete_patch = "diff --git a/symlink b/symlink\ndeleted file mode 120000\nindex 3b7781e..0000000\n--- a/symlink\n+++ /dev/null\n@@ -1 +0,0 @@\n-target2\n\\ No newline at end of file\n";
    let root = tempdir("symlink-backup");
    let gnu_dir = root.join("gnu");
    let p2_dir = root.join("p2");
    fs::create_dir(&gnu_dir).unwrap();
    fs::create_dir(&p2_dir).unwrap();

    let reference = reference_patch();
    let under_test = patch_under_test();
    for patch in [create_patch, modify_patch, delete_patch] {
        let gnu = run_patch(&reference, &gnu_dir, patch, &["-p1", "--backup"]);
        let ours = run_patch(&under_test, &p2_dir, patch, &["-p1", "--backup"]);
        assert_eq!(gnu.status.success(), ours.status.success());
        assert_eq!(
            fs::symlink_metadata(gnu_dir.join("symlink.orig"))
                .unwrap()
                .file_type()
                .is_symlink(),
            fs::symlink_metadata(p2_dir.join("symlink.orig"))
                .unwrap()
                .file_type()
                .is_symlink(),
            "symlink.orig kind after patch"
        );
        if gnu_dir.join("symlink.orig").is_symlink() {
            assert_eq!(
                fs::read_link(gnu_dir.join("symlink.orig")).unwrap(),
                fs::read_link(p2_dir.join("symlink.orig")).unwrap()
            );
        } else {
            assert_eq!(
                fs::read(gnu_dir.join("symlink.orig")).unwrap(),
                fs::read(p2_dir.join("symlink.orig")).unwrap()
            );
        }
    }

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn deletes_empty_file_from_git_diff_like_gnu_patch() {
    assert_matches_gnu_patch(
        &[("f", "")],
        "diff --git a/f b/f\ndeleted file mode 100644\nindex e69de29..0000000\n",
        &[],
        &["f"],
    );
}

#[cfg(unix)]
#[test]
fn creates_git_file_mode_like_gnu_patch() {
    let patch = "diff --git a/f b/f\nnew file mode 100710\nindex 0000000..3e75765\n--- /dev/null\n+++ b/f\n@@ -0,0 +1 @@\n+new\n";
    let root = tempdir("mode");
    let gnu_dir = root.join("gnu");
    let p2_dir = root.join("p2");
    fs::create_dir(&gnu_dir).unwrap();
    fs::create_dir(&p2_dir).unwrap();

    let p2 = patch_under_test();
    let reference = reference_patch();
    let gnu = run_patch(&reference, &gnu_dir, patch, &["-p1"]);
    let ours = run_patch(&p2, &p2_dir, patch, &["-p1"]);
    assert_eq!(gnu.status.success(), ours.status.success());
    assert_eq!(
        fs::metadata(gnu_dir.join("f"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        fs::metadata(p2_dir.join("f")).unwrap().permissions().mode() & 0o777
    );

    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn applies_git_mode_change_like_gnu_patch() {
    let patch = "diff --git a/f b/f\nold mode 100644\nnew mode 100600\nindex 3367afd..3e75765\n--- a/f\n+++ b/f\n@@ -1 +1 @@\n-old\n+new\n";
    let root = tempdir("mode-change");
    let gnu_dir = root.join("gnu");
    let p2_dir = root.join("p2");
    fs::create_dir(&gnu_dir).unwrap();
    fs::create_dir(&p2_dir).unwrap();
    fs::write(gnu_dir.join("f"), "old\n").unwrap();
    fs::write(p2_dir.join("f"), "old\n").unwrap();
    fs::set_permissions(gnu_dir.join("f"), fs::Permissions::from_mode(0o644)).unwrap();
    fs::set_permissions(p2_dir.join("f"), fs::Permissions::from_mode(0o644)).unwrap();

    let p2 = patch_under_test();
    let reference = reference_patch();
    let gnu = run_patch(&reference, &gnu_dir, patch, &["-p1"]);
    let ours = run_patch(&p2, &p2_dir, patch, &["-p1"]);
    assert_eq!(gnu.status.success(), ours.status.success());
    assert_eq!(
        fs::metadata(gnu_dir.join("f"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        fs::metadata(p2_dir.join("f")).unwrap().permissions().mode() & 0o777
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sets_utc_timestamp_from_patch_header_like_gnu_patch() {
    let patch =
        "--- a/f\t2009-03-13 00:00:00\n+++ b/f\t2009-03-14 00:00:00\n@@ -1 +1 @@\n-one\n+two\n";
    let root = tempdir("set-utc");
    let gnu_dir = root.join("gnu");
    let p2_dir = root.join("p2");
    fs::create_dir(&gnu_dir).unwrap();
    fs::create_dir(&p2_dir).unwrap();
    fs::write(gnu_dir.join("f"), "one\n").unwrap();
    fs::write(p2_dir.join("f"), "one\n").unwrap();
    let timestamp1 = FileTime::from_unix_time(1_236_902_400, 0);
    set_file_mtime(gnu_dir.join("f"), timestamp1).unwrap();
    set_file_mtime(p2_dir.join("f"), timestamp1).unwrap();

    let p2 = patch_under_test();
    let reference = reference_patch();
    let gnu = run_patch(
        &reference,
        &gnu_dir,
        patch,
        &["-p1", "--backup", "--set-utc"],
    );
    let ours = run_patch(&p2, &p2_dir, patch, &["-p1", "--backup", "--set-utc"]);
    assert_eq!(
        gnu.status.success(),
        ours.status.success(),
        "gnu stderr: {}\np2 stderr: {}",
        String::from_utf8_lossy(&gnu.stderr),
        String::from_utf8_lossy(&ours.stderr)
    );
    assert_eq!(
        fs::read(gnu_dir.join("f")).unwrap(),
        fs::read(p2_dir.join("f")).unwrap()
    );
    assert_eq!(
        FileTime::from_last_modification_time(&fs::metadata(gnu_dir.join("f")).unwrap()).seconds(),
        FileTime::from_last_modification_time(&fs::metadata(p2_dir.join("f")).unwrap()).seconds()
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sets_fractional_timestamp_with_timezone_like_gnu_patch() {
    let patch = "--- a/f\t2009-03-13 00:00:00.000000000 +0000\n+++ b/f\t2009-03-14 01:00:00.123456789 +0100\n@@ -1 +1 @@\n-one\n+two\n";
    let root = tempdir("set-utc-fractional-zone");
    let gnu_dir = root.join("gnu");
    let p2_dir = root.join("p2");
    fs::create_dir(&gnu_dir).unwrap();
    fs::create_dir(&p2_dir).unwrap();
    fs::write(gnu_dir.join("f"), "one\n").unwrap();
    fs::write(p2_dir.join("f"), "one\n").unwrap();
    let timestamp1 = FileTime::from_unix_time(1_236_902_400, 0);
    set_file_mtime(gnu_dir.join("f"), timestamp1).unwrap();
    set_file_mtime(p2_dir.join("f"), timestamp1).unwrap();

    let p2 = patch_under_test();
    let reference = reference_patch();
    let gnu = run_patch(&reference, &gnu_dir, patch, &["-p1", "--set-utc"]);
    let ours = run_patch(&p2, &p2_dir, patch, &["-p1", "--set-utc"]);
    assert_eq!(
        gnu.status.success(),
        ours.status.success(),
        "gnu stderr: {}\np2 stderr: {}",
        String::from_utf8_lossy(&gnu.stderr),
        String::from_utf8_lossy(&ours.stderr)
    );
    let gnu_time = FileTime::from_last_modification_time(&fs::metadata(gnu_dir.join("f")).unwrap());
    let p2_time = FileTime::from_last_modification_time(&fs::metadata(p2_dir.join("f")).unwrap());
    assert_eq!(gnu_time.seconds(), p2_time.seconds());
    assert_eq!(gnu_time.nanoseconds(), p2_time.nanoseconds());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sets_timestamp_with_named_zero_timezone_like_gnu_patch() {
    for zone in ["UTC", "GMT", "UT", "Z", "utc", "gmt"] {
        let patch = format!(
            "--- a/f\t2009-03-13 00:00:00 {zone}\n+++ b/f\t2009-03-14 00:00:00 {zone}\n@@ -1 +1 @@\n-one\n+two\n"
        );
        let root = tempdir("set-utc-named-zone");
        let gnu_dir = root.join("gnu");
        let p2_dir = root.join("p2");
        fs::create_dir(&gnu_dir).unwrap();
        fs::create_dir(&p2_dir).unwrap();
        fs::write(gnu_dir.join("f"), "one\n").unwrap();
        fs::write(p2_dir.join("f"), "one\n").unwrap();
        let timestamp1 = FileTime::from_unix_time(1_236_902_400, 0);
        set_file_mtime(gnu_dir.join("f"), timestamp1).unwrap();
        set_file_mtime(p2_dir.join("f"), timestamp1).unwrap();

        let p2 = patch_under_test();
        let reference = reference_patch();
        let gnu = run_patch(&reference, &gnu_dir, &patch, &["-p1", "--set-utc"]);
        let ours = run_patch(&p2, &p2_dir, &patch, &["-p1", "--set-utc"]);
        assert_eq!(
            gnu.status.success(),
            ours.status.success(),
            "zone {zone}: gnu stderr: {}\np2 stderr: {}",
            String::from_utf8_lossy(&gnu.stderr),
            String::from_utf8_lossy(&ours.stderr)
        );
        let gnu_time =
            FileTime::from_last_modification_time(&fs::metadata(gnu_dir.join("f")).unwrap());
        let p2_time =
            FileTime::from_last_modification_time(&fs::metadata(p2_dir.join("f")).unwrap());
        assert_eq!(gnu_time.seconds(), p2_time.seconds(), "zone {zone}");
        assert_eq!(gnu_time.seconds(), 1_236_988_800, "zone {zone}");

        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn short_set_utc_option_sets_timestamp_like_gnu_patch() {
    let patch =
        "--- a/f\t2009-03-13 00:00:00\n+++ b/f\t2009-03-14 00:00:00\n@@ -1 +1 @@\n-one\n+two\n";
    let root = tempdir("set-utc-short");
    let gnu_dir = root.join("gnu");
    let p2_dir = root.join("p2");
    fs::create_dir(&gnu_dir).unwrap();
    fs::create_dir(&p2_dir).unwrap();
    fs::write(gnu_dir.join("f"), "one\n").unwrap();
    fs::write(p2_dir.join("f"), "one\n").unwrap();
    let timestamp1 = FileTime::from_unix_time(1_236_902_400, 0);
    set_file_mtime(gnu_dir.join("f"), timestamp1).unwrap();
    set_file_mtime(p2_dir.join("f"), timestamp1).unwrap();

    let p2 = patch_under_test();
    let reference = reference_patch();
    let gnu = run_patch(&reference, &gnu_dir, patch, &["-p1", "-Z"]);
    let ours = run_patch(&p2, &p2_dir, patch, &["-p1", "-Z"]);
    assert_eq!(
        gnu.status.success(),
        ours.status.success(),
        "gnu stderr: {}\np2 stderr: {}",
        String::from_utf8_lossy(&gnu.stderr),
        String::from_utf8_lossy(&ours.stderr)
    );
    let gnu_time = FileTime::from_last_modification_time(&fs::metadata(gnu_dir.join("f")).unwrap());
    let p2_time = FileTime::from_last_modification_time(&fs::metadata(p2_dir.join("f")).unwrap());
    assert_eq!(gnu_time.seconds(), p2_time.seconds());

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn set_time_uses_local_timezone_like_gnu_patch() {
    let patch =
        "--- a/f\t2009-03-13 00:00:00\n+++ b/f\t2009-03-14 00:00:00\n@@ -1 +1 @@\n-one\n+two\n";
    let root = tempdir("set-time-local");
    let gnu_dir = root.join("gnu");
    let p2_dir = root.join("p2");
    fs::create_dir(&gnu_dir).unwrap();
    fs::create_dir(&p2_dir).unwrap();
    fs::write(gnu_dir.join("f"), "one\n").unwrap();
    fs::write(p2_dir.join("f"), "one\n").unwrap();
    let source_time = FileTime::from_unix_time(1_236_920_400, 0);
    set_file_mtime(gnu_dir.join("f"), source_time).unwrap();
    set_file_mtime(p2_dir.join("f"), source_time).unwrap();

    let p2 = patch_under_test();
    let reference = reference_patch();
    let gnu = run_patch_with_env(
        &reference,
        &gnu_dir,
        patch,
        &["-p1", "--set-time"],
        &[("TZ", "EST5")],
    );
    let ours = run_patch_with_env(
        &p2,
        &p2_dir,
        patch,
        &["-p1", "--set-time"],
        &[("TZ", "EST5")],
    );
    assert_eq!(
        gnu.status.success(),
        ours.status.success(),
        "gnu stderr: {}\np2 stderr: {}",
        String::from_utf8_lossy(&gnu.stderr),
        String::from_utf8_lossy(&ours.stderr)
    );
    let gnu_time = FileTime::from_last_modification_time(&fs::metadata(gnu_dir.join("f")).unwrap());
    let p2_time = FileTime::from_last_modification_time(&fs::metadata(p2_dir.join("f")).unwrap());
    assert_eq!(gnu_time.seconds(), p2_time.seconds());
    assert_eq!(gnu_time.seconds(), 1_237_006_800);

    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn recreates_reject_file_with_current_umask_like_gnu_patch() {
    let patch = "--- a/f\t2009-03-13 00:00:00\n+++ b/f\t2009-03-14 00:00:00\n@@ -1 +1 @@\n-one\n+two\n@@ -2 +2 @@\n-three\n+four\n";
    let root = tempdir("reject-umask");
    let gnu_dir = root.join("gnu");
    let p2_dir = root.join("p2");
    fs::create_dir(&gnu_dir).unwrap();
    fs::create_dir(&p2_dir).unwrap();
    for dir in [&gnu_dir, &p2_dir] {
        fs::write(dir.join("f"), "one\nfive\n").unwrap();
        fs::write(dir.join("f.rej"), "old\n").unwrap();
        fs::set_permissions(dir.join("f.rej"), fs::Permissions::from_mode(0o644)).unwrap();
    }

    let p2 = patch_under_test();
    let reference = reference_patch();
    let gnu = run_patch_with_umask(
        &reference,
        &gnu_dir,
        patch,
        &["-p1", "--backup", "--set-utc"],
        "027",
    );
    let ours = run_patch_with_umask(
        &p2,
        &p2_dir,
        patch,
        &["-p1", "--backup", "--set-utc"],
        "027",
    );
    assert_eq!(
        gnu.status.success(),
        ours.status.success(),
        "gnu stderr: {}\np2 stderr: {}",
        String::from_utf8_lossy(&gnu.stderr),
        String::from_utf8_lossy(&ours.stderr)
    );
    assert_eq!(
        fs::metadata(gnu_dir.join("f.rej"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        fs::metadata(p2_dir.join("f.rej"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    );

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn header_without_hunks_is_skipped_like_gnu_patch() {
    // Alone, it is garbage; next to a real file patch, it is ignored.
    assert_matches_gnu_patch(&[("f", "a\n")], "--- a/f\n+++ b/f\n", &["-p1"], &["f"]);
    assert_matches_gnu_patch(
        &[("f", "a\n"), ("g", "b\n")],
        "--- a/f\n+++ b/f\n--- a/g\n+++ b/g\n@@ -1 +1 @@\n-b\n+B\n",
        &["-p1"],
        &["f", "g"],
    );
}

#[test]
fn no_newline_markers_apply_like_gnu_patch() {
    let marker = "\\ No newline at end of file\n";
    assert_matches_gnu_patch(
        &[("f", "a")],
        &format!("--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n{marker}+b\n"),
        &["-p1"],
        &["f"],
    );
    assert_matches_gnu_patch(
        &[("f", "a\n")],
        &format!("--- a/f\n+++ b/f\n@@ -1 +1 @@\n-a\n+b\n{marker}"),
        &["-p1"],
        &["f"],
    );
    assert_matches_gnu_patch(
        &[("f", "x\na")],
        &format!("--- a/f\n+++ b/f\n@@ -1,2 +1,2 @@\n x\n-a\n{marker}+b\n"),
        &["-p1"],
        &["f"],
    );
}

/// Compares the `Hunk #` lines of `--verbose` output, which show where each
/// hunk applied and with what offset and fuzz.
fn assert_verbose_output_matches_gnu_patch(name: &str, file: &str, patch: &str) {
    let root = tempdir(name);
    let mut outputs = Vec::new();
    for (side, binary) in [("gnu", reference_patch()), ("p2", patch_under_test())] {
        let dir = root.join(side);
        fs::create_dir(&dir).unwrap();
        fs::write(dir.join("f"), file).unwrap();
        let output = run_patch(&binary, &dir, patch, &["-p1", "--verbose"]);
        let hunks: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|line| line.starts_with("Hunk #"))
            .map(str::to_string)
            .collect();
        outputs.push(hunks);
    }
    assert_eq!(outputs[0], outputs[1]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn hunks_after_added_lines_report_no_offset_like_gnu_patch() {
    let file: String = (1..=20).map(|n| format!("{n}\n")).collect();
    assert_verbose_output_matches_gnu_patch(
        "report-after-added-lines",
        &file,
        "--- a/f\n+++ b/f\n@@ -2,3 +2,4 @@\n 2\n+2b\n 3\n 4\n@@ -15,3 +16,3 @@\n 15\n-16\n+16b\n 17\n",
    );
}

#[test]
fn short_trailing_context_at_end_of_file_reports_no_fuzz_like_gnu_patch() {
    assert_verbose_output_matches_gnu_patch(
        "report-eof-context",
        "1\n2\n3\n4\n5\n",
        "--- a/f\n+++ b/f\n@@ -1,5 +1,5 @@\n 1\n 2\n 3\n-4\n+four\n 5\n",
    );
}
