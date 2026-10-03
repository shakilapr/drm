# ADR-0005: Architecture vs specification terminology; no directory reorganization

## Status

`accepted`
Date: 2026-10-03 · Deciders: project owner

## Context and Problem Statement

Current usage calls `docs/architecture*.md` the "normative core specification" and `docs/spec/` "non-normative derived specifications". That inverts the usual relationship: an architecture description states how and why the system is structured, while a specification states behavior an implementation must follow. Renaming or moving files now would touch many cross-references.

## Decision Drivers

- Clear, stable meaning of each document class.
- No mass cross-reference churn in a solo repository.
- Compatibility with the documentation set already described in codebase.md.

## Considered Options

- **Full directory reorganization** (`docs/architecture/`, split specs) — deferred: high churn, no functional gain today; revisit when the documentation set grows.
- **Terminology decision only** — chosen.
- **Keep current wording** — rejected: the inversion misleads new readers and future tooling.

## Decision Outcome

Chosen: terminology is adopted now; file layout is unchanged.

- **Architecture** (`docs/architecture.md`, `docs/spatial_architecture.md`, `docs/ui-architecture.md`): how and why DRM is structured; remains normative and wins conflicts.
- **Specification** (`docs/spec/`): exact behavior an implementation must follow, derived from architecture and algorithm docs.
- **Requirements** (`docs/req/`): what the system is required to achieve.
- **Algorithms** (`docs/algorithms/`): canonical implementation algorithms.
- **ADRs** (`docs/adr/`): why a significant decision was made.

Any future directory reorganization requires a new ADR.

### Positive Consequences

- Document classes have unambiguous meaning without moving files.

### Negative Consequences

- Directory names still reflect the old split until a future reorganization.

## Confirmation

None required (terminology decision). codebase.md and new documents must use these meanings.

## More Information

- Accepted ADRs are immutable; supersession creates a new record (codebase.md 6.3).
- Conventions: `docs/standards.md`.
