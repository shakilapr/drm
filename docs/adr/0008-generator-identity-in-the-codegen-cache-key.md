# ADR-0008: Generator identity in the codegen cache key

## Status

`accepted`
Date: 2026-10-14 · Deciders: E2 (toolchain)

## Context and Problem Statement

`xtask gen-check` was green while the committed generated trees were stale. After
a renderer fix (`Default` impl emission for generated Rust topic modules,
commit `b07d072`), a plain `drm build` on a workspace with an existing
`.drm/gen` tree reported `64 generated (0 rendered)`: every unit's fingerprint
still matched, so `plan_with` skipped it
(`crates/drm-workspace/src/codegen/mod.rs`), and the tree kept modules produced
by the previous renderer. The documented lever — "generator version bump"
(`codegen-invalidation.md`, I-G2 edge case) — is a hand-maintained constant
(`GENERATOR_VERSION`), and nothing forced it to move when the renderer changed.
Falsified: a renderer revision that changes output bytes did **not** invalidate
affected artifacts.

The Codegen & Invalidation algorithm documented the unit fingerprint input as
`hash(schema, topic_identity, generator_version, options)` — a value that a human
must keep truthful.

## Decision Drivers

- `I-G2` (TimestampPreserved): a changed fingerprint regenerates; byte-identical
  output keeps its mtime, so over-invalidation costs no churn.
- Phase 0 acceptance: "changing a renderer invalidates affected generated
  artifacts" and "an unchanged build is correctly cached" must both hold.
- Determinism: the same sources must always yield the same cache key, from a
  clean checkout, on every host.
- No new inputs to the *contract* fingerprint (`FINGERPRINT_FORMAT_VERSION` stays
  frozen); this concerns codegen unit fingerprints only.

## Considered Options

- **Derive a generator identity from the renderer sources** — chosen. A
  128-bit digest over the renderer's own bytes (`codegen/mod.rs` +
  `codegen/render.rs`, embedded with `include_str!`) is folded into every topic
  and node fingerprint. An edited renderer changes the identity, so every
  affected unit's cache key moves; an untouched renderer reproduces the identity
  exactly, so an unchanged build stays cached.
- **Gate the hand-maintained version** — rejected: it requires a committed digest
  per version and a reviewer to keep them paired; a developer can refresh the
  digest without bumping the version and the gate cannot tell.
- **Content-compare every unit on every build** — rejected: it renders all
  `topics × 4 languages` on each build (the cost `plan` exists to avoid) and the
  byte comparison still needs a write decision.
- **Timestamp/polling (mtime of renderer sources)** — rejected: filesystem
  mtimes are not reproducible from a clean checkout (git does not preserve them),
  which the determinism driver forbids.

## Decision Outcome

Chosen: **derive the generator identity from the renderer sources and fold it
into the unit fingerprint.**

- `generator_identity()` returns `BLAKE3_128` over the concatenated bytes of the
  renderer sources, memoised per process.
- `topic_fingerprint` / `node_fingerprint` include it; the
  `*_with_generator` and `plan_with_generator` variants take it explicitly so the
  invalidation property is testable without editing sources.
- `GENERATOR_VERSION` stays for deliberate semantic bumps.

### Positive Consequences

- A renderer change regenerates affected artifacts with no human step
  (`gen_generator_identity_invalidates_and_caches`).
- Rebuilding an unchanged tree stays cached, and I-G2 keeps mtimes stable.
- The frozen contract fingerprint format is untouched.

### Negative Consequences

- The identity changes for edits that do not alter output (comments, formatting),
  causing one extra render — bounded by I-G2 to a byte-compare plus a metadata
  update, never a rewrite.
- Renderer bytes are embedded in the binary (≈ tens of KiB), and the digest is
  computed once per process.

## Confirmation

`cargo test -p drm-workspace --test codegen_invariants gen_generator_identity_invalidates_and_caches`:
a plan under a different generator identity dirties the affected topic and node
units; re-planning under the same identity reports no dirty unit; both
fingerprint kinds differ across identities; the shipped identity is a real digest.

## More Information

- [codegen-invalidation.md](../algorithms/toolchain/codegen-invalidation.md) I-G2,
  `REQ-TOOL-027`.
- [attached-data-path.md](../algorithms/runtime/attached-data-path.md) is
  unrelated; this ADR concerns codegen caching only.
- ADRs are immutable; supersession creates a new record (codebase.md 6.3).
