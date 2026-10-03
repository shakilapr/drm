# ADR-0002: drm.h is handwritten; cbindgen rejected

## Status

`accepted`
Date: 2026-09-29 · Deciders: E4 (bindings), E1 (runtime)

## Context and Problem Statement

The C ABI (`bindings/c/include/drm/drm.h`) needs a stable, hand-reviewable header that is the single source of truth for `drm-ffi` exports (codebase.md 3.3, c-abi-surface.md). Generated headers risk drift between the Rust implementation and the published ABI, and Rust→C generation cannot express the fixed `int32_t` status vocabulary, `_Static_assert`s, or the C11 freestanding subset the MCU targets require.

## Decision Drivers

- Header must be the single source of truth for the ABI.
- MCU freestanding subset must stay enforceable by inspection.
- Status vocabulary and static asserts must be expressible exactly.

## Considered Options

- **cbindgen-generated header** — rejected: source of truth inverts (Rust defines ABI); cannot emit `_Static_assert`, opaque-type discipline, or the explicit-value status block without hand-maintained annotations; regeneration churn; clean-room constraint on review.
- **Handwritten header + conformance tests** — chosen.
- **Doxygen-annotated header parsed by custom tooling** — rejected: over-engineering for a deliberately small, append-only surface.

## Decision Outcome

Chosen: `drm.h` is handwritten and normative (single source of truth). `drm-ffi` exports must match it 1:1; `bindings/c/tests/` enforces ABI conformance (layout golden vectors, `status_map`, static asserts). Header changes require ADR-grade review; the header is never generated.

### Positive Consequences

- MCU freestanding subset stays enforceable by inspection.
- `DRM_ABI_VERSION` remains a deliberate, reviewable constant.

### Negative Consequences

- Hand sync between `drm.h` and `drm-ffi` — mitigated by conformance tests and semver checks.

## Confirmation

`bindings/c/tests/` (layout golden vectors, `status_map`, static asserts) — existing and permanent gate.

## More Information

- Accepted ADRs are immutable; supersession creates a new record (codebase.md 6.3).
- Normative home: `docs/spec/abi-policy.md`, `docs/algorithms/bindings/c-abi-surface.md`.
