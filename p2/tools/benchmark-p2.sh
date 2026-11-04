#!/usr/bin/env bash
set -euo pipefail

usage() {
	printf 'Usage: %s [output-dir]\n' "$0" >&2
	printf 'Default output-dir is /dev/shm/p2-benchmark-p2-<uid> when available, otherwise target/benchmark-p2.\n' >&2
	printf 'Environment:\n' >&2
	printf '  PATCH_BIN=/path/to/patch  GNU-compatible patch binary (default: patch)\n' >&2
	printf '  P2_BIN=/path/to/p2        p2 binary (default: target/release/p2)\n' >&2
	printf '  REPEAT=N                  timing runs per case/tool (default: 5)\n' >&2
}

default_output_dir() {
	if [ -d /dev/shm ] && [ -w /dev/shm ] && [ -x /dev/shm ]; then
		printf '/dev/shm/p2-benchmark-p2-%s\n' "$(id -u)"
	else
		printf 'target/benchmark-p2\n'
	fi
}

if [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ]; then
	usage
	exit 0
fi
if [ "$#" -gt 1 ]; then
	usage
	exit 2
fi

out=${1:-$(default_output_dir)}
patch_bin=${PATCH_BIN:-patch}
p2_bin=${P2_BIN:-target/release/p2}
repeat=${REPEAT:-5}

case "$repeat" in
	''|*[!0-9]*)
		printf 'REPEAT must be a positive integer\n' >&2
		exit 2
		;;
esac
if [ "$repeat" -lt 1 ]; then
	printf 'REPEAT must be a positive integer\n' >&2
	exit 2
fi

if ! command -v cargo >/dev/null 2>&1 && [ -x "$HOME/.cargo/bin/cargo" ]; then
	PATH="$HOME/.cargo/bin:$PATH"
fi
if ! command -v hyperfine >/dev/null 2>&1; then
	printf 'hyperfine is required. Install it and rerun this benchmark.\n' >&2
	exit 127
fi

if [ ! -x "$p2_bin" ]; then
	cargo build --release >/dev/null
fi

rm -rf "$out"
mkdir -p "$out"/{templates,patches,runs}
out=$(cd "$out" && pwd)
if [ -x "$p2_bin" ]; then
	p2_bin=$(cd "$(dirname "$p2_bin")" && pwd)/$(basename "$p2_bin")
fi

write_base_files() {
	local dir=$1
	local files=${2:-200}
	local lines=${3:-40}

	mkdir -p "$dir"
	for i in $(seq -w 0 $((files - 1))); do
		{
			printf 'file %s\n' "$i"
			for n in $(seq -w 1 "$lines"); do
				printf 'base line %s\n' "$n"
			done
		} >"$dir/file-$i.txt"
	done
}

make_many_files_patch() {
	local patch=$1
	local files=${2:-200}
	: >"$patch"
	for i in $(seq -w 0 $((files - 1))); do
		{
			printf -- '--- a/file-%s.txt\n' "$i"
			printf -- '+++ b/file-%s.txt\n' "$i"
			printf '@@ -1,3 +1,4 @@\n'
			printf ' file %s\n' "$i"
			printf ' base line 01\n'
			printf -- '+patched file %s\n' "$i"
			printf ' base line 02\n'
		} >>"$patch"
	done
}

make_big_file_template_and_patch() {
	local dir=$1
	local patch=$2
	local line_count=${3:-10000}
	local hunks=${4:-500}
	local step=$((line_count / hunks))

	mkdir -p "$dir"
	for n in $(seq -w 1 "$line_count"); do
		printf 'line %s\n' "$n"
	done >"$dir/big.txt"

	: >"$patch"
	printf -- '--- a/big.txt\n' >>"$patch"
	printf -- '+++ b/big.txt\n' >>"$patch"
	for h in $(seq 1 "$hunks"); do
		local line=$((h * step))
		local prev=$((line - 1))
		local next=$((line + 1))
		printf '@@ -%d,3 +%d,4 @@\n' "$prev" "$prev" >>"$patch"
		printf ' line %05d\n' "$prev" >>"$patch"
		printf ' line %05d\n' "$line" >>"$patch"
		printf -- '+patched hunk %03d\n' "$h" >>"$patch"
		printf ' line %05d\n' "$next" >>"$patch"
	done
}

