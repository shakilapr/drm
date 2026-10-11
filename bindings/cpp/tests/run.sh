#!/bin/sh
# bindings/cpp/tests/run.sh — build and run the C++ Borrow<T> RAII driver under
# BOTH g++ and clang++ with -std=c++20 -Wall -Wextra -Werror, link the two-TU
# ODR check, and assert every compile-fail fixture is rejected.
#
# Stub-backed by default via the test-only C stub (drm_stub.c); set
# DRM_CPP_REAL_FFI=1 to additionally link the real drm-ffi cdylib
# (target/debug/libdrm_ffi.so) when it has been built.
#
# REQ: REQ-BIND-013 (I-B1), REQ-BIND-011 (I-B2 compile-fail).
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(CDPATH= cd -- "$here/../../.." && pwd)
inc="$here/../include"
cinc="$here/../../c/include"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT INT TERM

INCS="-I$inc -I$cinc"

CXXFLAGS="-std=c++20 -Wall -Wextra -Werror"
CFLAGS="-std=c11 -Wall -Wextra -Werror"

fail() {
    echo "run.sh: FAIL: $*" >&2
    exit 1
}

cc_bin=${CC:-cc}
command -v "$cc_bin" >/dev/null 2>&1 || fail "no C compiler ($cc_bin)"

# Compile the test-only stub of the 12 ABI exports once.
"$cc_bin" $CFLAGS $INCS -c "$here/drm_stub.c" -o "$work/drm_stub.o" \
    || fail "stub compile failed"

for cxx in g++ clang++; do
    command -v "$cxx" >/dev/null 2>&1 || fail "missing compiler: $cxx"

    echo "== $cxx: borrow_raii (stub-backed) =="
    # shellcheck disable=SC2086
    "$cxx" $CXXFLAGS $INCS -DDRM_CPP_TEST_HOOKS \
        "$here/borrow_raii.cpp" "$work/drm_stub.o" -o "$work/borrow_raii_$cxx" \
        || fail "$cxx: driver compile failed"
    "$work/borrow_raii_$cxx" || fail "$cxx: driver run failed"

    echo "== $cxx: ODR two-TU link =="
    # shellcheck disable=SC2086
    "$cxx" $CXXFLAGS $INCS \
        "$here/odr_tu_a.cpp" "$here/odr_tu_b.cpp" "$work/drm_stub.o" -o "$work/odr_$cxx" \
        || fail "$cxx: ODR link failed"
    "$work/odr_$cxx" || fail "$cxx: ODR run failed"

    echo "== $cxx: compile-fail fixtures (must NOT compile) =="
    found=0
    for f in "$here"/compile_fail/*.cpp; do
        [ -e "$f" ] || break
        found=1
        # shellcheck disable=SC2086
        if "$cxx" $CXXFLAGS $INCS -c "$f" -o "$work/cf.o" 2>"$work/cf.err"; then
            fail "$cxx: fixture unexpectedly compiled: $f"
        fi
        echo "   $cxx: correctly rejected $(basename "$f")"
    done
    [ "$found" -eq 1 ] || fail "no compile-fail fixtures found under $here/compile_fail"
done

# Optional: link the real drm-ffi cdylib (no test hooks) when requested.
if [ "${DRM_CPP_REAL_FFI:-0}" = "1" ]; then
    so="$root/target/debug/libdrm_ffi.so"
    [ -f "$so" ] || fail "DRM_CPP_REAL_FFI=1 but $so is missing (run: cargo build -p drm-ffi)"
    for cxx in g++ clang++; do
        echo "== $cxx: borrow_raii vs real drm-ffi =="
        # shellcheck disable=SC2086
        "$cxx" $CXXFLAGS $INCS "$here/borrow_raii.cpp" \
            -L"$(dirname "$so")" -ldrm_ffi -Wl,-rpath,"$(dirname "$so")" \
            -o "$work/borrow_raii_real_$cxx" || fail "$cxx: real-ffi link failed"
        "$work/borrow_raii_real_$cxx" || fail "$cxx: real-ffi run failed"
    done
fi

echo "run.sh: PASS"
