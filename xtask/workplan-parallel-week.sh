#!/usr/bin/env bash
# Parallel work-plan week worker.
#
# Complements `workplan-driver-v2.sh`, which walks the plan head serially in the main
# working tree. This script takes ONE week, gives it its own git worktree and branch, runs
# the coding agent there, runs the full gate suite, and commits on green. It never touches
# `main` and never touches the ledger: the orchestrator merges a green branch into main and
# marks the week done, so the serial driver and the parallel workers cannot fight over the
# index.
#
# Usage: xtask/workplan-parallel-week.sh m16w2
# Env:   DRM_WORKPLAN_AGENT (opencode) · DRM_WORKPLAN_ATTEMPTS (3)
#        DRM_WORKPLAN_WT_ROOT (default: <repo>/../drm-wt-wt)
set -uo pipefail

WEEK="${1:?usage: workplan-parallel-week.sh <week>  (e.g. m16w2)}"
REPO="$(cd "$(dirname "$0")/.." && pwd)"
WT_ROOT="${DRM_WORKPLAN_WT_ROOT:-$REPO/../drm-wt-wt}"
WT="$WT_ROOT/$WEEK"
BRANCH="plan/$WEEK"
STEM="$(printf '%s' "$WEEK" | sed -E 's/^(m[0-9]+)w([0-9]+)$/\1-w\2/')"
FILE="work-plan-$STEM.md"
LOG="$WT_ROOT/$WEEK.log"
AGENT="${DRM_WORKPLAN_AGENT:-opencode}"
ATTEMPTS="${DRM_WORKPLAN_ATTEMPTS:-3}"
export PATH="$HOME/.cargo/bin:$PATH"

mkdir -p "$WT_ROOT"
log() { printf '[%s] %s\n' "$(date -Is)" "$*" >>"$LOG"; }

gates() {
  cargo xtask fmt --check &&
    cargo xtask lint &&
    cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings &&
    cargo xtask test &&
    cargo xtask check-layering &&
    cargo xtask audit-traceability
}

log "=== parallel worker $WEEK start (worktree $WT, branch $BRANCH) ==="

# `docs/work-plan/` is gitignored (the plan is local-only), so a fresh worktree does not have
# the week file the agent must read. Copy it in; the worker never commits it.
if ! git -C "$REPO" worktree list --porcelain | grep -qx "worktree $WT"; then
  git -C "$REPO" worktree add -B "$BRANCH" "$WT" main >>"$LOG" 2>&1 || {
    log "FATAL: could not create worktree"; exit 1; }
fi
mkdir -p "$WT/docs"
cp -r "$REPO/docs/work-plan" "$WT/docs/work-plan" 2>>"$LOG"

cd "$WT" || exit 1

ok=0
for attempt in $(seq 1 "$ATTEMPTS"); do
  log "attempt $attempt/$ATTEMPTS for $WEEK"
  if [ "$attempt" -eq 1 ]; then
    PROMPT="Implement work-plan week $WEEK exactly as specified in docs/work-plan/$FILE. Read AGENTS.md and every algorithm/spec doc the week references before editing. Add the code, tests, docs, gate evidence and metrics the week requires. Follow the repo rules: workspace lints deny missing_docs/unwrap/expect/panic; every public item and test cites a real REQ id from docs/req/registry.csv; backticks around code identifiers in doc comments; rustfmt max_width 100. When done run and make pass: cargo xtask fmt --check ; cargo xtask lint ; cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings ; cargo xtask test ; cargo xtask check-layering ; cargo xtask audit-traceability. Do NOT git commit."
  else
    PROMPT="Your previous attempt for work-plan week $WEEK failed the gate suite. Read the failing output below, fix the root causes, and finish the week. Do NOT git commit.

--- failing gate output (tail) ---
$(tail -c 4000 "$LOG")"
  fi
  "$AGENT" run "$PROMPT" >>"$LOG" 2>&1
  if gates >>"$LOG" 2>&1; then
    ok=1
    break
  fi
  log "attempt $attempt for $WEEK FAILED gates"
done

if [ "$ok" -eq 1 ]; then
  git add -A -- . ':(exclude)tests/fuzz/Cargo.lock' ':(exclude)handover.md' \
    ':(exclude).workplan-progress.json' ':(exclude).workplan-blocked.json'
  git diff --cached --quiet || git commit -q -m "feat: work-plan $WEEK (parallel worker)"
  log "GREEN $WEEK $(git rev-parse --short HEAD)"
else
  git add -A -- . ':(exclude)tests/fuzz/Cargo.lock' ':(exclude)handover.md' \
    ':(exclude).workplan-progress.json' ':(exclude).workplan-blocked.json'
  git diff --cached --quiet || git commit -q -m "wip(workplan): $WEEK failing attempt (gates red)"
  log "RED $WEEK"
fi
log "=== parallel worker $WEEK exit (ok=$ok) ==="
