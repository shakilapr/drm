# Pilot Packaging

Derived component spec (codebase.md §2.4; non-normative — the algorithm doc
[qualification.md](../algorithms/program/qualification.md) wins conflicts).
`REQ-QAL-026`; the release close-out packaging dry-run executed in M24W2.
Machine form: [xtask/src/packaging.rs](../../xtask/src/packaging.rs).

## Bundle set

`cargo xtask package --dry-run` assembles the declared pilot bundle set from
their sources. Each bundle names the artifact kind, its repository-relative
source, and the package rules it obeys. The dry-run renders the bundle manifest
that the real pilot bundles consume; `--check` fails while any source does not
resolve.

| Bundle | Kind | Source | Rule |
|---|---|---|---|
| `drm-crates` | workspace crates | `crates/` | lockstep version (codebase §6.1) |
| `drm-c` | C header + library | `bindings/c/include` | single-source `drm.h`, C11 (codebase §3.3) |
| `drm-cpp` | C++ headers | `bindings/cpp/include` | C++20 RAII surface (codebase §3.3) |
| `drm-rust` | Rust SDK crate | `bindings/rust/crates/drm` | safe wrapper over `drm-ffi` (codebase §3.2) |
| `drm-python` | Python wheel source | `bindings/python` | PyO3 native core + facade (codebase §3.4) |
| `drm-studio-protocol` | Studio package | `packages/studio-protocol` | generated output (§2.3 R2) |
| `drm-studio-extension-api` | Studio package | `packages/studio-extension-api` | only permitted extension surface (§2.3 R3) |

Packages under `packages/*` are consumed by apps and never import apps
(codebase §2.3 R1); `drm-visual-schemas` stays out of DRM core permanently
(§2.3 R4) and is not a DRM pilot bundle.

## Verification

```text
cargo xtask package --dry-run --json target/packaging-report.json
cargo xtask package --check
```

`BUNDLES_VERIFIED = resolved/total`; the bundle set is complete iff every
declared source resolves. The packaging gate is the CI `packaging` job.

REQ: REQ-QAL-026.