make_create_delete_template_and_patch() {
	local dir=$1
	local patch=$2
	local count=${3:-150}

	mkdir -p "$dir"
	for i in $(seq -w 0 $((count - 1))); do
		printf 'delete me %s\n' "$i" >"$dir/delete-$i.txt"
	done

	: >"$patch"
	for i in $(seq -w 0 $((count - 1))); do
		{
			printf -- '--- /dev/null\n'
			printf -- '+++ b/create-%s.txt\n' "$i"
			printf '@@ -0,0 +1,2 @@\n'
			printf -- '+created %s\n' "$i"
			printf -- '+payload %s\n' "$i"
			printf -- '--- a/delete-%s.txt\n' "$i"
			printf -- '+++ /dev/null\n'
			printf '@@ -1 +0,0 @@\n'
			printf -- '-delete me %s\n' "$i"
		} >>"$patch"
	done
}

make_git_copy_rename_template_and_patch() {
	local dir=$1
	local patch=$2
	local count=${3:-100}

	mkdir -p "$dir"
	for i in $(seq -w 0 $((count - 1))); do
		printf 'source %s\npayload %s\n' "$i" "$i" >"$dir/source-$i.txt"
		printf 'move %s\npayload %s\n' "$i" "$i" >"$dir/rename-$i.txt"
	done

	: >"$patch"
	for i in $(seq -w 0 $((count - 1))); do
		{
			printf 'diff --git a/source-%s.txt b/copy-%s.txt\n' "$i" "$i"
			printf 'copy from source-%s.txt\n' "$i"
			printf 'copy to copy-%s.txt\n' "$i"
			printf -- '--- a/source-%s.txt\n' "$i"
			printf -- '+++ b/copy-%s.txt\n' "$i"
			printf '@@ -1,2 +1,3 @@\n'
			printf ' source %s\n' "$i"
			printf ' payload %s\n' "$i"
			printf -- '+copied %s\n' "$i"
			printf 'diff --git a/rename-%s.txt b/renamed-%s.txt\n' "$i" "$i"
			printf 'rename from rename-%s.txt\n' "$i"
			printf 'rename to renamed-%s.txt\n' "$i"
			printf -- '--- a/rename-%s.txt\n' "$i"
			printf -- '+++ b/renamed-%s.txt\n' "$i"
			printf '@@ -1,2 +1,3 @@\n'
			printf ' move %s\n' "$i"
			printf ' payload %s\n' "$i"
			printf -- '+renamed %s\n' "$i"
		} >>"$patch"
	done
}

make_shifted_context_template_and_patch() {
	local dir=$1
	local patch=$2
	local files=${3:-100}

	write_base_files "$dir" "$files" 80
	for i in $(seq -w 0 $((files - 1))); do
		local file tmp
		file="$dir/file-$i.txt"
		tmp="$dir/.file-$i.txt.tmp"
		{
			printf 'inserted drift\n'
			cat "$file"
		} >"$tmp"
		mv "$tmp" "$file"
	done

	: >"$patch"
	for i in $(seq -w 0 $((files - 1))); do
		{
			printf -- '--- a/file-%s.txt\n' "$i"
			printf -- '+++ b/file-%s.txt\n' "$i"
			printf '@@ -1,3 +1,4 @@\n'
			printf ' file %s\n' "$i"
			printf ' base line 01\n'
			printf -- '+shifted patch %s\n' "$i"
			printf ' base line 02\n'
		} >>"$patch"
	done
}

chromium_like_path() {
	local index=$1
	case $((index % 8)) in
		0) printf 'chrome/browser/ui/views/file-%04d.cc' "$index" ;;
		1) printf 'chrome/browser/resources/settings/page-%04d.ts' "$index" ;;
		2) printf 'components/helium_services/service-%04d.cc' "$index" ;;
		3) printf 'third_party/blink/renderer/core/core-%04d.cc' "$index" ;;
		4) printf 'content/browser/renderer_host/host-%04d.cc' "$index" ;;
		5) printf 'ui/views/controls/control-%04d.h' "$index" ;;
		6) printf 'extensions/browser/extension-%04d.cc' "$index" ;;
		*) printf 'chrome/app/resources/resource-%04d.grdp' "$index" ;;
	esac
}

