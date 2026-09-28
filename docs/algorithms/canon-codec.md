# Canonical Codec Algorithms

Scope: canonical binary representation — primitive encode/decode over raw bytes, two-region layout computation (fixed header in declaration order + bounded variable tail), fixed/bounded array and enum codecs, checked fixed-width size arithmetic, message-level structural-bound enforcement (two-phase validate → write), UTF-8/enum validation, and deterministic observable bytes. Implements architecture.md §Canonical Binary Layout, §Structural Bounds, §Sample Metadata. Owners: E2 (`drm-workspace::canon` — layout) + E1 (`drm-core::canon` — codec over mapped bytes). Phases 3–4; the layout-fixture exchange is the permanent toolchain↔runtime gate.

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-TOOL-layout-le` | §Canonical Binary Layout — consistent little-endian | `REQ-TOOL-043` |
| `REQ-TOOL-layout-standard-sizes` | standard sizes (4-byte int, 4-byte IEEE float) | `REQ-TOOL-044` |
| `REQ-TOOL-layout-declaration-order` | §Canonical Binary Layout — fixed fields in declaration order, natural alignment, total padded; variable bodies follow in declaration order (two-region layout) | `REQ-TOOL-042` |
| `REQ-TOOL-bounds-structural` | §Structural Bounds — `string[32]`, `f32[16]`, `f32[<=16]` | `REQ-TOOL-001` |
| `REQ-TOOL-bounds-unbounded-reject` | unbounded data-plane structures rejected | `REQ-TOOL-002` |
| `REQ-TOOL-layout-bounded-align` | bounded array data area aligned to element alignment | `REQ-TOOL-040` |
| `REQ-TOOL-layout-checked` | all size/offset math checked; overflow ⇒ error | `REQ-TOOL-041` |
| `REQ-TOOL-observable-bytes` | observable canonical bytes deterministic; out-of-sample bytes inaccessible | `REQ-TOOL-051` |
| `REQ-CORE-codec-no-alloc` | codebase R5 — codec path zero allocation | `REQ-CORE-082` |
| `REQ-CORE-codec-bounds-before-write` | Phase 13 — reject beyond structural max before any write | `REQ-CORE-081` |
| `REQ-CORE-codec-no-ub` | arbitrary bytes decode to value or error, never UB | `REQ-CORE-083` |

## Data Structures

| Type | Size | Align | Canonical rule |
|---|---|---|---|
| `u8/i8/bool` | 1 | 1 | `bool` = byte 0/1; others ⇒ `InvalidData` on decode |
| `u16/i16` | 2 | 2 | LE |
| `u32/i32/f32` | 4 | 4 | LE; IEEE-754 bit pattern preserved (no NaN canonicalization) |
| `u64/i64/f64` | 8 | 8 | LE |
| `T[N]` | `N·size(T)` | `align(T)` | contiguous, row-major, no inter-element padding |
| `string[N]` | 4 (+4 offset) | 4 | **variable member**: `u32` byte-length descriptor [+ `u32` offset when > 1 variable member]; body = ≤ N UTF-8 bytes in the tail |
| `bytes[<=N]` | 4 (+4 offset) | 4 | **variable member**: `u32` length descriptor [+ `u32` offset]; body = ≤ N raw bytes in the tail |
| `T[<=N]` | 4 (+4 offset) | 4 | **variable member**: `u32` count descriptor [+ `u32` offset]; body = count × aligned elements in the tail |
| `enum{…}` | 1 | 1 | `u8` tag = declaration index; unknown tag ⇒ `InvalidData`; ≤ 256 variants (schema-parser `enum-too-large`) |

The bounded-array data area must be element-aligned: the body offset is element-aligned (two-region layout below), so a `u64`/`f64` body never lands at a misaligned address.

### Fixed Header + Bounded Tail (variable members)

Structs with ≥ 1 variable member (`string[N]`, `bytes[<=N]`, `T[<=N]`) use a two-region layout:

```text
FIXED HEADER                            VARIABLE TAIL
  fixed fields (declaration order,        bodies (declaration order,
  natural alignment, total padding)       contiguous, element-aligned)
  per-variable descriptor block:
    count : u32
    offset: u32  (only when > 1 variable member)
