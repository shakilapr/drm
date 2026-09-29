# ADR-0002: drm.h is handwritten; cbindgen rejected

- **Status**: accepted
- **Date**: 2026-09-29
- **Deciders**: E4 (bindings), E1 (runtime)

## Context

The C ABI (`bindings/c/include/drm/drm.h`) needs a stable, hand-reviewable header that is
the single source of truth for `drm-ffi` exports (codebase.md 3.3, c-abi-surface.md).
Generated headers risk drift between the Rust implementation and the published ABI, and
Rust->C generation cannot express the fixed `int32_t` status vocabulary, `_Static_assert`s,
or the C11 freestanding subset the MCU targets require.

## Alternatives

1. **cbindgen-generated header** — automatic sync with Rust; rejected: source of truth
   inverts (Rust defines ABI), cannot emit `_Static_assert`, opaque-type discipline, or the
   explicit-value status block without hand-maintained annotations; regeneration churn on
   unrelated edits; clean-room constraint on review.
2. **Handwritten header + conformance tests** — chosen.
3. **Doxygen-annotated C header parsed by custom tooling** — over-engineering for a
   surface that is intentionally small and append-only.

## Decision

`drm.h` is handwritten and normative (status: single source of truth). `drm-ffi` exports
must match it 1:1; `bindings/c/tests/` enforces ABI conformance (layout golden vectors,
`status_map`, static asserts). Header changes require ADR-grade review; the header is
never generated.

## Consequences

- Hand sync between `drm.h` and `drm-ffi` — mitigated by conformance tests + semver-checks.
- MCU freestanding subset stays enforceable by inspection.
- `DRM_ABI_VERSION` remains a deliberate, reviewable constant.
