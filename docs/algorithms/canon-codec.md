# Canonical Codec Algorithms

Scope: canonical binary representation — primitive encode/decode over raw bytes, layout computation (declaration order, natural alignment, total padding), fixed/bounded array and enum codecs, structurally correct bounded-array alignment, checked size arithmetic, structural-bound enforcement, and deterministic observable bytes. Implements architecture.md §Canonical Binary Layout, §Structural Bounds, §Sample Metadata. Owners: E2 (`drm-workspace::canon` — layout) + E1 (`drm-core::canon` — codec over mapped bytes). Phases 3–4; the layout-fixture exchange is the permanent toolchain↔runtime gate.

## REQ Bindings

| Provisional slug | Architecture anchor | Numeric ID |
|---|---|---|
| `REQ-TOOL-layout-le` | §Canonical Binary Layout — consistent little-endian | — |
| `REQ-TOOL-layout-standard-sizes` | standard sizes (4-byte int, 4-byte IEEE float) | — |
| `REQ-TOOL-layout-declaration-order` | fields in declaration order, natural alignment, total padded | — |
| `REQ-TOOL-bounds-structural` | §Structural Bounds — `string[32]`, `f32[16]`, `f32[<=16]` | — |
| `REQ-TOOL-bounds-unbounded-reject` | unbounded data-plane structures rejected | — |
| `REQ-TOOL-layout-bounded-align` | bounded array data area aligned to element alignment | — |
| `REQ-TOOL-layout-checked` | all size/offset math checked; overflow ⇒ error | — |
| `REQ-TOOL-observable-bytes` | observable canonical bytes deterministic; out-of-sample bytes inaccessible | — |
| `REQ-CORE-codec-no-alloc` | codebase R5 — codec path zero allocation | — |
| `REQ-CORE-codec-bounds-before-write` | Phase 13 — reject beyond structural max before any write | — |
| `REQ-CORE-codec-no-ub` | arbitrary bytes decode to value or error, never UB | — |

## Data Structures

| Type | Size | Align | Canonical rule |
|---|---|---|---|
| `u8/i8/bool` | 1 | 1 | `bool` = byte 0/1; others ⇒ `InvalidData` on decode |
| `u16/i16` | 2 | 2 | LE |
| `u32/i32/f32` | 4 | 4 | LE; IEEE-754 bit pattern preserved (no NaN canonicalization) |
| `u64/i64/f64` | 8 | 8 | LE |
| `T[N]` | `N·size(T)` | `align(T)` | contiguous, row-major, no inter-element padding |
| `string[N]` | `4+N` | 4 | `u32` byte-length header + ≤ N UTF-8 bytes (N = max bytes) |
| `bytes[<=N]` | `4+N` | 4 | `u32` length + ≤ N raw bytes |
| `T[<=N]` | `align_up(4, align(T)) + N·size(T)` | `max(4, align(T))` | `u32` count, then elements at `data_off = align_up(4, align(T))` |
| `enum{…}` | 1 | 1 | `u8` tag = declaration index; unknown tag ⇒ `InvalidData` |

The bounded-array data area must be element-aligned: a naive `4 + N·size(T)` puts the first `u64`/`f64` element at `offset+4` (not 8-byte aligned). The data offset is therefore `align_up(4, align(T))`.

```rust
pub struct Layout { size: usize, align: usize }

fn layout(t: &Type) -> Result<Layout, Status> {
 Ok(match t {
 Prim(p) => prim_layout(p),
 Enum { .. } => Layout { size: 1, align: 1 },
 String { max } | Bytes { max } => Layout { size: 4usize.checked_add(*max as usize)?, align: 4 },
 FixedArray { elem, n } => {
 let e = layout(&elem.as_type())?;
 Layout { size: e.size.checked_mul(*n as usize)?, align: e.align }
 }
 Bounded { elem, max } => {
 let e = layout(&elem.as_type())?;
 let data_off = align_up_checked(4, e.align)?; // I7: element-aligned data area
 Layout {
 size: data_off.checked_add(e.size.checked_mul(*max as usize)?)?,
 align: e.align.max(4),
 }
 }
 })
}

fn layout_struct(fields: &[Field]) -> Result<StructLayout, Status> {
 // offset_i = align_up_checked(cursor, align_i)?; cursor = offset_i.checked_add(size_i)?;
 // total = align_up_checked(cursor, max_align)?; struct_align = max align_i. (DECLARATION ORDER)
 // All arithmetic checked (I7).
}
```

## Invariants