--------------------------------------------------------- ← sample_len
```

- **Pure-fixed structs**: single region, unchanged — declaration order, natural alignment, total padding. Fixed-size topics never pay any length field (architecture §Sample Metadata).
- **Exactly one variable member** ⇒ offset is implicit (body starts at `align_up(header_end, elem_align)`); the descriptor degenerates to the `u32` length header of a bulk topic (architecture: "prepend an actual byte length before the payload").
- **`offset` is absolute** (from sample start): element-aligned, `≥ header_end`, so aligned zero-copy access needs no relocation.
- **`sample_len`** (committed at publish, channel-publish.md) = `header_end + Σ aligned body bytes`. Slot capacity (max header + max tail) is fixed at activation.
- **Bytes at `≥ sample_len` are outside the current sample**: never observable, never transmitted, never required to be cleared. Stale tail bytes are unreachable by construction — no `prev_len`, no shrink memset, no `..capacity` scrubbing.
- **Decoder validation per descriptor**: `count ≤ declared_max` ∧ `offset ≡ 0 (mod elem_align)` ∧ `offset ≥ header_end` ∧ `offset + count·elem_size ≤ sample_len` ∧ body ranges pairwise disjoint ∧ declaration-ordered. Any violation ⇒ `InvalidData` (I5: value or error, never UB).

```rust
// Canonical arithmetic domain is FIXED-WIDTH u64 — never usize (C1: usize is pointer-width;
// on 32-bit targets `as usize` silently truncates before any checked op can fire).
pub struct CanonLayout { size: u64, align: u32 }
pub struct CanonOffset(u64);
pub struct CanonLen(u64);
// Target mapping happens exactly once, at the use site:
//   usize::try_from(layout.size).map_err(|_| InvalidConfig)?
// A 32-bit target rejects samples it cannot address; the canonical calculation never truncates.
// MVP caps (counts/lengths are u32 on the wire): max bounded count/length ≤ u32::MAX;
// canonical sample size ≤ u32::MAX (channel-metadata enforces the same bound at attach).

fn layout(t: &Type) -> Result<CanonLayout, Status> {
 Ok(match t {
  Prim(p) => prim_layout(p),
  Enum { .. } => CanonLayout { size: 1, align: 1 },
  String { max } | Bytes { max } => CanonLayout { size: 4, align: 4 }, // descriptor; body lives in the tail
  FixedArray { elem, n } => {
  let e = layout(&elem.as_type())?;
  CanonLayout { size: e.size.checked_mul(u64::from(*n)).ok_or(SizeOverflow)?, align: e.align }
  }
  Bounded { .. } => CanonLayout { size: 4, align: 4 }, // descriptor (+4 offset when > 1 variable member);
                                                       // layout_struct re-derives the element layout for tail packing
 })
}

