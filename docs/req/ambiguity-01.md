# Ambiguity Report #1 — registry extraction pass 1 (M01W1-E2-T3)

Source: `docs/req/migrate-registry.ps1 -Report` over `docs/req/registry.csv`
(325 rows, 325 unique; 106+ rows flagged for phase resolution; xtask
`audit-traceability --registry` reports the live count). Classification is
per-row; no row is resolved by guessing (docs/req/README.md).

## Classes

| Class | Meaning | Rows |
|---|---|---|
| wording-gap | phase column lists multiple work-plan phases; the gating phase must be chosen from the owning algorithm doc's Test Matrix anchor | all flagged rows |

No contradictions or missing definitions were observed in pass 1; flagged rows
are phase-resolution work, not statement defects.

## Proposed resolution

For each flagged row, the decider picks the single phase whose weekly gate
verifies the requirement, using the `source_ref` algorithm doc's Test Matrix as
the anchor. Multi-phase entries that legitimately gate in several weeks keep the
phase that closes the last open sub-requirement.

## Deciders

- CORE rows: E1 (runtime) · TOOL: E2 · ROUTE: E3 · SPATIAL: E2 · BIND: E4 ·
  STUDIO: E4 · OPS: E1 · QAL: E4 (rotating test lead).

## Deadline

Resolution completes with the per-algorithm REQ reviews in W2–W4
(work-plan-m01-w2.md, work-plan-m01-w3.md); unresolved rows block their phase
gate, not the W1 gate.
