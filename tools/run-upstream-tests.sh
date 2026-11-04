#!/usr/bin/env bash
# Run upstream quilt's test suite against q, using the same environment as
# upstream's `make check` (test/test.quiltrc, QUILT_PATCHES_PREFIX, %{P}).
set -u

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
quilt_src=${QUILT_SRC:-"$HOME/quilt"}

cargo build --manifest-path "$repo/Cargo.toml" >/dev/null || exit 1

cd "$quilt_src/test" || exit 1

if [ "$#" -eq 0 ]; then
	set -- *.test
fi

status=0
for test in "$@"; do
	test=${test#test/}
	out=$(mktemp)
	if env \
		-u QUILT_COLORS \
		-u QUILT_DIFF_OPTS \
		-u QUILT_PATCHES \
		-u QUILT_PATCHES_ARGS \
		-u QUILT_PATCH_OPTS \
		-u QUILT_PUSH_ARGS \
		-u QUILT_SERIES \
		-u QUILT_SERIES_ARGS \
		-u POSIXLY_CORRECT \
		LANG=C LC_ALL=C \
		PATH="$repo/target/debug:$repo/tools/bin:$PATH" \
		QUILT_PC=.pc \
		QUILT_DIR="$repo/tools/quilt" \
		QUILTRC="$quilt_src/test/test.quiltrc" \
		P=patches/ _P=../patches/ \
		./run -q "$test" >"$out" 2>&1
	then
		printf '%-36s %s\n' "$test" "$(tail -n 1 "$out")"
	else
		status=1
		printf '%-36s %s\n' "$test" "$(tail -n 1 "$out")"
		if [ "${VERBOSE_FAILURES:-0}" != 0 ]; then
			cat "$out"
		fi
	fi
	rm -f "$out"
done

exit "$status"
