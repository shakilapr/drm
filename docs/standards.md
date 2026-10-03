# Standards and Conventions

Central registry. Documents point here instead of restating. Guidance adopted; conformance never claimed.

## Document Standards

| Area | Standard |
|---|---|
| Normative language | BCP 14 (RFC 2119 + 8174); uppercase keywords only, only when all-caps |
| Architecture decisions | MADR 4.0.0 (`docs/adr/`) |
| Versioning | SemVer 2.0.0; lockstep 0.x until API freeze |
| Changelog | Keep a Changelog 1.1.0 |
| Commits | Conventional Commits 1.0.0 |
| Spec structure | RFC 7322 ideas: terminology, security, normative/informative references |
| Editor | EditorConfig (`.editorconfig`) |

## Conventions (adopted as concepts, unnamed)

- **Requirements**: unique append-only IDs; one-line normative statements; bidirectional traceability arch ⇄ req ⇄ impl ⇄ test (`xtask audit-traceability`); status `open|specified|implemented|verified`; flagged rows resolved manually, never guessed.
- **Architecture description**: system-of-interest, stakeholders, concerns, views, rationale; rationale lives in ADRs; architecture docs win conflicts.
- **Testing**: chain req → condition → case → environment → execution → result → traceability record; categories unit/property/compile-fail/platform/conformance/fuzz/sanitizer/stress/soak/golden; green phase suites become permanent blocking gates; flaky tests quarantined ≤ 1 week.

## Coding Standards (current enforcement only)

| Language | Rules | Gate |
|---|---|---|
| Rust | rustfmt; deny `missing_docs`, `unsafe_op_in_unsafe_fn`, clippy `all+pedantic`, `unwrap_used`/`expect_used`/`panic` (incl. tests); Result errors, no library panics; `// SAFETY:` on every unsafe; Rust API Guidelines naming; zero heap on data path; layering (`drm-types` zero deps); REQ citations on public fns/error branches/tests; tests use the `fixture` idiom | `fmt`, `lint`, `check-layering`, `audit-traceability` |
| C | C11 (freestanding on MCU); clang-format LLVM/100; CERT C baseline + defensive subset (fixed-width, explicit returns, bounded copies, no heap on MCU); `drm.h` append-only | `fmt`, conformance tests |
| C++ | C++20; Core Guidelines + CERT C++ baseline; no conformance claims; a future safety subset ⇒ C++17 | `fmt`, tidy (future) |
| Python | ruff `E,F,W,I,N,UP,B,SIM`; mypy `--strict`; owned-reads-only binding | `lint` |
| All | `@generated` banner, never hand-edit; comments carry invariants/citations only; zero data-path allocation | `gen-check`, review |

## Quality Model (measured in `docs/metrics/`, gates in `docs/gates/`)

Correctness (golden mismatches = 0) · Reliability (stress corruption = 0) · Performance (RT allocations/op = 0, latency percentiles) · Compatibility (3-lang + OS parity) · Maintainability (layering violations = 0, undocumented APIs = 0) · Security (advisories = 0, unjustified unsafe = 0).

## Secure Development

NIST SP 800-218 (SSDF): `deny.toml` advisories/licenses, fuzz + sanitizer gates, private disclosure (`SECURITY.md`), release gates, SBOM planned.

## Safety

Practices designed to facilitate integration into functional-safety development environments. No conformance claims.

## Planned / Deferred

REUSE 3.3 (`LICENSES/`, SPDX headers, `REUSE.toml`) · CFF 1.2.0 citation · RFC 9116 fields in `SECURITY.md` · `xtask sbom` (registered, later phase) · review classes + community files when contributors arrive.
