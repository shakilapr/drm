# Canonical C ABI Surface Algorithms

Scope: the stable C ABI (`drm.h`, crate `drm-ffi`) — opaque context/publisher/consumer handles, raw-pointer `extern "C"` functions with pointer AND length checks before any Rust reference/slice, validate-once endpoint open with borrower-record lease, copy publish plus zero-copy publisher write-loan, borrow/copy read operations, fixed `int32_t` status codes, fail-stop panic rule, one-way ABI compatibility, explicit close operations. Implements architecture.md §Canonical C ABI, §Low-Level C ABI and Endpoint Handles, §Core Language Ecosystem. Owner: E4 (crate `drm-ffi`, header `bindings/c/include/drm/drm.h`) with E1 (core hooks). Phase 7 (finalize), 20–22 (binding substrate). Depends on [channel-metadata.md](../runtime/channel-metadata.md), [channel-publish.md](../runtime/channel-publish.md) (copy + loan transactions), [hazard-protocol.md](../runtime/hazard-protocol.md), [publisher-lifecycle.md](../runtime/publisher-lifecycle.md), [consumer-registration.md](../runtime/consumer-registration.md), [deployment-identity.md](../deployment-identity.md).

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-BIND-abi-borrow-view` | caller-owned `drm_borrow_view` + token; explicit release | `REQ-BIND-001` |
| `REQ-BIND-abi-close` | explicit close; per-handle thread-safety contracts | `REQ-BIND-002` |
| `REQ-BIND-abi-no-borrow-alloc` | borrow acquire/release performs no heap allocation (codebase R5) | `REQ-BIND-003` |
| `REQ-BIND-abi-opaque-handles` | opaque handles incl. `drm_context`; never raw cross-process pointers | `REQ-BIND-004` |
| `REQ-BIND-abi-raw-pointers` | null + length checks precede any reference/slice; length bound before slice | `REQ-BIND-005` |
| `REQ-BIND-abi-single-borrow` | one outstanding borrow per consumer endpoint | `REQ-BIND-006` |
| `REQ-BIND-abi-status-mirror` | one status taxonomy mirrored 1:1 to fixed `int32_t` codes (codebase R4) | `REQ-BIND-007` |
| `REQ-BIND-abi-validation-once` | validation once at endpoint open; zero per-message permission/schema checks | `REQ-BIND-008` |
| `REQ-BIND-abi-versioning` | append-only; never rename; layout change = version bump | `REQ-BIND-009` |
| `REQ-BIND-abi-consumer-lease` | consumer open claims one declared borrower record; exhausted ⇒ `Busy` | `REQ-BIND-010` |
| `REQ-BIND-abi-status-rep` | fixed `int32_t` representation; unknown codes safe (no `repr(C)` enum) | `REQ-BIND-017` |
| `REQ-BIND-abi-compat-one-way` | old application × newer library guaranteed; newer symbols need newer runtime | `REQ-BIND-018` |
| `REQ-BIND-abi-write-loan` | `write_acquire`/`commit`/`abort` writes directly into a shared slot | `REQ-BIND-019` |
| `REQ-BIND-abi-fail-stop` | panic terminates the node process; no in-process recovery; no `Internal` status | `REQ-BIND-020` |
| `REQ-BIND-abi-context` | opaque `drm_context` with explicit open/close lifecycle | `REQ-BIND-021` |

## Data Structures

```c
// bindings/c/include/drm/drm.h — handwritten, single source of truth (ADR 0001)
#include <stdint.h>

typedef int32_t drm_status; // fixed width — no C enum in the ABI, so compiler enum
                            // repr flags (-fshort-enums) cannot alter it (I-A9)

