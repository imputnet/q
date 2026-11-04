#!/usr/bin/env bash
set -euo pipefail

usage() {
	printf 'Usage: %s [fixture-dir]\n' "$0" >&2
	printf 'Environment:\n' >&2
	printf '  Q_BIN=/path/to/q (default: target/release/q)\n' >&2
	printf '  QUILT_BIN=/path/to/quilt (default: /usr/bin/quilt if present, else quilt)\n' >&2
	printf '  BENCHMARK_RESULTS=path (default: target/benchmark-results.md)\n' >&2
	printf '  BENCHMARK_WARMUP=N (default: 3)\n' >&2
	printf '  BENCHMARK_MIN_RUNS=N (default: 10)\n' >&2
	printf '  RUSTFLAGS=... (default: -C target-cpu=native -C debuginfo=1)\n' >&2
}

if [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ]; then
	usage
	exit 0
fi

if [ "$#" -gt 1 ]; then
	usage
	exit 2
fi

if ! command -v hyperfine >/dev/null 2>&1; then
	printf 'error: hyperfine is required\n' >&2
	exit 1
fi

root=$(cd "$(dirname "$0")/.." && pwd)
fixture=${1:-"$root/target/benchmark-fixture"}
results=${BENCHMARK_RESULTS:-"$root/target/benchmark-results.md"}
q_bin=${Q_BIN:-"$root/target/release/q"}
if [ -z "${QUILT_BIN:-}" ]; then
	if [ -x /usr/bin/quilt ]; then
		quilt_bin=/usr/bin/quilt
	else
		quilt_bin=quilt
	fi
else
	quilt_bin=$QUILT_BIN
fi
warmup=${BENCHMARK_WARMUP:-3}
min_runs=${BENCHMARK_MIN_RUNS:-10}
run_dir=$root/target/benchmark-run
tmp_results=$root/target/benchmark-results
quilt_unset='unset QUILT_COLORS QUILT_DIFF_ARGS QUILT_DIFF_OPTS QUILT_PATCHES QUILT_PATCHES_ARGS QUILT_PATCH_OPTS QUILT_PUSH_ARGS QUILT_REFRESH_ARGS QUILT_SERIES QUILT_SERIES_ARGS;'

cd "$root"
RUSTFLAGS=${RUSTFLAGS:--C target-cpu=native -C debuginfo=1} cargo build --release
QUILT_BIN="$quilt_bin" "$root/tools/benchmark-fixture.sh" "$fixture"
fixture=$(cd "$fixture" && pwd)

rm -rf "$tmp_results"
mkdir -p "$tmp_results"
mkdir -p "$(dirname "$results")"
{
	printf '# q benchmark\n\n'
	printf 'Fixture: `%s`\n\n' "$fixture"
	printf 'q: `%s`\n\n' "$q_bin"
	printf 'quilt: `%s`\n\n' "$quilt_bin"
	printf 'Hyperfine: `%s`\n\n' "$(hyperfine --version)"
} >"$results"

bench_pair() {
	name=$1
	template=$2
	q_command=$3
	quilt_command=$4
	out=$tmp_results/$name.md
	prepare="rm -rf '$run_dir' && cp -a '$template' '$run_dir'"
	hyperfine \
		--warmup "$warmup" \
		--min-runs "$min_runs" \
		--prepare "$prepare" \
		--command-name "q $name" "$q_command" \
		--command-name "quilt $name" "$quilt_command" \
		--export-markdown "$out"
	{
		printf '## %s\n\n' "$name"
		cat "$out"
		printf '\n'
	} >>"$results"
}

q_env="$quilt_unset QUILTRC=/dev/null"
quilt_env="$quilt_unset QUILTRC=/dev/null"
q_refresh_env="$quilt_unset QUILTRC=/dev/null QUILT_REFRESH_ARGS=--no-timestamps"
quilt_refresh_env="$quilt_unset QUILTRC=/dev/null QUILT_REFRESH_ARGS=--no-timestamps"

bench_pair \
	"push -a" \
	"$fixture/popped-template" \
	"cd '$run_dir' && $q_env '$q_bin' push -a" \
	"cd '$run_dir' && $quilt_env '$quilt_bin' push -a"

bench_pair \
	"pop -a" \
	"$fixture/applied-template" \
	"cd '$run_dir' && $q_env '$q_bin' pop -a" \
	"cd '$run_dir' && $quilt_env '$quilt_bin' pop -a"

bench_pair \
	"refresh --no-timestamps" \
	"$fixture/applied-template" \
	"cd '$run_dir' && $q_env '$q_bin' refresh --no-timestamps" \
	"cd '$run_dir' && $quilt_env '$quilt_bin' refresh --no-timestamps"

bench_pair \
	"push -a --refresh" \
	"$fixture/popped-template" \
	"cd '$run_dir' && $q_refresh_env '$q_bin' push -a --refresh" \
	"cd '$run_dir' && $quilt_refresh_env '$quilt_bin' push -a --refresh"

bench_pair \
	"pop -a --refresh" \
	"$fixture/applied-template" \
	"cd '$run_dir' && $q_refresh_env '$q_bin' pop -a --refresh" \
	"cd '$run_dir' && $quilt_refresh_env '$quilt_bin' pop -a --refresh"

bench_pair \
	"fold" \
	"$fixture/applied-template" \
	"cd '$run_dir' && $q_env '$q_bin' fold < '$fixture/fold.diff'" \
	"cd '$run_dir' && $quilt_env '$quilt_bin' fold < '$fixture/fold.diff'"

printf 'Wrote benchmark results to %s\n' "$results"
