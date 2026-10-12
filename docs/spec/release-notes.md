# v1.0 Release Notes (Release Candidate)

Derived component spec (codebase.md §2.4; non-normative — the algorithm doc
[qualification.md](../algorithms/program/qualification.md) wins conflicts).
`REQ-QAL-028`; the v1.0 release candidate drafted in M24W3. Machine form:
[xtask/src/release.rs](../../xtask/src/release.rs). The dated release notes land
with the v1.0 tag in M24W4; this file is the drafted note the RC folds in.

## Candidate

`cargo xtask release --check` folds the completed Phase 34 qualification packet
into the v1.0 release candidate. The candidate is tagged with the stamped
deployment identity recomputed from the declared inputs (`REQ-CORE-085`), folds
the frozen API version (`DRM_ABI_VERSION`), and requires the release notes
drafted for the RC tag.

| Field | Value |
|---|---|
| Tag | `v1.0.0-rc.1` |
| Release version | `0.1.0` (workspace lockstep, codebase §6.1) |
| ABI version | `DRM_ABI_VERSION` from `bindings/c/include/drm/drm.h` |
| Deployment identity | recomputed from the pilot stamp (`REQ-CORE-085`) |
| Qualification packet | `I-QAL1`..`I-QAL9` complete |
| API freeze | `API_FROZEN = 1` (`REQ-QAL-023`) |

`V10_RC = 1` (candidate complete: packet + freeze + identity + notes + waiver).

## Highlighted changes

- Phase 34 whole-system qualification packet complete: traceability `I-QAL1`,
  fault/soak `I-QAL2`, repro `I-QAL3`, conformance `I-QAL4`, perf `I-QAL5`,
  coverage/mutation `I-QAL8`/`I-QAL9` (`REQ-QAL-027`).
- v1.0 API freeze executed: versions locked, frozen surfaces append-only, artifact
  stamps carry their ABI version + contract fingerprint (`REQ-QAL-023`).
- Pilot deployed (router + MCU + Studio UI) with a stamped deployment identity and
  a clean rollback drill (`REQ-QAL-024`/`REQ-QAL-025`); pilot bundles verified
  (`REQ-QAL-026`).
- Waiver discipline introduced (`REQ-QAL-029`): a red evidence line is only
  cleared by a recorded waiver with a rationale and a review quorum, never
  silently.

## Verification

```text
cargo xtask release --check --json target/release-candidate.json
cargo xtask qualify --check
```

REQ: REQ-QAL-028.