#define DRM_OK                     ((drm_status)0)
#define DRM_ERR_INVALID_ARG        ((drm_status)1)
#define DRM_ERR_NOT_FOUND          ((drm_status)2)
#define DRM_ERR_ALREADY_EXISTS     ((drm_status)3)
#define DRM_ERR_PERMISSION         ((drm_status)4)
#define DRM_ERR_QUEUE_FULL         ((drm_status)5)
#define DRM_ERR_GAP                ((drm_status)6)
#define DRM_ERR_STALE              ((drm_status)7)
#define DRM_ERR_WRONG_ABI          ((drm_status)8)
#define DRM_ERR_POOL_EXHAUSTED     ((drm_status)9)
#define DRM_ERR_RETRY              ((drm_status)10)
#define DRM_ERR_BUSY               ((drm_status)11)
#define DRM_ERR_NO_SAMPLE          ((drm_status)12)
#define DRM_ERR_INVALID_BORROW     ((drm_status)13)
#define DRM_ERR_GENERATION_EXHAUSTED ((drm_status)14)
#define DRM_ERR_INVALID_DATA       ((drm_status)15)
#define DRM_ERR_INVALID_CONFIG     ((drm_status)16)
#define DRM_ERR_NOT_READY          ((drm_status)17)
#define DRM_ERR_LOCK_FAILED        ((drm_status)18)
#define DRM_ERR_SEQUENCE_EXHAUSTED ((drm_status)19)
#define DRM_ERR_EPOCH_EXHAUSTED    ((drm_status)20)
#define DRM_ERR_RING_STAMP_EXHAUSTED ((drm_status)21)
// Append-only: new codes get explicit numbers forever; never renumber, never reuse.
// No DRM_ERR_INTERNAL — fail-stop (I-A6): OS/environment failures map to precise codes above.

typedef struct drm_context drm_context;   // opaque deployment context
typedef struct drm_publisher drm_publisher; // opaque
typedef struct drm_consumer drm_consumer;   // opaque

typedef struct drm_borrow_view { // caller-owned; caller stack/heap, never drm-owned
 const void *data; // points into the consumer's LOCAL mapping; valid until release
 size_t len;
 uint64_t token; // opaque; 0 = invalid
} drm_borrow_view;

typedef struct drm_write_view { // publisher loan; valid until commit/abort
 void *data; // slot bytes in the publisher's LOCAL mapping
 size_t capacity; // acquired capacity <= structural max
 uint64_t token; // opaque; 0 = invalid
} drm_write_view;

#define DRM_ABI_VERSION 0 // bump on layout change only; additions do not bump (I-A8)

_Static_assert(sizeof(drm_status) == 4, "drm_status is int32_t"); // C11
```

```rust
// drm-ffi internal (Rust side, unsafe boundary — the ONLY unsafe besides drm-core).
pub type drm_status = i32; // mirrored 1:1 (I-A4); unknown values surface as
                           // Error::UnknownStatus(code) in bindings — never a
                           // repr(C) fieldless enum (I-A9)

// No per-borrow allocation: the consumer endpoint holds ONE hazard slot and issues one token.
pub struct ConsumerBox { auth: EndpointAuth, core: drm_core::Consumer, outstanding: Option<u64> }