fn layout_struct(fields: &[Field]) -> Result<StructLayout, Status> {
 // Region 1 (header): fixed fields in declaration order, natural alignment, total padded;
 //   then the descriptor block: one u32 count per variable member (+ u32 offset each
 //   when the struct has > 1 variable member), in declaration order.
 // Region 2 (tail): bodies in declaration order; body_k offset = running cursor
 //   aligned up to the element alignment (absolute offsets, CanonOffset).
 // All arithmetic in u64 checked_add/checked_mul/checked align_up (I7).
 // Final: sample_max = header_end + Σ max-aligned bodies; > u32::MAX ⇒ SizeOverflow (MVP cap).
 // struct_align = max align over header fields; layout_hash covers the two-region render
 //   (field offsets + descriptor offsets + tail packing rules).
}
```

## Invariants

- **I1 GoldenIdentity**: for every `tests/fixtures/golden/` vector: Rust encode == C decode == Python(ctypes) decode, byte-identical. Tests: `golden_primitives`, `golden_composites`.
- **I2 LEBytes**: canonical bytes are little-endian on every host. The proof is the hand-authored byte oracle (`u32 0x12345678` ⇒ `78 56 34 12`, I9) — not an assertion about the build host. MVP targets are little-endian (x86-64, aarch64); big-endian targets are out of MVP scope. The explicit LE assertion on x86_64 + aarch64 is a regression guard only, not a portability proof. Tests: `canon_le_assert`, `golden_independent`.
- **I3 BoundsBeforeWrite**: payload exceeding any structural bound ⇒ `InvalidArg`/`InvalidData` **before any destination byte is written** — for fields AND whole messages. Composite encoding is two-phase: `validate_x(value) → Plan` (all bounded lengths, UTF-8, enum tags, nested fields, descriptor arithmetic, final `sample_len`; allocation-free — the schema is compile-time known), then `write_x(dst, value, plan)`. No destination byte changes until whole-sample validation succeeds. Tests: `boundary_matrix` (max+1 rows), `no_partial_write` (late-field overflow leaves every destination byte untouched — 0xAA probe).
- **I4 NoAlloc**: steady-state encode/decode zero heap allocation. Test: `alloc` (counting GlobalAlloc).
- **I5 NoUB**: arbitrary input bytes decode to value or `InvalidData` — never UB/panic. Tests: read-fuzz, Miri (safe modules), fault-injection corrupted fixture.
- **I6 LayoutAgreement**: `drm-core` tests consume `layout.json` emitted by `drm-workspace` — zero mismatches. Test: `crosscheck` (permanent CI gate).
- **I7 CheckedArithmetic**: all size/offset/align arithmetic runs in fixed-width `u64` (`CanonSize`/`CanonOffset`) with `checked_add`/`checked_mul`/checked align_up; any overflow ⇒ error (`SizeOverflow`/`InvalidConfig`), never a wrapped small value. `usize` appears only at the target boundary via `try_from` (a 32-bit target rejects what it cannot address). Test: `codec_overflow` (huge `N`, near-`u64::MAX` offsets, 32-bit target rejection).
- **I8 ObservableBytes**: the current canonical sample is exactly `[0, sample_len)`; every byte in it is deterministic. Bytes at `≥ sample_len` (unused tail capacity) are outside the sample — inaccessible, never transmitted, never required to be cleared. Test: `observable_bytes` (shrink a body, republish: no stale byte addressable, descriptors validate, tail never scrubbed).
- **I9 IndependentVectors**: a subset of golden vectors is hand-authored directly from this specification (not produced by the implementation under test). Test: `golden_independent`.
- **I10 Utf8Strict**: `string[N]` — N counts UTF-8 **bytes**, never characters. Encode: value must be valid UTF-8 with byte length ≤ N ⇒ else `InvalidArg`. Decode: declared length ≤ N, range inside the sample, bytes valid UTF-8 ⇒ else `InvalidData` (never UB). Malformed sequences rejected on both paths: `C0 AF`, `E0 80 80`, `ED A0 80`, `F5 80 80 80`, truncated multibyte. Test: `string_utf8_matrix`.

## Functions

```rust
// ---- drm-core::canon (runtime; mapped/shared bytes) ----
fn write_f32(buf: &mut [u8], off: usize, v: f32) -> Result<(), Status> {
 check_bounds(buf.len(), off, 4)?; // I3 before write
 buf[off..off+4].copy_from_slice(&v.to_le_bytes()); // I2
 Ok(())
}

fn read_u32(buf: &[u8], off: usize) -> Result<u32, Status> {
 check_bounds(buf.len(), off, 4)?;
 Ok(u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]))
}

fn read_bool(buf: &[u8], off: usize) -> Result<bool, Status> {
 Ok(match read_u8(buf, off)? { 0 => false, 1 => true, _ => return Err(InvalidData) }) // I5
}

// Variable body access (two-region layout): validate the descriptor, hand out the body slice.
// Never trusts caller-supplied prior state — the slot's own descriptor is the only source.
fn body_range(buf: &[u8], d: Descriptor, sample_len: usize) -> Result<Range<usize>, Status> {
 if d.count > d.max { return Err(InvalidData); } // structural bound
 if d.offset < d.header_end || d.offset % d.elem_align != 0 { return Err(InvalidData); } // aligned, inside header guard
 let bytes = u64::from(d.count).checked_mul(d.elem_size).ok_or(InvalidData)?; // I7
 let end = u64::from(d.offset).checked_add(bytes).ok_or(InvalidData)?;
 if end > u64::try_from(sample_len).map_err(|_| InvalidData)? { return Err(InvalidData); } // body ⊆ [0, sample_len)
 let end = usize::try_from(end).map_err(|_| InvalidData)?; // provably lossless: end ≤ sample_len
 Ok(d.offset .. end) // aligned, bounds-checked, zero-copy
}

