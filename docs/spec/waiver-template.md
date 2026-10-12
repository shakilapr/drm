# Waiver Template

Derived component spec (codebase.md §2.4; non-normative — the algorithm doc
[qualification.md](../algorithms/program/qualification.md) wins conflicts).
`REQ-QAL-029`; the pre-drafted waiver template recorded in M24W3. Machine form:
[xtask/src/release.rs](../../xtask/src/release.rs).

## Discipline

A red evidence line at the Phase 34 gate may not pass silently. It is only
cleared by a recorded waiver carrying a rationale and a review quorum; a red
line with no waiver, or with an invalid one, fails `cargo xtask release --check`.
The waiver is folded into the release candidate as `waiver_clean` (`V10_RC`).

A waiver is valid iff it names the line and carries:

- a **non-empty rationale** — why the line is red and why it is accepted; and
- a **review quorum** of `WAIVER_QUORUM` (`2`) **distinct** approvers.

A blank rationale, a missing line, or a single (or duplicate) approver is an
informal waiver and is rejected. There is no silent pass.

## Template

```text
Waiver
  line:      <evidence-line id, e.g. I-QAL2 or REQ-QAL-024>
  rationale: <why the line is red and why the deferral is accepted; non-empty>
  approvers: <name>, <name>        # at least WAIVER_QUORUM = 2 distinct approvers
```

Recorded verdict: `waiver_clean = true` iff every red line has a valid waiver.
The M24W3 gate has no red evidence lines, so the ledger is empty and therefore
clean; the first real waiver requires an ADR (work-plan risks: *no waiver without
ADR*).

## Verification

```text
cargo xtask release --check
```

REQ: REQ-QAL-029.