// Publisher core state is lock-free (busy guard); the handle is shared-safe (I-A7).
pub struct PublisherBox { auth: EndpointAuth, core: drm_core::Publisher }
```

## Invariants

- **I-A1 ValidateOnce**: authorization and descriptor validation execute exactly once per endpoint open; steady-state exports perform zero permission or schema/descriptor checks. Test: `ffi_validate_once` (`authorization_checks == 0 ∧ descriptor_checks == 0` over 1e6 steady-state calls).
- **I-A2 PointerChecked**: every exported function checks null pointers, `(data == NULL && len != 0)`, and — **before forming any Rust reference or slice** — validates externally supplied lengths against the endpoint's structural maximum and `isize::MAX`. Every failure path zeroes all out-params first, so an ignored status can never observe stale pointers/tokens. Non-null invalid/dangling pointers remain caller UB, never claimed detectable. Tests: `ffi_null_pointers`, `ffi_len_boundaries` (0, 1, max−1, max, max+1, `isize::MAX`, `SIZE_MAX`, NULL+0; ASan/UBSan), `ffi_out_zeroed`.
- **I-A3 OpaqueSafe**: client-visible handles (context, publisher, consumer, views) are opaque or plain data; no raw cross-process pointer ever appears. Test: header inspection + code review gate.
- **I-A4 StatusMirror**: every Rust `Status` variant has exactly one C code value and vice versa (append-only). Test: `status_map` (exhaustive both directions).
- **I-A5 BorrowDiscipline**: one outstanding borrow per consumer endpoint; a second ⇒ `Busy`; release with a stale/foreign/double token ⇒ `InvalidBorrow`; no heap allocation on acquire/release. Test: `ffi_borrow_discipline`, `ffi_no_borrow_alloc`, `ffi_borrow_busy`.
- **I-A6 FailStopNoUnwind**: `extern "C"` exports never unwind. Expected conditions return `Status`. A Rust panic in `drm-core`/`drm-ffi` is an internal invariant violation: the node process terminates (fail-stop) and normal crash recovery applies (OS locks, epoch, registration-lock reclamation). After `fork()`, inherited DRM objects are invalid in the child (shm-backend I5). No `catch_unwind`; release profile `panic = "abort"`; no `Internal` status — anything safe to return has a precise code. Test: `ffi_panic_failstop` (injected panic in child process ⇒ abort; restarted publisher/consumer recover per publisher-lifecycle.md).
- **I-A7 HandleThreadSafety**: thread-safety is per handle. A publisher endpoint is shared-safe: concurrent `drm_publish_copy`/`drm_write_*` on one publisher produce exactly one in-progress write; losers get `Busy` with no data race and no corrupted slot (core guard, hazard-protocol I-HZ6). A consumer endpoint serves one borrower (one cursor, one outstanding borrow); concurrent borrowers claim distinct records via `drm_consumer_open` (lease, REQ-BIND-010). `drm_context` is shared-safe for concurrent endpoint opens. A close is never concurrent with other calls on the same handle. Tests: `ffi_publish_storm` (TSan; exactly one winner, zero races), `ffi_consumer_lease`, debug misuse assertions.
- **I-A8 OneWayCompat**: symbols are append-only, never renamed; a layout change bumps the ABI major. Older applications always run against newer libraries. Newer applications run against older libraries only when they use no newer symbols (link failure otherwise); node contract fingerprints record the required ABI (deployment-identity.md). Test: `abi_compat` (v0 golden suite × current exports) + symbol-presence lint.
- **I-A9 StatusRep**: C `drm_status` is a fixed `int32_t` with explicit values; language bindings map unknown codes to `UnknownStatus(code)` — an old binding never materializes a future code as a `repr(C)` enum. Tests: `status_rep_static_asserts` (GCC/Clang/MSVC), `status_unknown_code`.
- **I-A10 LoanDiscipline**: `drm_write_acquire` validates the requested capacity against the structural max **before** the publish guard; `write_commit(len)` accepts `len ≤ acquired capacity` (fixed-size topics: `len ==` fixed size) and is the only step that touches ring/generation/`write_seq`; `write_abort` and a failed commit leave visible channel state unchanged (channel-publish I-PB2); a slot acquired but never committed holds junk no sequence references. Tests: `ffi_loan_discipline`, `ffi_loan_abort_invisible`.

## Functions

```rust
// ---- Deployment context (I-A3/I-A21; deployment-identity.md §node_startup) ----
#[no_mangle]
pub unsafe extern "C" fn drm_context_open(
 path: *const c_char, // NULL ⇒ DRM_DEPLOYMENT env var; absent ⇒ NotFound
 out: *mut *mut drm_context) -> drm_status
{
 zero_out(out)?; // I-A2: zeroed before any failure path
 if out.is_null() { return DRM_ERR_INVALID_ARG; }
 let path = copy_cstr(path)?; // owned copy before use; embedded NUL ⇒ InvalidArg
 match drm_core::context_open(&path) { // fingerprint-guard inputs loaded ONCE (I-A1)
 Ok(ctx) => { *out = Box::into_raw(ctx); DRM_OK }
 Err(e) => e.into(),
 }
}
#[no_mangle] pub unsafe extern "C" fn drm_context_close(ctx: *mut drm_context) -> drm_status
{ /* null-check; drop; caller must have closed derived endpoints (I-A7) */ }

