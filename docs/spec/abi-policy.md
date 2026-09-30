# C ABI Compatibility Policy

Scope: the stable C ABI surface — `bindings/c/include/drm/drm.h` (handwritten, single
source of truth, ADR 0002) and the exported symbols of crate `drm-ffi`. Implements
architecture.md §Canonical C ABI; work-plan M01W3-E4-T5.

## Rules

| Rule | Statement |
|---|---|
| Append-only | Symbols and status codes are added; never renamed, never removed. |
| Codes | New `DRM_ERR_*` codes get explicit fresh values forever; never renumber, never reuse. |
| Layout change | Any change to the layout of an exported struct or calling convention bumps `DRM_ABI_VERSION` (major). |
| Additions | Pure additions (new symbols, new trailing-optional capability) do not bump the version. |
| One-way compatibility | Old application × newer library is guaranteed. A newer application × older library links only if it uses no newer symbols (link failure otherwise); node contract fingerprints record the required ABI (deployment-identity.md). |
| Deprecation | A deprecated symbol is marked in `drm.h` and kept for a two-version window before removal eligibility; removal requires a version bump and a release-notes entry. |
| Status representation | `drm_status` is fixed `int32_t`; unknown codes surface as `UnknownStatus(code)` in bindings, never a `repr(C)` fieldless enum (I-A9). |
| Fail-stop | No `DRM_ERR_INTERNAL`; expected conditions return precise codes, internal invariant violation terminates the node (I-A6). |

## Mechanical checks

- `drm-ffi` test `drm_h_status_map_exhaustive`: drm.h codes ↔ `Status::code()` set 1:1 both
  directions; Rust side is a wildcard-free exhaustive `match` (compile-time gate). REQ-BIND-007.
- Header syntax-only builds: `cc -fsyntax-only -std=c99` and `-std=c11` across two compilers
  (gcc, clang) — the `-fshort-enums` hazard is excluded by construction (no C enum in ABI). REQ-BIND-017.

REQ: REQ-BIND-017, REQ-BIND-018, REQ-BIND-009.