write_chromium_like_file() {
	local root=$1
	local relative=$2
	local path="$root/$relative"

	mkdir -p "$(dirname "$path")"
	{
		printf 'synthetic source %s\n' "$relative"
		for n in $(seq -w 1 80); do
			printf 'stable context line %s\n' "$n"
		done
	} >"$path"
}

make_chromium_series_shape() {
	local dir=$1
	local patch_dir=$2
	local patch_count=${3:-300}

	mkdir -p "$dir" "$patch_dir"
	: >"$patch_dir/series"
	: >"$patch_dir/combined.patch"

	for p in $(seq 0 $((patch_count - 1))); do
		local patch
		patch=$(printf 'patch-%03d.patch' "$p")
		printf '%s\n' "$patch" >>"$patch_dir/series"
		: >"$patch_dir/$patch"

		local touch_count=$((1 + (p % 5)))
		if [ $((p % 75)) -eq 0 ]; then
			touch_count=80
		fi

		for j in $(seq 0 $((touch_count - 1))); do
			local file_index=$((p * 100 + j))
			local path
			path=$(chromium_like_path "$file_index")
			write_chromium_like_file "$dir" "$path"
			{
				printf -- '--- a/%s\n' "$path"
				printf -- '+++ b/%s\n' "$path"
				printf '@@ -19,5 +19,6 @@\n'
				printf ' stable context line 18\n'
				printf ' stable context line 19\n'
				printf ' stable context line 20\n'
				printf -- '+series patch %03d file %03d\n' "$p" "$j"
				printf ' stable context line 21\n'
				printf ' stable context line 22\n'
			} >>"$patch_dir/$patch"
		done
		cat "$patch_dir/$patch" >>"$patch_dir/combined.patch"
	done
}

make_dependent_series_shape() {
	local dir=$1
	local patch_dir=$2
	local patch_count=${3:-300}
	local line_count=${4:-1000}

	mkdir -p "$dir" "$patch_dir"
	: >"$patch_dir/series"
	: >"$patch_dir/combined.patch"

	for n in $(seq 1 "$line_count"); do
		if [ "$n" -eq 500 ]; then
			printf 'state 000\n'
		else
			printf 'stable line %04d\n' "$n"
		fi
	done >"$dir/shared.txt"

	for p in $(seq 0 $((patch_count - 1))); do
		local patch old new
		patch=$(printf 'patch-%03d.patch' "$p")
		old=$(printf 'state %03d' "$p")
		new=$(printf 'state %03d' "$((p + 1))")
		printf '%s\n' "$patch" >>"$patch_dir/series"
		{
			printf -- '--- a/shared.txt\n'
			printf -- '+++ b/shared.txt\n'
			printf '@@ -498,5 +498,5 @@\n'
			printf ' stable line 0498\n'
			printf ' stable line 0499\n'
			printf -- '-%s\n' "$old"
			printf -- '+%s\n' "$new"
			printf ' stable line 0501\n'
			printf ' stable line 0502\n'
		} >"$patch_dir/$patch"
		cat "$patch_dir/$patch" >>"$patch_dir/combined.patch"
	done
}

generate_fixtures() {
	write_base_files "$out/templates/many-files" 200 40
	make_many_files_patch "$out/patches/many-files.patch" 200

	make_big_file_template_and_patch \
		"$out/templates/big-file-many-hunks" \
		"$out/patches/big-file-many-hunks.patch" \
		10000 \
		500

	make_create_delete_template_and_patch \
		"$out/templates/create-delete" \
		"$out/patches/create-delete.patch" \
		150

	make_git_copy_rename_template_and_patch \
		"$out/templates/git-copy-rename" \
		"$out/patches/git-copy-rename.patch" \
		100

	make_shifted_context_template_and_patch \
		"$out/templates/shifted-context" \
		"$out/patches/shifted-context.patch" \
		100

	make_chromium_series_shape \
		"$out/templates/chromium-series-shape" \
		"$out/patches/chromium-series-shape" \
		300

	make_dependent_series_shape \
		"$out/templates/dependent-series-shape" \
		"$out/patches/dependent-series-shape" \
		300 \
		1000
}

quote() {
	printf '%q' "$1"
}