// ---- Endpoint open (validation happens ONCE here — I-A1) ----
#[no_mangle]
pub unsafe extern "C" fn drm_publisher_open(
 ctx: *mut drm_context, topic_id: u32, out: *mut *mut drm_publisher) -> drm_status
{
 zero_out(out)?;
 if ctx.is_null() || out.is_null() { return DRM_ERR_INVALID_ARG; } // I-A2: check before deref
 let ctx = &*ctx;
 match validate_open(ctx, topic_id, Direction::Publisher)
  .and_then(drm_core::publisher_open) { // ONCE: manifest auth + fingerprint + descriptor
 Ok(core) => {
  // Box::into_raw is control-plane (once per open), NOT the data path.
  *out = Box::into_raw(Box::new(PublisherBox { auth: ctx.auth(topic_id), core })); DRM_OK
 }
 Err(e) => e.into(),
 }
}

#[no_mangle]
pub unsafe extern "C" fn drm_consumer_open(
 ctx: *mut drm_context, topic_id: u32, out: *mut *mut drm_consumer) -> drm_status
{
 zero_out(out)?;
 if ctx.is_null() || out.is_null() { return DRM_ERR_INVALID_ARG; }
 // validate_open (once) then CLAIM one declared borrower record
 // (consumer-registration.md: OS lock + generation CAS); no vacant record ⇒ Busy.
}

// ---- Publish: copy path (small structured topics) ----
#[no_mangle]
pub unsafe extern "C" fn drm_publish_copy(
 p: *mut drm_publisher, data: *const u8, len: usize) -> drm_status
{
 if p.is_null() || (data.is_null() && len != 0) { return DRM_ERR_INVALID_ARG; } // I-A2
 let p = &*p;
 if len > p.core.max_payload_len() || len > isize::MAX as usize {
  return DRM_ERR_INVALID_ARG; // structural bound BEFORE any slice (I-A2)
 }
 let payload = if len == 0 { &[][..] } else { slice::from_raw_parts(data, len) };
 p.core.publish_copy(payload).into() // auth captured at open (I-A1); channel-publish.md copy form
}

// ---- Publish: zero-copy loan path (bulk direct-write; channel-publish.md loan form) ----
#[no_mangle]
pub unsafe extern "C" fn drm_write_acquire(
 p: *mut drm_publisher, requested: usize, out: *mut drm_write_view) -> drm_status
{
 zero_out(out)?;
 // requested > structural max ⇒ InvalidArg BEFORE guard (I-A2/I-A10);
 // then guard CAS → Busy loser, seq/overflow checks, slot select, generation reserve.
}

#[no_mangle]
pub unsafe extern "C" fn drm_write_commit(
 p: *mut drm_publisher, token: u64, len: usize) -> drm_status
{
 // token lookup; len <= acquired capacity else InvalidArg (I-A10);
 // length header + ring preflight/commit + generation store + write_seq release (channel-publish.md).
}

#[no_mangle]
pub unsafe extern "C" fn drm_write_abort(p: *mut drm_publisher, token: u64) -> drm_status
{ /* guard drop only; no visible effect (I-A10) */ }

// ---- Read: transient borrow (one outstanding per endpoint — I-A5) ----
#[no_mangle]
pub unsafe extern "C" fn drm_borrow_latest(
 c: *mut drm_consumer, out: *mut drm_borrow_view) -> drm_status
{
 zero_out(out)?;
 if c.is_null() || out.is_null() { return DRM_ERR_INVALID_ARG; }
 match (*c).borrow_latest() { // one outstanding (I-A5); hazard-protocol.md
 Ok(v) => { (*out).data = v.ptr; (*out).len = v.len; (*out).token = v.token; DRM_OK }
 Err(e) => e.into(), // *out stays zeroed (I-A2)
 }
}

#[no_mangle]
pub unsafe extern "C" fn drm_borrow_next(
 c: *mut drm_consumer, out: *mut drm_borrow_view, out_missed: *mut u64) -> drm_status
{
 zero_out(out)?; zero_out(out_missed)?;
 // cursor advance (cursor-delivery.md); lapped ⇒ DRM_ERR_GAP with exact *out_missed (I-C2);
 // *out zeroed — GAP produces no view.
}

