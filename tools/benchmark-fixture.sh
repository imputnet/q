#!/usr/bin/env bash
set -euo pipefail

usage() {
	printf 'Usage: %s [output-dir]\n' "$0" >&2
	printf 'Environment: QUILT_BIN=/path/to/quilt (default: quilt)\n' >&2
}

if [ "${1:-}" = "-h" ] || [ "${1:-}" = "--help" ]; then
	usage
	exit 0
fi

if [ "$#" -gt 1 ]; then
	usage
	exit 2
fi

out=${1:-target/benchmark-fixture}
quilt_bin=${QUILT_BIN:-quilt}

run_quilt() {
	(
		for name in ${!QUILT@}; do
			unset "$name"
		done
		QUILTRC=/dev/null "$quilt_bin" "$@"
	)
}

rm -rf "$out"
mkdir -p "$out/source"

out_abs=$(cd "$out" && pwd)
src=$out_abs/source
applied=$out_abs/applied-template
popped=$out_abs/popped-template

cd "$src"
mkdir patches

for i in $(seq -w 0 199); do
	{
		printf 'file %s\n' "$i"
		for n in $(seq 1 20); do
			printf 'base line %02d\n' "$n"
		done
	} >"file-$i.txt"
done

for p in $(seq -w 0 79); do
	run_quilt new "patch-$p.diff" >/dev/null
	for j in 0 1 2; do
		idx=$(printf '%03d' $(((10#$p * 3 + j) % 200)))
		file="file-$idx.txt"
		run_quilt add "$file" >/dev/null
		printf 'patch %s change %s\n' "$p" "$j" >>"$file"
	done
	run_quilt refresh --no-timestamps >/dev/null
done

cp -a "$src" "$applied"
run_quilt pop -a >/dev/null
cp -a "$src" "$popped"

cat >"$out_abs/fold.diff" <<'EOF'
--- a/file-000.txt
+++ b/file-000.txt
@@ -21,3 +21,4 @@
 base line 20
 patch 00 change 0
 patch 66 change 2
+folded benchmark change
EOF

printf 'Created benchmark fixture in %s\n' "$out"
printf 'Applied template: %s\n' "$applied"
printf 'Popped template: %s\n' "$popped"
printf 'Fold input: %s\n' "$out_abs/fold.diff"
