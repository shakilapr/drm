# Phase 4 Gate — Canonical Layout, Golden Triple

Evidence mapping, week plan M02W4-E2-T1 criteria ⇄ tests.
Implementation: `drm-types` codec, fixtures under [tests/fixtures/golden/](../../tests/fixtures/golden/),
runners in Rust suites, C ([bindings/c/tests/golden/](../../bindings/c/tests/golden/)), Python
([bindings/python/tests/test_golden_bytes.py](../../bindings/python/tests/test_golden_bytes.py)).
Verdict: **PASS** (triple green locally; evidence CI-replayed below).

## Criteria ⇄ evidence

| Criterion | Evidence | Status |
|---|---|---|
| Golden set final | `xtask golden-gen` idempotent (`git diff --exit-code tests/fixtures/golden`); fixtures frozen: primitives 24 positive + 1 malformed-negative, composites 10, descriptor 1 ⇒ 35 golden vectors | PASS |
| Canonical layout identical across languages | Python `pytest bindings/python/tests` (3 passed: primitives, composites, UTF-8-malformed rejection); C `golden_composites` (10/10 vectors, gcc `-Wall -Wextra -Werror`); Rust `golden_primitives` + `golden_composites` suites | PASS |
| Descriptor byte-exactness | `descriptor_golden.rs` (field-for-field byte table + compile-time offset asserts; REQ-CORE-014/015/016) | PASS |
| C++ closure documented | C++ decode-only golden reader (`bindings/cpp/tests/golden/golden_decode.cpp`; g++ 13.3 + clang++ 18.1) verifies 35/35 vectors — closes REQ-BIND-013 | PASS (M07W4) |

## Golden totals (M02W4 `GOLDEN_TOTAL ≥ 30 ×3 langs`)

| Language | Runner | Vectors verified | Status |
|---|---|---|---|
| Rust | `cargo test --workspace` (`golden_primitives`, `golden_composites`, `descriptor_golden`) | 24 + 10 + 1 | PASS |
| C | `gcc bindings/c/tests/golden/golden_composites.c` runner | 10 composites (+ `golden_read.c` reader semantics) | PASS |
| Python | `pytest bindings/python/tests/test_golden_bytes.py` | 24 + 10 (+ 1 negative) | PASS |
| C++ | `bindings/cpp/tests/golden/run.sh` (g++ + clang++) | 24 + 10 + 1 | PASS |

## Requirements covered

Canonical binary layout: REQ-TOOL-040 (bounded arrays aligned), REQ-TOOL-041 (checked math),
REQ-TOOL-042 (declaration order, natural alignment, padding), REQ-TOOL-043 (little-endian),
REQ-TOOL-044 (standard sizes). Descriptor: REQ-CORE-014 (aligned control state, offset asserts),
REQ-CORE-015 (stored once per segment), REQ-CORE-016 (exact offsets/counts/size).

## Regression board (M02W4-E4-T1, all suites green via `cargo xtask test`)

| Crate | Suites |
|---|---|
| `drm-cli` | unit (0) |
| `drm-core` | lib 47; `attach_semantics` 4; `canon_le_assert` 1; `canon_read_fuzz` 2 (1 ignored, nightly); `crosscheck` 1; `descriptor_golden` 2; `golden_primitives` 1; `ordering_smoke` 2; stress binaries `no_fd_leak`/`platform`/`quota_edge`/`shm_kill` (0 local, stress lane) |
| `drm-ffi` | lib 2 |
| `drm-router` | lib 21; `config_parse` 9; `crc32c_kat` 3; `framing_property` 3; `framing_record` 4; `route_table` 2 |
| `drm-types` | lib 7; `blake3_kat` 3 |
| `drm-workspace` | lib 10; `boundary_matrix` 2; `fixtures_valid` 3; `fuzz_campaign` 1 (ignored, nightly); `fuzz_seeds` 2; `golden_composites` 11; `handle_stability` 7; `layout_emit` 2; `model_attrs` 4; `model_determinism` 4; `model_fingerprint` 6; `model_nodes` 5; `model_roundtrip` 1; `model_topology` 7; `parser_corpus` 4; `property_types` 1; `roundtrip` 1; `topology_graph` 19; `types_codec` 13; `types_layout` 7; `types_unbounded` 8 |
| `xtask` | lib 1; soak parser 1 |

Status: 0 failures across the workspace; doc-tests green (6 crates). Signing role E3.

## Notes

- The golden byte contract is the frozen vector set, not any runner: adding vectors requires
  `xtask golden-gen` plus all language runners in the same change.
- Negative-classification fixtures (UTF-8 malformed) verify decode-side rejection parity, not just
  byte identity.
- CI replays the triple via `cargo xtask test` (nightly board); sanitizer/fuzz lanes remain
  CI-pending per [phase-08.md](phase-08.md) notes.
