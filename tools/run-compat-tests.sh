#!/usr/bin/env bash
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
quilt_bin=${1:-${QUILT_BIN:-/usr/bin/quilt}}

run_test() {
	local test=$1
	shift
	Q_TEST_BIN="$quilt_bin" cargo test --test "$test" -- "$@"
}

cd "$repo"

# Entirely q-specific suites are intentionally omitted:
# - completion
# - git_absorb
# - metadata
run_test guards
run_test header
run_test snapshot_setup

# q color handling, malformed-env reporting, and accepting --version after
# global options (quilt only takes it alone) are intentional q behavior.
run_test env \
	--skip combined_diff_color_always_honors_color_environment \
	--skip diff_color_always_honors_color_environment \
	--skip malformed_command_args_environment_is_reported \
	--skip version_works_after_global_options

# q graph output is intentionally richer than the distro quilt binary.
run_test graph \
	--skip graph_accepts_upstream_option_forms \
	--skip graph_lines_filters_non_overlapping_file_changes \
	--skip graph_outputs_dot_for_shared_files

# q mail has stricter validation.
run_test mail \
	--skip mail_rejects_duplicate_patch_subjects \
	--skip mail_reply_to_without_subject_requires_intro_subject

# q hides q metadata from grep.
run_test query \
	--skip grep_skips_configured_metadata_directories

# These are q additions or differences from the distro quilt package.
# Savannah upstream's own test suite expects the missing/empty patch behavior.
run_test stack \
	--skip add_new_records_existing_file_as_created \
	--skip fold_uses_fast_path_for_patch_environment_options \
	--skip patch_basename_match_must_be_unambiguous \
	--skip patch_environment_options_are_honored \
	--skip pop_accepts_unique_patch_basename \
	--skip push_and_pop_empty_patch_file \
	--skip push_applies_missing_patch_file_as_empty_patch