- **I1 GoldenIdentity**: for every `tests/fixtures/golden/` vector: Rust encode == C decode == Python(ctypes) decode, byte-identical. Tests: `golden_primitives`, `golden_composites`.
- **I2 LEAssert**: little-endian byte order on all target archs; explicit LE assertion executes on x86_64 + aarch64. Test: `canon_le_assert`.
- **I3 BoundsBeforeWrite**: payload exceeding structural bound ⇒ `InvalidArg`/`InvalidData` **before any destination byte is written**. Test: `boundary_matrix` (max+1 rows).
- **I4 NoAlloc**: steady-state encode/decode zero heap allocation. Test: `alloc` (counting GlobalAlloc).
- **I5 NoUB**: arbitrary input bytes decode to value or `InvalidData` — never UB/panic. Tests: read-fuzz, Miri (safe modules), fault-injection corrupted fixture.
- **I6 LayoutAgreement**: `drm-core` tests consume `layout.json` emitted by `drm-workspace` — zero mismatches. Test: `crosscheck` (permanent CI gate).
- **I7 CheckedArithmetic**: every size/offset/align computation uses `checked_add`/`checked_mul`/`checked align_up`; any overflow ⇒ error (`InvalidData`/`InvalidConfig`), never a wrapped small value. Test: `codec_overflow` (huge `N`, near-`usize::MAX` offsets).
- **I8 ObservableBytes**: every byte of the current canonical sample is deterministic; bytes outside the current encoded length are inaccessible and never transmitted; a reused region that would otherwise become observable is cleared before publication (e.g. shrink `new_len..prev_len` for a variable payload), never `new_len..capacity` blindly. Test: `observable_bytes` (record, shrink, assert no stale byte observable; memset-probe asserts no `..capacity` clear).
- **I9 IndependentVectors**: a subset of golden vectors is hand-authored directly from this specification (not produced by the implementation under test). Test: `golden_independent`.

## Functions

```rust
// ---- drm-core::canon (runtime; mapped/shared bytes) ----
fn write_f32(buf: &mut [u8], off: usize, v: f32) -> Result<(), Status> {
 check_bounds(buf.len(), off, 4)?; // I3 before write
 buf[off..off+4].copy_from_slice(&v.to_le_bytes()); // I2
 Ok(())
}

fn read_u32(buf: &[u8], off: usize) -> Result<u32, Status> { check_bounds(buf.len(), off, 4)?; Ok(u32::from_le_bytes(..)) }

fn read_bool(buf: &[u8], off: usize) -> Result<bool, Status> {
 Ok(match read_u8(buf, off)? { 0 => false, 1 => true, _ => return Err(InvalidData) }) // I5
}

fn read_bounded(buf: &[u8], off: usize, cap: usize, elem_size: usize, elem_align: usize)
 -> Result<Range<usize>, Status>
{
 let n = read_u32(buf, off)? as usize; // header first
 if n > cap { return Err(InvalidData); } // structural bound
 let header_end = off.checked_add(4).ok_or(InvalidData)?; // I7: no unchecked off+4
 let data_off = align_up_checked(header_end, elem_align)?; // I7: element-aligned data area
 let bytes = n.checked_mul(elem_size).ok_or(InvalidData)?; // I7: overflow-safe
 check_bounds(buf.len(), data_off, bytes)?;
 Ok(data_off .. data_off + bytes) // slice handed to caller
}

// Variable-length write: clearing policy (I8)
fn write_bounded(buf: &mut [u8], off: usize, cap: usize, new_len: usize, prev_len_slot: &mut usize, payload: &[u8])
 -> Result<(), Status>
{
 // ... write header + payload ...
 // I8: clear only the region that was visible before and is no longer part of the sample,
 // never the whole capacity.
 buf[off + data_off + new_len .. off + data_off + prev_len.max(new_len)].fill(0);
 *prev_len_slot = new_len;
 Ok(())
}

// ---- drm-workspace::canon (toolchain) ----
fn emit_layout_json(model: &WorkspaceModel, topic: &TopicId) -> LayoutJson
// per-field {name, offset, size, align} + total — the I6 contract artifact (checked arithmetic)
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
| length header > declared cap | `InvalidData` pre-read |
| count × elem_size overflow | `InvalidData` (checked_mul) |
| `off + 4` overflow / near `usize::MAX` | `InvalidData` (checked_add), never wrapped |
| bounded array of `u64`/`f64` | data area element-aligned (data_off ≡ 0 mod 8) |
| empty bounded payload | valid; count 0, data area empty |
| offset+size beyond buffer | `InvalidArg`/`InvalidData` pre-access |
| unknown enum tag | `InvalidData` |
| shrink variable payload on reuse | prior visible tail cleared; capacity tail untouched (I8) |

## Test Matrix

| Case | Expected |
|---|---|
| 20 primitive + ≥ 10 composite vectors, 3 langs | I1 |
| `golden_independent` (hand-authored oracle) | I9 |
| boundary matrix (type × 0/max/empty/full/max+1) | accept/reject exact |
| `codec_overflow` (huge N, near-max offsets) | I7 checked errors |
| `observable_bytes` (shrink + memset probe) | I8 |
| ≥ 5 hand-computed struct fixtures | match `layout_struct` |
| counting-alloc on codec path | 0 allocations (I4) |
| read-fuzz 5 min + Miri | I5 |
| crosscheck layout.json | I6 zero mismatch |
| alignment fixtures (`T[<=N]` of u64 on x86-64 + ARM64) | element-aligned data area |
