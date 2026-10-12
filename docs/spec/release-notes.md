# v1.0 Release Notes

Derived component spec (codebase.md §2.4; non-normative — the algorithm doc
[qualification.md](../algorithms/program/qualification.md) wins conflicts).
`REQ-QAL-028` (release candidate) and `REQ-QAL-030` (v1.0 release). Machine form:
[xtask/src/release.rs](../../xtask/src/release.rs) (candidate fold) and
[xtask/src/closeout.rs](../../xtask/src/closeout.rs) (release + program
close-out). The candidate was drafted in M24W3; the dated release lands with the
v1.0 tag in M24W4.

## Release

`cargo xtask closeout --check` folds the completed Phase 34 qualification packet
into the v1.0 release. The release is tagged with the stamped deployment
identity recomputed from the declared inputs (`REQ-CORE-085`), carries the
frozen API version (`DRM_ABI_VERSION`), and is built for every declared-supported
target.

| Field | Value |
|---|---|
| Tag | `v1.0.0` |
| Release version | `0.1.0` (workspace lockstep, codebase §6.1) |
| ABI version | `DRM_ABI_VERSION` from `bindings/c/include/drm/drm.h` |
| Deployment identity | recomputed from the pilot stamp (`REQ-CORE-085`) |
| Qualification packet | `I-QAL1`..`I-QAL9` complete |
| API freeze | `API_FROZEN = 1` (`REQ-QAL-023`) |
| Release targets | `ubuntu-x86_64`, `ubuntu-aarch64`, `windows-x86_64`, `h743`, `f407` |
| Publish set | crates + C/C++ headers + Rust SDK + Python wheel + Studio packages (codebase §2.3) |

`V10_RELEASED = 1` (release built + published + identity stamped).

## Highlighted changes

- Phase 34 whole-system qualification packet complete: traceability `I-QAL1`,
  fault/soak `I-QAL2`, repro `I-QAL3`, conformance `I-QAL4`, perf `I-QAL5`,
  coverage/mutation `I-QAL8`/`I-QAL9` (`REQ-QAL-027`).
- v1.0 API freeze executed: versions locked, frozen surfaces append-only, artifact
  stamps carry their ABI version + contract fingerprint (`REQ-QAL-023`).
- Pilot deployed (router + MCU + Studio UI) with a stamped deployment identity and
  a clean rollback drill (`REQ-QAL-024`/`REQ-QAL-025`); pilot bundles verified
  (`REQ-QAL-026`).
- Waiver discipline (`REQ-QAL-029`): a red evidence line is only cleared by a
  recorded waiver with a rationale and a review quorum, never silently.
- Program close-out (`REQ-QAL-030`/`REQ-QAL-031`/`REQ-QAL-032`): v1.0 released,
  post-1.0 backlog groomed, program retrospective complete.

## Migration notes

v1.0 is the first frozen release; there is no earlier frozen ABI to migrate from.
For applications built against the pre-freeze drafts:

- Rebuild against the frozen `drm.h`; the ABI is one-way compatible
  (`REQ-BIND-018`) — older applications run against newer libraries, and newer
  applications require the newer runtime (recorded in node contract
  fingerprints).
- Regenerate bindings (`drm dev`) so the compiled contract fingerprint matches
  the deployment expectation; a mismatch refuses startup (`REQ-CORE-092`).
- Deployment IDs are runtime-only (`deployment.json`); no recompile is needed
  for placement/routing/ID changes (`REQ-CORE-084`, `REQ-CORE-089`).

## Announcement

DRM v1.0 is released: a decentralized, contract-driven data middleware for
deterministic robotics and computer systems on Linux and Windows. Local
zero-copy shared-memory IPC, automatic boundary routing, and build-time
verification, with first-party C, C++, Rust, and Python bindings.

## Verification

```text
cargo xtask release --check --json target/release-candidate.json
cargo xtask closeout --check --json target/closeout-report.json
cargo xtask qualify --check
```

REQ: REQ-QAL-028, REQ-QAL-030.