// ---- Read: copy into caller-owned memory (cursor-delivery.md I-C9: core never allocates) ----
#[no_mangle]
pub unsafe extern "C" fn drm_copy_latest(
 c: *mut drm_consumer, buf: *mut u8, cap: usize, out_len: *mut usize) -> drm_status
{
 zero_out(out_len)?;
 // acquire → copy into buf → release hazard immediately; cap < required ⇒ InvalidArg with
 // *out_len = required, no partial copy; empty channel ⇒ DRM_ERR_NO_SAMPLE.
}

#[no_mangle]
pub unsafe extern "C" fn drm_copy_next(
 c: *mut drm_consumer, buf: *mut u8, cap: usize,
 out_len: *mut usize, out_missed: *mut u64) -> drm_status
{ /* copy form of drm_borrow_next; GAP ⇒ DRM_ERR_GAP + exact *out_missed, no copy */ }

// ---- Release / close / version ----
#[no_mangle]
pub unsafe extern "C" fn drm_borrow_release(c: *mut drm_consumer, token: u64) -> drm_status
{
 if c.is_null() || token == 0 { return DRM_ERR_INVALID_ARG; } // I-A2
 match (*c).release_token(token) { // stale/foreign/double ⇒ InvalidBorrow
 Ok(()) => DRM_OK,
 Err(e) => e.into(),
 }
}
#[no_mangle] pub unsafe extern "C" fn drm_publisher_close(p: *mut drm_publisher) -> drm_status
{ /* null-check; outstanding loan ⇒ Busy; drop */ }
#[no_mangle] pub unsafe extern "C" fn drm_consumer_close(c: *mut drm_consumer) -> drm_status
{ /* null-check; outstanding borrow ⇒ Busy (consumer-registration I-CR4); drop after release */ }
#[no_mangle] pub extern "C" fn drm_abi_version() -> u32 { DRM_ABI_VERSION }
```

Every export shares the same boundary rules: null/length checks first (I-A2), status-only errors, no `catch_unwind` (I-A6) — there is no `ffi_catch` wrapper to forget.

## Protocols

*Deployment context lifecycle:*

1. `drm run` (or the platform launcher) starts the node with the deployment directory in `DRM_DEPLOYMENT`.
2. `drm_context_open(NULL | path)` loads `deployment.json`, validates fingerprint-guard inputs once (deployment-identity.md §node_startup), returns the opaque context.
3. The context is shared-safe for concurrent endpoint opens; `drm_context_close` after all derived endpoints are closed.

*Open-and-validate-once (publisher):*

1. C node calls `drm_publisher_open(ctx, topic_id, &h)`.
2. FFI checks pointers (I-A2), then runs manifest authorization, fingerprint guard, and descriptor validation (channel-metadata §validate_on_attach) — **once**.
3. Authorized handle returned; steady-state `drm_publish_copy` is auth-free (zero per-message overhead).
4. Undeclared topic at open ⇒ `Permission` (architectural correctness, not security sandbox).

*Consumer lease (per-borrower records):*

1. Activation reserves `borrowers` records (consumer-registration.md) for each declared zero-copy consume endpoint.
2. Each borrower thread calls `drm_consumer_open`; it claims one record (OS lock + generation CAS).
3. No vacant record ⇒ `Busy`; the thread uses a copy path or waits for a sibling endpoint to close.

*Borrow scope (C discipline):*

1. `drm_borrow_latest(c, &view)` → `view.data`/`view.len`/`view.token`; a second call before release ⇒ `Busy`.
2. MUST call `drm_borrow_release(c, view.token)` before the iteration ends; external I/O requires `drm_copy_*` first (architecture borrow-lifecycle rule).
3. Debug builds: a leaked outstanding token at close is reported.

*Publisher write-loan (bulk zero-copy):*

1. `drm_write_acquire(p, cap)` — capacity checked against the structural max before the guard (I-A10); then channel-publish.md loan steps: guard CAS (`Busy` loser), sequence/overflow checks, slot select, generation reserve. Returns `drm_write_view` into the publisher's local mapping.
2. Application writes payload bytes directly into `view.data` (no DRM copy).
3. `drm_write_commit(p, token, len)` finalizes: length header (variable topics), ring preflight/commit, generation store, `write_seq` release.
4. `drm_write_abort` on any failure path. A crash before commit leaves the slot unreachable; restart order applies (publisher-lifecycle.md I-L7).

*Thread-safety summary (I-A7):*

| Handle | Concurrent use |
|---|---|
| `drm_context` | concurrent endpoint opens safe; close after endpoints closed |
| `drm_publisher` | `drm_publish_copy`/`drm_write_*` concurrent-safe; exactly one in-progress write, losers `Busy`; close not concurrent |
| `drm_consumer` | single-borrower only; lease additional records for additional threads |

## Edge Cases & Errors

| Case | Result |
|---|---|
| Null context/handle/out pointer | `InvalidArg` before deref; out-params zeroed (I-A2) |
| Non-null invalid/dangling pointer | caller contract violation; behavior is undefined by the C API, never claimed detectable |
| `data == NULL && len != 0` | `InvalidArg` before slice |
| `len > structural max` or `len > isize::MAX` | `InvalidArg` before any Rust reference/slice exists (I-A2) |
| Undeclared topic at open | `Permission` |
| Wrong-ABI binary vs deployment | `WrongAbi` at open (fingerprint guard) |
| No vacant borrower record at consumer open | `Busy` (I-A7 lease) |
| Concurrent publish on one publisher | exactly one proceeds; others `DRM_ERR_BUSY`, no race, no corruption (I-A7 / I-HZ6) |
| second borrow before release | `DRM_ERR_BUSY` (I-A5) |
| borrow of empty channel | `DRM_ERR_NO_SAMPLE` |
| copy with `cap < required` | `InvalidArg`, `*out_len = required`, no partial copy (I-C9) |
| lapped sequential read | `DRM_ERR_GAP` + exact `*out_missed` (I-C2); view/copy skipped |
| stale/foreign/double release token | `DRM_ERR_INVALID_BORROW` (I-A5) |
| `write_commit` with `len > acquired capacity` | `InvalidArg`; nothing committed (I-A10) |
| `write_abort` after acquire | `OK`; visible channel state unchanged (I-A10) |
| generation/sequence/ring-stamp exhausted | specific counter-exhaustion status before payload write (channel-publish.md) |
| close consumer with outstanding borrow | `DRM_ERR_BUSY`; record stays claimed (`Closing`) until release (consumer-registration I-CR4) |
| close publisher with outstanding loan | `DRM_ERR_BUSY`; commit or abort first |
| new-header binary × older runtime library | link error iff it uses newer symbols (documented; I-A8) |
| Rust panic inside `drm-core`/`drm-ffi` | node process terminates (I-A6); crash recovery per publisher-lifecycle.md |

## Test Matrix

| Case | Expected |
|---|---|
| `ffi_validate_once` | `authorization_checks == 0 ∧ descriptor_checks == 0` over 1e6 steady-state calls (I-A1) |
| `ffi_null_pointers` + `ffi_len_boundaries` | I-A2 (ASan/UBSan clean) |
| `ffi_out_zeroed` | every failure path leaves zeroed out-params |
| `status_map` | I-A4 exhaustive both directions |
| `status_rep_static_asserts` / `status_unknown_code` | I-A9 (GCC/Clang/MSVC; unknown ⇒ `UnknownStatus`) |
| `ffi_borrow_discipline` / `ffi_borrow_busy` / `ffi_no_borrow_alloc` | I-A5: no alloc, token discipline |
| `ffi_loan_discipline` / `ffi_loan_abort_invisible` | I-A10 |
| `ffi_panic_failstop` (child-process injected panic) | I-A6: abort; restart recovery clean |
| `ffi_publish_storm` (N threads × one publisher, TSan) | I-A7: one winner, losers `Busy`, zero races |
| `ffi_consumer_lease` (claim N, N+1th) | I-A7: Nth+1 ⇒ `Busy` |
| `ffi_parity` (C↔Rust↔Python) | identical bytes both directions |
| `abi_spike` (gcc/clang/clang-cl, c99/c11) | compiles clean |
| `abi_compat` (v0 golden suite × current exports) | I-A8 one-way |
