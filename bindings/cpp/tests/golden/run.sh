#!/bin/sh
# bindings/cpp/tests/golden/run.sh — build and run the decode-only C++ golden
# reader under BOTH g++ and clang++ with -std=c++20 -Wall -Wextra -Werror.
#
# Decodes every committed canon-codec golden fixture and asserts the decoded
# values match the fixture metadata byte-for-byte:
#   primitives/index.json (+ <stem>.hex)   — 24 vectors
#   composites/<stem>.json (+ <stem>.hex)  — 10 vectors
#   descriptor/descriptor_golden.json      —  1 vector
# This is the fourth independent decoder (Rust/C/Python/C++) and closes the
# Phase 4 C++ deferral.
#
# REQ: REQ-BIND-013, REQ-TOOL-042, REQ-TOOL-043, REQ-TOOL-044 (canon-codec I1).
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../../../.." && pwd)
fixtures="$root/tests/fixtures/golden"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT INT TERM

CXXFLAGS="-std=c++20 -Wall -Wextra -Werror"

fail() {
    echo "run.sh: FAIL: $*" >&2
    exit 1
}

[ -d "$fixtures/primitives" ] || fail "missing fixtures dir: $fixtures"

summary=""
for cxx in g++ clang++; do
    command -v "$cxx" >/dev/null 2>&1 || fail "missing compiler: $cxx"

    echo "== $cxx: golden_decode =="
    # shellcheck disable=SC2086
    "$cxx" $CXXFLAGS "$here/golden_decode.cpp" -o "$work/golden_$cxx" \
        || fail "$cxx: golden_decode compile failed"
    out=$("$work/golden_$cxx" "$fixtures") || fail "$cxx: golden_decode run failed"
    echo "$out"
    summary=$(printf '%s\n' "$out" | tail -n1)
done

echo "run.sh: PASS ($summary; g++ + clang++)"
