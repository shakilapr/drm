//! trybuild compile-fail set (bindings-cpp-rust-python.md §Test Matrix, I-B4):
//! safe code must not be able to read a payload after guard drop, reuse a
//! moved-from guard, or move/share the guard across threads. Fixtures live in
//! `tests/ui/`; each must fail to compile.
//! REQ: REQ-BIND-016.

/// REQ: REQ-BIND-016 — every forbidden borrow/threading case is rejected by the
/// compiler; the fixtures are never expected to build.
#[test]
fn compile_fail() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/*.rs");
}