// Generated composite encoders (I3 two-phase; no allocation — the schema is compile-time known):
// fn encode_x(dst: &mut [u8], value: &X) -> Result<CanonLen, Status> {
//     let plan = validate_x(value)?; // ALL bounds, UTF-8 (I10), enum tags, descriptor math, final sample_len
//     write_x(dst, value, &plan)     // descriptors, then bodies — zero bytes touched before this point
// }
// C callers get the same split: drm_validate_x(...) then drm_write_x(...).

// ---- drm-workspace::canon (toolchain) ----
fn emit_layout_json(model: &WorkspaceModel, topic: &TopicId) -> LayoutJson
// per-field {name, offset, size, align, region} + header_end + sample_max — the I6 contract
// artifact (u64 checked arithmetic; covers the two-region render)
```

## Protocols

*Layout fixture exchange (I6):*

1. `drm-workspace` emits `layout.json` per fixture during test setup.
2. `drm-core` crosscheck test loads same JSON, writes via codec, reads back structurally.
3. Mismatch ⇒ CI fail; spec change ⇒ both sides regenerate in one PR (R1 sync).

*Golden vector lifecycle (I1/I9):*

1. Hand-authored vectors: a specification author derives `{json,hex}` directly from this document; these are the independent oracle (I9).
2. `xtask golden-gen` may generate additional vectors via the drm-core write path — committed artifacts, never generated in CI.
3. C reader (`bindings/c/tests/golden/`) and Python reader (`bindings/python/tests/`) decode independently; all three languages must agree byte-for-byte, and generated vectors must match the hand-authored oracle.

## Edge Cases & Errors

| Case | Result |
|---|---|
| bool byte ∉ {0,1} | `InvalidData` (decode) |
| length/count descriptor > declared max | `InvalidData` pre-read |
| count × elem_size overflow | `InvalidData` (checked_mul) |
| descriptor offset misaligned / < header_end / bodies overlapping or out of order | `InvalidData` (I5 — two-region validation) |
| body end > sample_len | `InvalidData` — bodies must lie inside the current sample |
| `off + 4` overflow / near `u64::MAX` | `InvalidData`/`SizeOverflow` (checked_add), never wrapped |
| canonical sample size > `u32::MAX` | `SizeOverflow` at layout (MVP cap); 32-bit target maps via `try_from` ⇒ `InvalidConfig` |
| tail body of `u64`/`f64` | body offset element-aligned (absolute) |
| empty bounded body | valid; count 0, zero bytes in tail |
| offset+size beyond buffer | `InvalidArg`/`InvalidData` pre-access |
| unknown enum tag | `InvalidData` |
| enum > 256 variants | schema-parser `enum-too-large` (build time) |
| string value invalid UTF-8 (encode or decode side) | `InvalidArg`/`InvalidData` (I10 `string_utf8_matrix`) |
| string byte length > N (N = UTF-8 bytes) | `InvalidArg` (encode) / `InvalidData` (decode) |
| composite encode, late field over bound | whole-sample validation fails first; destination untouched (I3 `no_partial_write`) |
| shrink variable body, republish | stale tail bytes unreachable — no clearing (I8) |

## Test Matrix

| Case | Expected |
|---|---|
| 20 primitive + ≥ 10 composite vectors, 3 langs | I1 |
| `golden_independent` (hand-authored oracle incl. LE byte oracle `78 56 34 12`) | I9 + I2 |
| boundary matrix (type × 0/max/empty/full/max+1) | accept/reject exact |
| `no_partial_write` (late-field overflow, 0xAA destination probe) | I3 message-level two-phase |
| `codec_overflow` (huge N, near-`u64::MAX` offsets, 32-bit target) | I7 checked errors + `try_from` rejection |
| `observable_bytes` (shrink + republish; tail never scrubbed) | I8 |
| `string_utf8_matrix` (`C0 AF`, `E0 80 80`, `ED A0 80`, `F5 80 80 80`, truncated) | I10 reject both directions |
| ≥ 5 hand-computed struct fixtures (incl. multi-variable two-region) | match `layout_struct` |
| counting-alloc on codec path | 0 allocations (I4) |
| read-fuzz 5 min + Miri | I5 |
| crosscheck layout.json | I6 zero mismatch |
| alignment fixtures (`T[<=N]` of u64; two-region tail packing on x86-64 + ARM64) | element-aligned bodies |
