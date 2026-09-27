# Canonical C ABI Surface Algorithms

Scope: the stable C ABI (`drm.h`, crate `drm-ffi`) — raw-pointer `extern "C"` functions with pointer checks before any Rust reference/slice, endpoint handles opened once with authorization, zero per-message overhead, a caller-owned borrow view/token with no per-borrow allocation, explicit close operations, status taxonomy mirroring, ABI versioning. Implements architecture.md §Canonical C ABI, §Low-Level C ABI and Endpoint Handles, §Core Language Ecosystem. Owner: E4 (crate `drm-ffi`, header `bindings/c/include/drm/drm.h`) with E1 (core hooks). Phase 7 (finalize), 20–22 (binding substrate). Depends on [channel-metadata.md](../runtime/channel-metadata.md), [hazard-protocol.md](../runtime/hazard-protocol.md), [publisher-lifecycle.md](../runtime/publisher-lifecycle.md), [deployment-identity.md](../deployment-identity.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-BIND-abi-raw-pointers` | `extern "C"` raw pointers; checks precede any reference/slice | — |
| `REQ-BIND-abi-validation-once` | access validation once at endpoint open; zero per-message checks | — |
| `REQ-BIND-abi-opaque-handles` | opaque handles; never raw cross-process pointers | — |
| `REQ-BIND-abi-status-mirror` | one status taxonomy mirrored 1:1 to C codes (codebase R4) | — |
| `REQ-BIND-abi-borrow-view` | caller-owned `drm_borrow_view` + token; explicit release | — |
| `REQ-BIND-abi-single-borrow` | one outstanding borrow per consumer endpoint | — |
| `REQ-BIND-abi-no-borrow-alloc` | borrow acquire/release performs no heap allocation (codebase R5) | — |
| `REQ-BIND-abi-close` | explicit publisher/consumer close; documented thread-safety | — |
| `REQ-BIND-abi-versioning` | append-only; never rename; layout change = version bump | — |

## Data Structures

```c
// bindings/c/include/drm/drm.h — handwritten, single source of truth (ADR 0001)
typedef struct drm_publisher drm_publisher; // opaque
typedef struct drm_consumer drm_consumer;

typedef struct drm_borrow_view { // caller-owned; caller stack/heap, never drm-owned
 const void *data; // points into the consumer's LOCAL mapping; valid until release
 size_t len;
 uint64_t token; // opaque; 0 = invalid
} drm_borrow_view;

#define DRM_ABI_VERSION 0 // bump on layout change only; additions do not bump

typedef enum { // mirrors drm-types::Status 1:1 (append-only)
 DRM_OK = 0, DRM_ERR_INVALID_ARG, DRM_ERR_NOT_FOUND, DRM_ERR_ALREADY_EXISTS,
 DRM_ERR_PERMISSION, DRM_ERR_QUEUE_FULL, DRM_ERR_GAP, DRM_ERR_STALE,
 DRM_ERR_WRONG_ABI, DRM_ERR_POOL_EXHAUSTED, DRM_ERR_RETRY, DRM_ERR_BUSY,
 DRM_ERR_NO_SAMPLE, DRM_ERR_INVALID_BORROW, DRM_ERR_GENERATION_EXHAUSTED,
 DRM_ERR_INTERNAL,
} drm_status;
```

```rust
// drm-ffi internal (Rust side, unsafe boundary — the ONLY unsafe besides drm-core).
// No per-borrow allocation: the consumer endpoint holds ONE hazard slot and issues one token.
pub struct ConsumerBox { auth: EndpointAuth, core: drm_core::Consumer, outstanding: Option<u64> }
```

## Invariants

- **I-A1 ValidateOnce**: authorization and descriptor validation execute exactly once per endpoint open; `drm_publish`/`drm_read_*` perform zero permission or schema checks. Test: `ffi_validate_once` (counter).
- **I-A2 PointerChecked**: every exported function checks each raw pointer (and `data.is_null() && len != 0`) **before** forming a Rust reference or slice; a null/foreign pointer yields `InvalidArg`, never UB. Test: `ffi_null_pointers`, `ffi_foreign_handles` (ASan/UBSan).
- **I-A3 OpaqueSafe**: client-visible handles are opaque; no raw cross-process pointer ever appears. Test: header inspection + code review gate.
- **I-A4 StatusMirror**: every Rust `Status` variant has exactly one C code and vice versa (append-only). Test: `status_map` (exhaustive both directions).
- **I-A5 BorrowDiscipline**: one outstanding borrow per consumer endpoint; a second ⇒ `Busy`; release with a stale/foreign/double token ⇒ `InvalidBorrow`; no heap allocation on acquire/release. Test: `ffi_borrow_discipline`, `ffi_no_borrow_alloc`, `ffi_borrow_busy`.
- **I-A6 NoUnwind**: no Rust panic crosses the FFI boundary (catch_unwind shield ⇒ `DRM_ERR_INTERNAL`). Test: `ffi_no_unwind` (injected panic fixture).
- **I-A7 ThreadSafety**: endpoints are not thread-safe by default; a caller must not use one endpoint concurrently from multiple threads (each concurrent borrower opens its own consumer). Test: documented contract + `ffi_thread_contract` (debug assertion on concurrent misuse).

## Functions

```rust
#[no_mangle]
pub unsafe extern "C" fn drm_publisher_open(
 ctx: *const DeployContext, topic_id: u32, out: *mut *mut drm_publisher) -> drm_status
{
 if ctx.is_null() || out.is_null() { return DRM_ERR_INVALID_ARG; } // I-A2: check before deref
 let ctx = &*ctx;
 match validate_open(ctx, topic_id, Direction::Publisher)
 .and_then(drm_core::publisher_open) { // ONCE: guard + descriptor validation
 Ok(core) => {
 // Box::into_raw is control-plane (once per open), NOT the data path.
 *out = Box::into_raw(Box::new(PublisherBox { auth: ctx.auth(topic_id), core })); DRM_OK
 }
 Err(e) => e.into(),
 }
}

#[no_mangle]
pub unsafe extern "C" fn drm_publish(
 p: *mut drm_publisher, data: *const u8, len: usize) -> drm_status
{
 if p.is_null() { return DRM_ERR_INVALID_ARG; } // I-A2
 if data.is_null() && len != 0 { return DRM_ERR_INVALID_ARG; }
 let payload = if len == 0 { &[][..] } else { slice::from_raw_parts(data, len) };
 (*p).core.publish(payload).into() // auth captured at open (I-A1)
}

#[no_mangle]
pub unsafe extern "C" fn drm_borrow_latest(
 c: *mut drm_consumer, out: *mut drm_borrow_view) -> drm_status
{
 if c.is_null() || out.is_null() { return DRM_ERR_INVALID_ARG; } // I-A2
 match (*c).borrow_latest() { // one outstanding (I-A5); hazard-protocol.md
 Ok(v) => { (*out).data = v.ptr; (*out).len = v.len; (*out).token = v.token; DRM_OK }
 Err(e) => e.into(),
 }
}

#[no_mangle]
pub unsafe extern "C" fn drm_borrow_release(
 c: *mut drm_consumer, token: u64) -> drm_status
{
 if c.is_null() || token == 0 { return DRM_ERR_INVALID_ARG; } // I-A2
 match (*c).release_token(token) { // stale/foreign/double ⇒ InvalidBorrow
 Ok(()) => DRM_OK,
 Err(Status::InvalidBorrow) => DRM_ERR_INVALID_BORROW,
 Err(e) => e.into(),
 }
}

#[no_mangle] pub unsafe extern "C" fn drm_publisher_close(p: *mut drm_publisher) -> drm_status { /* null-check; drop */ }
#[no_mangle] pub unsafe extern "C" fn drm_consumer_close(c: *mut drm_consumer) -> drm_status { /* null-check; drop */ }
#[no_mangle] pub extern "C" fn drm_abi_version() -> u32 { DRM_ABI_VERSION }
```

## Protocols

*Open-and-validate-once (publisher):*

1. C node calls `drm_publisher_open(deploy_ctx, topic_id, &h)`.
2. FFI null-checks pointers first (I-A2), then checks manifest authorization, fingerprint guard (deployment-identity §node_startup), and descriptor validation (channel-metadata §validate_on_attach) — **once**.
3. Authorized handle returned; steady-state `drm_publish` is auth-free (zero per-message overhead).
4. Undeclared topic at open ⇒ `Permission` (architectural correctness, not security sandbox).

*Borrow scope (C discipline):*

1. `drm_borrow_latest(&c, &view)` → `view.data`/`view.len`/`view.token`; a second call before release ⇒ `Busy`.
2. MUST call `drm_borrow_release(c, view.token)` before the iteration ends; external I/O requires `drm_read_*` copy first (architecture borrow-lifecycle rule).
3. Debug builds: a leaked outstanding token at close is reported.

## Edge Cases & Errors

| Case | Result |
|---|---|
| Null context/handle/out pointer | `InvalidArg` before deref (I-A2) |
| `data == NULL && len != 0` | `InvalidArg` before slice |
| Undeclared topic at open | `Permission` |
| Wrong-ABI binary vs deployment | `WrongAbi` at open (guard) |
| publish size > structural max | `InvalidArg` (pool check pre-write) |
| concurrent publish on one topic | `DRM_ERR_BUSY` (hazard-protocol I-HZ6) |
| second borrow before release | `DRM_ERR_BUSY` (I-A5) |
| borrow of empty channel | `DRM_ERR_NO_SAMPLE` |
| stale/foreign/double release token | `DRM_ERR_INVALID_BORROW` (I-A5) |
| generation exhausted | `DRM_ERR_GENERATION_EXHAUSTED` |
| gap on sequential copy read | `DRM_ERR_GAP` + `out_gap_missed` accessor |
| Rust panic inside core (bug) | catch_unwind ⇒ `DRM_ERR_INTERNAL`, never unwind across FFI (I-A6) |
| close with outstanding borrow | `InvalidBorrow`; handle not freed until released |

## Test Matrix

| Case | Expected |
|---|---|
| `ffi_validate_once` | 0 per-message checks over 1e6 calls |
| `ffi_null_pointers` / `ffi_foreign_handles` | I-A2 (ASan/UBSan clean) |
| `status_map` | I-A4 exhaustive |
| `ffi_borrow_discipline` / `ffi_borrow_busy` / `ffi_no_borrow_alloc` | I-A5: no alloc, token discipline |
| `ffi_no_unwind` | I-A6 |
| `ffi_thread_contract` | I-A7 |
| `ffi_parity` (C↔Rust↔Python) | identical bytes both directions |
| `abi_spike` (gcc/clang/clang-cl, c99/c11) | compiles clean |
| `abi_compat` (old header × new lib, new header × old lib) | Phase 7 gate suite |
