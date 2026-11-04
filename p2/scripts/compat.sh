#!/usr/bin/env bash
set -u

usage() {
    cat <<'EOF'
Usage: scripts/compat.sh MODE [TEST...]

Modes:
  gnu-tests              run GNU patch tests against p2
                         (aliases: upstream-tests, patch-tests-against-p2)
  p2-fixtures            run p2 parity fixtures against p2 and GNU patch
  p2-fixtures-on-patch   run p2 parity fixtures with GNU patch as implementation
                         (alias: p2-tests-against-patch)
  q-tests-against-p2     run q tests with p2 shimmed in as patch/gpatch
  all                    run p2-fixtures, p2-fixtures-on-patch, and all GNU tests

Environment:
  GNU_PATCH_DIR          path to a GNU patch checkout (required for GNU tests)
  Q_DIR                  path to the q checkout (default: the parent of p2)
  CARGO                  cargo binary (default: cargo)
  P2_REFERENCE_PATCH     reference patch binary (default: patch)
  P2_UNDER_TEST          implementation used by p2 fixtures

If no GNU tests are passed, gnu-tests runs every discovered GNU patch test file.
Pass explicit GNU test names or paths to focus on a subset.
EOF
}

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cargo_bin="${CARGO:-cargo}"
gnu_patch_dir="${GNU_PATCH_DIR:-}"
q_dir="${Q_DIR:-$(dirname "$repo_root")}"
reference_patch="${P2_REFERENCE_PATCH:-patch}"
p2_bin="$repo_root/target/debug/p2"

mode="${1:-all}"
if [[ $# -gt 0 ]]; then
    shift
fi

build_p2() {
    (cd "$repo_root" && "$cargo_bin" build)
}

run_p2_fixtures() {
    local under_test=$1
    echo "== p2 fixtures: reference=$reference_patch under-test=$under_test"
    (
        cd "$repo_root" &&
        P2_REFERENCE_PATCH="$reference_patch" \
        P2_UNDER_TEST="$under_test" \
        "$cargo_bin" test --test parity
    )
}

all_gnu_tests() {
    local path
    for path in "$gnu_patch_dir"/tests/*; do
        [[ -f "$path" ]] || continue
        case "${path##*/}" in
            Makefile*|test-lib.sh) ;;
            *) printf '%s\n' "${path##*/}" ;;
        esac
    done | sort
}

run_gnu_tests() {
    if [[ -z "$gnu_patch_dir" || ! -d "$gnu_patch_dir/tests" ]]; then
        echo "GNU patch tests not found; set GNU_PATCH_DIR to a GNU patch checkout" >&2
        exit 2
    fi

    local tests=("$@")
    if [[ ${#tests[@]} -eq 0 ]]; then
        mapfile -t tests < <(all_gnu_tests)
    fi

    local tmp failures=0 test_name test_path output_path status
    tmp="$(mktemp -d)"
    mkdir -p "$tmp/tests"

    echo "== GNU patch tests against p2: ${tests[*]}"
    for test_name in "${tests[@]}"; do
        if [[ "$test_name" == */* ]]; then
            test_path="$test_name"
        else
            test_path="$gnu_patch_dir/tests/$test_name"
        fi

        if [[ ! -f "$test_path" ]]; then
            echo "missing GNU test: $test_name" >&2
            failures=$((failures + 1))
            continue
        fi

        output_path="$tmp/$(basename "$test_path").out"
        printf 'gnu %-32s ' "$(basename "$test_path")"
        env \
            srcdir="$gnu_patch_dir/tests" \
            abs_top_builddir="$tmp" \
            PATCH="$p2_bin" \
            sh "$test_path" >"$output_path" 2>&1
        status=$?
        if [[ $status -eq 77 ]] || { [[ $status -eq 0 ]] &&
            grep -qi 'skipping this test\|skipping test\|test skipped' "$output_path"; }; then
            echo "skip"
        elif [[ $status -eq 0 ]]; then
            echo "ok"
        else
            echo "FAIL"
            sed -n '1,120p' "$output_path"
            failures=$((failures + 1))
        fi
    done

    rm -rf "$tmp"
    if [[ $failures -ne 0 ]]; then
        exit 1
    fi
}

run_q_tests_against_p2() {
    if [[ ! -d "$q_dir" ]]; then
        echo "q checkout not found at $q_dir" >&2
        exit 2
    fi

    local tmp status
    tmp="$(mktemp -d)"
    printf '#!/usr/bin/env bash\nexec %q "$@"\n' "$p2_bin" >"$tmp/patch"
    cp "$tmp/patch" "$tmp/gpatch"
    chmod +x "$tmp/patch" "$tmp/gpatch"

    echo "== q tests against p2: q=$q_dir patch-shim=$tmp"
    (cd "$q_dir" && PATH="$tmp:$PATH" "$cargo_bin" test "$@")
    status=$?
    rm -rf "$tmp"
    return "$status"
}

case "$mode" in
    -h|--help)
        usage
        ;;
    gnu-tests|upstream-tests|patch-tests-against-p2)
        build_p2 && run_gnu_tests "$@"
        ;;
    p2-fixtures)
        build_p2 && run_p2_fixtures "${P2_UNDER_TEST:-$p2_bin}"
        ;;
    p2-fixtures-on-patch|p2-tests-against-patch)
        run_p2_fixtures "$reference_patch"
        ;;
    q-tests-against-p2)
        build_p2 && run_q_tests_against_p2 "$@"
        ;;
    all)
        build_p2 &&
            run_p2_fixtures "${P2_UNDER_TEST:-$p2_bin}" &&
            run_p2_fixtures "$reference_patch" &&
            run_gnu_tests "$@"
        ;;
    *)
        usage >&2
        exit 2
        ;;
esac