bench_case() {
	local name=$1
	local template=$2
	local patch=$3
	local args=$4

	local patch_dir="$out/runs/$name-patch"
	local p2_dir="$out/runs/$name-p2"
	local prepare patch_cmd p2_cmd
	prepare="rm -rf $(quote "$patch_dir") $(quote "$p2_dir") && cp -a $(quote "$template") $(quote "$patch_dir") && cp -a $(quote "$template") $(quote "$p2_dir")"
	patch_cmd="$(quote "$patch_bin") $args -i $(quote "$patch") -d $(quote "$patch_dir") >/dev/null"
	p2_cmd="$(quote "$p2_bin") $args -i $(quote "$patch") -d $(quote "$p2_dir") >/dev/null"
	printf '\n== %s ==\n' "$name"
	hyperfine \
		--warmup 1 \
		--runs "$repeat" \
		--prepare "$prepare" \
		--command-name "patch" \
		--command-name "p2" \
		"$patch_cmd" \
		"$p2_cmd"
}

bench_series_case() {
	local name=$1
	local template=$2
	local patch_root=$3
	local series=$4
	local args=$5

	local patch_dir="$out/runs/$name-patch"
	local p2_dir="$out/runs/$name-p2"
	local patch_concat_dir="$out/runs/$name-patch-concat"
	local p2_concat_dir="$out/runs/$name-p2-concat"
	local combined="$patch_root/combined.patch"
	local prepare patch_cmd p2_cmd patch_concat_cmd p2_concat_cmd
	prepare="rm -rf $(quote "$patch_dir") $(quote "$p2_dir") $(quote "$patch_concat_dir") $(quote "$p2_concat_dir") && cp -a $(quote "$template") $(quote "$patch_dir") && cp -a $(quote "$template") $(quote "$p2_dir") && cp -a $(quote "$template") $(quote "$patch_concat_dir") && cp -a $(quote "$template") $(quote "$p2_concat_dir")"
	patch_cmd="while IFS= read -r patch_file; do $(quote "$patch_bin") $args -i $(quote "$patch_root")/\"\$patch_file\" -d $(quote "$patch_dir") >/dev/null; done < $(quote "$series")"
	p2_cmd="while IFS= read -r patch_file; do $(quote "$p2_bin") $args -i $(quote "$patch_root")/\"\$patch_file\" -d $(quote "$p2_dir") >/dev/null; done < $(quote "$series")"
	patch_concat_cmd="$(quote "$patch_bin") $args -i $(quote "$combined") -d $(quote "$patch_concat_dir") >/dev/null"
	p2_concat_cmd="$(quote "$p2_bin") $args -i $(quote "$combined") -d $(quote "$p2_concat_dir") >/dev/null"
	printf '\n== %s ==\n' "$name"
	hyperfine \
		--warmup 1 \
		--runs "$repeat" \
		--prepare "$prepare" \
		--command-name "patch" \
		--command-name "p2" \
		--command-name "patch concat" \
		--command-name "p2 concat" \
		"$patch_cmd" \
		"$p2_cmd" \
		"$patch_concat_cmd" \
		"$p2_concat_cmd"
}

printf 'Generating benchmark fixture in %s\n' "$out"
generate_fixtures

printf 'patch binary: %s\n' "$patch_bin"
printf 'p2 binary:    %s\n' "$p2_bin"
printf 'runs:         %s\n' "$repeat"

common_args="-p1 --no-backup-if-mismatch"

bench_case many-files "$out/templates/many-files" "$out/patches/many-files.patch" "$common_args"
bench_case big-file-many-hunks "$out/templates/big-file-many-hunks" "$out/patches/big-file-many-hunks.patch" "$common_args"
bench_case create-delete "$out/templates/create-delete" "$out/patches/create-delete.patch" "$common_args"
bench_case git-copy-rename "$out/templates/git-copy-rename" "$out/patches/git-copy-rename.patch" "$common_args"
bench_case shifted-context "$out/templates/shifted-context" "$out/patches/shifted-context.patch" "$common_args"
bench_series_case chromium-series-shape \
	"$out/templates/chromium-series-shape" \
	"$out/patches/chromium-series-shape" \
	"$out/patches/chromium-series-shape/series" \
	"$common_args"
bench_series_case dependent-series-shape \
	"$out/templates/dependent-series-shape" \
	"$out/patches/dependent-series-shape" \
	"$out/patches/dependent-series-shape/series" \
	"$common_args"
