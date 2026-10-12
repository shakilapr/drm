# Public API Inventory — Freeze Prep

Derived component spec (codebase.md §2.4; non-normative — the algorithm docs
[qualification.md](../algorithms/program/qualification.md) and
[c-abi-surface.md](../algorithms/bindings/c-abi-surface.md) win conflicts).
`REQ-QAL-021`; release-wave freeze prep. The machine form is
[xtask/src/inventory.rs](../../xtask/src/inventory.rs): every row below is
re-resolved against its owner by `cargo xtask qualify`, so a renamed or removed
symbol fails the gate (`API_INVENTORY = resolved/total`). Freeze is executed in
M24W2 (`API_FROZEN = 1`); removal happens only at a major or ABI-version bump
(codebase.md §6.4).

## Freeze candidates (v1.0 block)

Append-only and never renamed (I-A8); a layout change bumps `DRM_ABI_VERSION`.

### Canonical C ABI exports (`drm.h`)

| Symbol | REQ |
|---|---|
| `drm_abi_version` | `REQ-BIND-018` |
| `drm_context_open` | `REQ-BIND-021` |
| `drm_context_close` | `REQ-BIND-002` |
| `drm_publisher_open` | `REQ-BIND-008` |
| `drm_publisher_close` | `REQ-BIND-002` |
| `drm_consumer_open` | `REQ-BIND-010` |
| `drm_consumer_close` | `REQ-BIND-002` |
| `drm_publish_copy` | `REQ-BIND-006` |
| `drm_read_latest` | `REQ-BIND-009` |
| `drm_read_next` | `REQ-BIND-019` |
| `drm_borrow_latest` | `REQ-BIND-001` |
| `drm_borrow_release` | `REQ-BIND-001` |

### Opaque handles (`drm.h`)

| Symbol | REQ |
|---|---|
| `drm_context` | `REQ-BIND-004` |
| `drm_publisher` | `REQ-BIND-004` |
| `drm_consumer` | `REQ-BIND-004` |

### Status taxonomy (`drm.h`, fixed `int32_t`, append-only)

| Symbol | REQ |
|---|---|
| `DRM_OK` | `REQ-BIND-007` |
| `DRM_ERR_INVALID_ARG` | `REQ-BIND-007` |
| `DRM_ERR_NOT_FOUND` | `REQ-BIND-007` |
| `DRM_ERR_ALREADY_EXISTS` | `REQ-BIND-007` |
| `DRM_ERR_PERMISSION` | `REQ-BIND-007` |
| `DRM_ERR_QUEUE_FULL` | `REQ-BIND-007` |
| `DRM_ERR_GAP` | `REQ-BIND-007` |
| `DRM_ERR_STALE` | `REQ-BIND-007` |
| `DRM_ERR_WRONG_ABI` | `REQ-BIND-017` |
| `DRM_ERR_POOL_EXHAUSTED` | `REQ-BIND-007` |
| `DRM_ERR_RETRY` | `REQ-BIND-007` |
| `DRM_ERR_BUSY` | `REQ-BIND-007` |
| `DRM_ERR_NO_SAMPLE` | `REQ-BIND-007` |
| `DRM_ERR_INVALID_BORROW` | `REQ-BIND-007` |
| `DRM_ERR_GENERATION_EXHAUSTED` | `REQ-BIND-007` |
| `DRM_ERR_INVALID_DATA` | `REQ-BIND-007` |
| `DRM_ERR_INVALID_CONFIG` | `REQ-BIND-007` |
| `DRM_ERR_NOT_READY` | `REQ-BIND-007` |
| `DRM_ERR_LOCK_FAILED` | `REQ-BIND-007` |
| `DRM_ERR_SEQUENCE_EXHAUSTED` | `REQ-BIND-007` |
| `DRM_ERR_EPOCH_EXHAUSTED` | `REQ-BIND-007` |
| `DRM_ERR_RING_STAMP_EXHAUSTED` | `REQ-BIND-007` |
| `DRM_ERR_NOT_IMPLEMENTED` | `REQ-BIND-017` |

### Public Rust items

| Item | Owner | REQ |
|---|---|---|
| `pub enum Status` | `crates/drm-types/src/lib.rs` | `REQ-CORE-024` |
| `pub struct Channel` | `crates/drm-core/src/channel/mod.rs` | `REQ-CORE-021` |
| `pub struct HistoryRing` | `crates/drm-core/src/ring/mod.rs` | `REQ-CORE-066` |
| `pub enum OverflowPolicy` | `crates/drm-core/src/channel/overflow.rs` | `REQ-CORE-039` |
| `pub fn acquire` | `crates/drm-core/src/hazard/mod.rs` | `REQ-CORE-030` |
| `pub trait TransportAdapter` | `crates/drm-router/src/transport/mod.rs` | `REQ-ROUTE-009` |
| `pub struct WorkspaceModel` | `crates/drm-workspace/src/model/mod.rs` | `REQ-TOOL-003` |

## Deferred (post-v1.0)

Specified but not implemented; **excluded from the freeze set**. Owner:
[c-abi-surface.md](../algorithms/bindings/c-abi-surface.md).

| Symbol | REQ | Note |
|---|---|---|
| `drm_write_acquire` | `REQ-BIND-019` | zero-copy write loan (bulk direct-write) |
| `drm_write_commit` | `REQ-BIND-019` | loan commit |
| `drm_write_abort` | `REQ-BIND-019` | loan abort |
| `drm_copy_latest` | `REQ-BIND-009` | copy-into-caller read |
| `drm_copy_next` | `REQ-BIND-019` | sequential copy read |
| `drm_borrow_next` | `REQ-BIND-001` | sequential borrow read |
| `drm_log` | `REQ-BIND-023` | best-effort app-log export |

REQ: REQ-QAL-021.
