#!/usr/bin/env bash
# Unattended work-plan driver: advance one week at a time until the plan is
# complete. For each week it asks a coding agent (default: opencode) to
# implement `docs/work-plan/work-plan-mNN-wN.md`, then verifies the full gate
# suite and commits on green. It stops on the first gate failure so a human can
# intervene. Progress is tracked by `cargo xtask workplan` (ledger:
# `.workplan-progress.json`).
#
# Env: DRM_WORKPLAN_AGENT (default opencode), DRM_WORKPLAN_LOG, DRM_WORKPLAN_MAX.
set -uo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
LOG="${DRM_WORKPLAN_LOG:-workplan-driver.log}"
AGENT="${DRM_WORKPLAN_AGENT:-opencode}"
MAX="${DRM_WORKPLAN_MAX:-200}"

log() { printf '[%s] %s\n' "$(date -Is)" "$*" >>"$LOG"; }

gates() {
  cargo xtask fmt --check &&
    cargo xtask lint &&
    cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings &&
    cargo xtask test &&
    cargo xtask check-layering &&
    cargo xtask audit-traceability
}

log "driver start (agent=$AGENT max=$MAX)"
for _ in $(seq 1 "$MAX"); do
  WEEK=$(cargo xtask workplan next 2>/dev/null | grep -E '^m[0-9]+w[0-9]+$' | tail -1)
  if [ -z "$WEEK" ]; then
    log "ALL WEEKS COMPLETE"
    break
  fi
  FILE="work-plan-$(printf '%s' "$WEEK" | sed -E 's/^(m[0-9]+)w/\1-w/').md"
  log "=== week $WEEK ($FILE) ==="
  cargo xtask workplan status >>"$LOG" 2>&1

  "$AGENT" run "Implement work-plan week $WEEK exactly as specified in docs/work-plan/$FILE. Read AGENTS.md and every algorithm/spec doc the week references before editing. Add the code, tests, docs, gate evidence and metrics the week requires. Follow the repo rules: workspace lints deny missing_docs/unwrap/expect/panic; every public item and test cites a real REQ id from docs/req/registry.csv; backticks around code identifiers in doc comments; rustfmt max_width 100. When done, run and make pass: cargo xtask fmt --check ; cargo xtask lint ; cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings ; cargo xtask test ; cargo xtask check-layering ; cargo xtask audit-traceability. Do NOT git commit; leave the working tree with all gates green." >>"$LOG" 2>&1
  log "agent finished for $WEEK; verifying gates"

  if gates >>"$LOG" 2>&1; then
    git add -A -- . ':(exclude)tests/fuzz/Cargo.lock' ':(exclude)handover.md'
    if git diff --cached --quiet; then
      log "no changes produced for $WEEK"
    else
      git commit -q -m "feat: work-plan $WEEK (autonomous driver)" && log "committed $WEEK" || log "commit failed for $WEEK"
    fi
    cargo xtask workplan mark-done "$WEEK" >>"$LOG" 2>&1
    git add .workplan-progress.json && git commit -q -m "chore(workplan): mark $WEEK complete" >>"$LOG" 2>&1
    log "week $WEEK complete"
  else
    log "GATES FAILED for $WEEK - stopping for review (see $LOG)"
    break
  fi
done
log "driver exit"
