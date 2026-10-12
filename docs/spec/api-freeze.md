# API Freeze Note — v1.0

Derived component spec (codebase.md §2.4; non-normative — the algorithm docs
[qualification.md](../algorithms/program/qualification.md) and
[c-abi-surface.md](../algorithms/bindings/c-abi-surface.md) win conflicts).
`REQ-QAL-023`; the freeze executed in M24W2 (`API_FROZEN = 1`). Machine form:
[xtask/src/freeze.rs](../../xtask/src/freeze.rs). The surface locked here is the
freeze set from [api-inventory.md](api-inventory.md) (`REQ-QAL-021`).

## What is frozen

From this note onward the following are **append-only**: never renamed, never
renumbered, never re-typed. Additions are allowed; any layout change bumps
`DRM_ABI_VERSION` and reopens the freeze.

| Surface | Frozen scope | Rule |
|---|---|---|
| Canonical C ABI (`bindings/c/include/drm/drm.h`) | exports, opaque handles, `drm_status` codes | append-only (I-A8); new codes get fresh numbers |
| Rust runtime crates (`drm-types`, `drm-core`, `drm-router`) | public freeze candidates | semver-checked; new items only |
| Bindings crates (`bindings/rust`, `bindings/python`) | public items | append-only |
| Lockstep version | every crate + binding at one version | codebase §6.1 |

The deferred post-v1.0 exports (`drm_write_*`, `drm_copy_*`, `drm_borrow_next`,
`drm_log`) are **excluded** from the freeze set; they land only after v1.0.

## Version lock

`cargo xtask freeze` reads every declared manifest from disk and verifies each is
at the locked release version:

| Manifest | Lock source |
|---|---|
| `Cargo.toml` (`[workspace.package] version`) | workspace crates (`version.workspace = true`) |
| `bindings/rust/crates/drm/Cargo.toml` | Rust SDK crate (workspace version) |
| `bindings/python/pyproject.toml` | Python wheel (`[project] version`) |
| `bindings/python/native/Cargo.toml` | Python native core (workspace version) |
| `packages/studio-protocol/package.json` | Studio protocol package |
| `packages/studio-extension-api/package.json` | Studio extension API package |

A manifest that diverges from the locked version fails the command.

## Artifact stamping

Every released artifact is stamped with the deployment identity inputs
(deployment-identity.md, `REQ-CORE-085`) computed as
`hash(ABI version, topic schemas/layouts, declared interfaces)`. The ABI version
in the stamp is `DRM_ABI_VERSION` from `drm.h`; the fingerprint is 128 bits. The
stamp is recomputed from the declared inputs, so a stamp that disagrees with the
inputs is a gate failure — the deployment identity is never configured, only
derived.

## Verification

```text
cargo xtask freeze --check --json target/freeze-report.json
```

`API_FROZEN = 1` iff versions are locked, every frozen surface re-resolves, every
stamp matches its declared inputs, and this note is published. The freeze gate is
the CI `freeze` job.

REQ: REQ-QAL-023.
