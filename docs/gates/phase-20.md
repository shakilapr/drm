# Phase 20 Gate — C++ Binding

Evidence is executable and stays green under the permanent gates.
Verdict: **PASS** (RAII substrate, scoping, and 4-language golden identity);
the generated per-manifest node API lands with the code generator in Phase 24.

Implementation: header-only `bindings/cpp/include/drm/drm.hpp`
([bindings-cpp-rust-python.md](../algorithms/bindings/bindings-cpp-rust-python.md)
§C++) over the C ABI ([c-abi-surface.md](../algorithms/bindings/c-abi-surface.md)).

## Criteria ⇄ evidence

| Criterion | Evidence | Status |
|---|---|---|
| Every schema maps correctly | C++ decode-only golden reader verifies 35/35 vectors (24 primitives + 10 composites + descriptor) under g++ 13.3 and clang++ 18.1 — closes the Phase 4 C++ deferral (REQ-BIND-013); full generated node API is Phase 24 | PASS (golden) / PARTIAL (generator) |
| Borrow releases on every scope-exit path incl. exceptions | `borrow_raii.cpp` (normal exit, early return, exception unwinding, move) — release exactly once, proven by counter + ABI single-borrow rule; g++ 13.3 + clang++ 18.1 | PASS |
| No accessor returns a pointer/reference/span into the slot; retention requires copy | `Borrow<T>` exposes payload via a bounded accessor; the compile-fail fixture `raw_pointer_escape.cpp` rejects the forbidden pointer escape (I-B2) | PASS |
| Undeclared operations absent from the generated node API | `compile_fail/undeclared_op.cpp` (I-B2); full generator is Phase 24 | PARTIAL |
| C++↔C/Rust/Python interoperability matrix | C++ driver links against the real `drm-ffi` cdylib; the golden reader is the 4th independent decode path | PASS (4-lang golden) |

## Invariant coverage (bindings-cpp-rust-python.md I-B1/I-B2)

| Invariant | Test |
|---|---|
| I-B1 BorrowDiscipline (C++) | `bindings/cpp/tests/borrow_raii.cpp` |
| I-B2 NodeApiScoped | `bindings/cpp/tests/compile_fail/*` |

## Variables (M07W2 → M07W4)

| Variable | Target | Measured |
|---|---|---|
| `CPP_BORROW_GREEN` | RAII release on every scope exit incl. exceptions (I-B1) | `borrow_raii` g++ + clang++ |
| `CPP_NODE_API` | undeclared ops absent (compile-fail, I-B2) | 3 compile-fail fixtures |
| `CPP_INTEROP` | C++↔C/Rust/Python cells green | cdylib-linked driver green; full matrix pending |

## Requirements covered

REQ-BIND-013 (C++ binding), REQ-BIND-001/006 (borrow discipline), REQ-BIND-021
(RAII lifecycle), REQ-BIND-002/004 (handle ownership).

## Notes

- The generated node-API surface (per-manifest, schema-mapped) lands with the
  code generator in Phase 24; this gate proves the hand-written RAII substrate
  and the scoping contract it must satisfy.
