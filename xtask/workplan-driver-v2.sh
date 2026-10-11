#!/usr/bin/env bash
# Work-plan driver v2: retry-with-repair, quarantine, continue.
#
# Differences from workplan-driver.sh:
#   * on a gate failure it re-invokes the agent with the failing output (up to
#     DRM_WORKPLAN_ATTEMPTS), instead of stopping;
#   * after the last attempt it parks the week's work on a `wip/<week>` branch
#     (main stays green), records the week in `.workplan-blocked.json`, and
#     moves on when DRM_WORKPLAN_CONTINUE=1 (AGENTS.md #14 quarantine);
#   * skips weeks already quarantined so the run does not stall.
#
# Env: DRM_WORKPLAN_AGENT (opencode), DRM_WORKPLAN_ATTEMPTS (3),
#      DRM_WORKPLAN_CONTINUE (1), DRM_WORKPLAN_LOG, DRM_WORKPLAN_MAX.
set -uo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
LOG="${DRM_WORKPLAN_LOG:-workplan-driver.log}"
AGENT="${DRM_WORKPLAN_AGENT:-opencode}"
ATTEMPTS="${DRM_WORKPLAN_ATTEMPTS:-3}"
CONTINUE="${DRM_WORKPLAN_CONTINUE:-1}"
MAX="${DRM_WORKPLAN_MAX:-200}"
LEDGER=.workplan-progress.json
BLOCKED=.workplan-blocked.json

log() { printf '[%s] %s\n' "$(date -Is)" "$*" >>"$LOG"; }

gates() {
  cargo xtask fmt --check &&
    cargo xtask lint &&
    cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings &&
    cargo xtask test &&
    cargo xtask check-layering &&
    cargo xtask audit-traceability
}

# Print the first week that is neither done nor quarantined.
pick() {
  python3 - "$LEDGER" "$BLOCKED" <<'PY'
import json, os, re, sys
led, blk = sys.argv[1], sys.argv[2]
done = set()
if os.path.exists(led):
    done = {re.sub(r"-w", "w", s.strip().lower().removeprefix("work-plan-").removesuffix(".md"))
            for s in json.load(open(led)).get("done", [])}
blocked = set(json.load(open(blk)).get("blocked", [])) if os.path.exists(blk) else set()
weeks = []
for name in os.listdir("docs/work-plan"):
    m = re.match(r"work-plan-m(\d+)-w(\d+)\.md$", name)
    if m:
        weeks.append((int(m.group(1)), int(m.group(2)), f"m{int(m.group(1)):02d}w{int(m.group(2))}"))
weeks.sort()
for _, _, stem in weeks:
    if stem not in done and stem not in blocked:
        print(stem)
        break
PY
}

# m08w1 -> work-plan-m08-w1.md
file_of() { printf 'work-plan-%s.md\n' "$(printf '%s' "$1" | sed -E 's/^(m[0-9]+)w([0-9]+)$/\1-w\2/')"; }

quarantine() {
  python3 - "$BLOCKED" "$1" <<'PY'
import json, os, sys
p, w = sys.argv[1], sys.argv[2]
d = json.load(open(p)) if os.path.exists(p) else {"blocked": []}
if w not in d["blocked"]:
    d["blocked"].append(w)
open(p, "w").write(json.dumps(d, indent=2) + "\n")
PY
}

log "driver v2 start (agent=$AGENT attempts=$ATTEMPTS continue=$CONTINUE max=$MAX)"
for _ in $(seq 1 "$MAX"); do
  WEEK=$(pick)
  if [ -z "$WEEK" ]; then
    log "ALL WEEKS DONE OR QUARANTINED (blocked: $BLOCKED)"
    break
  fi
  FILE=$(file_of "$WEEK")
  log "=== week $WEEK ($FILE) ==="

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
    git add -A -- . ':(exclude)tests/fuzz/Cargo.lock' ':(exclude)handover.md'
    git diff --cached --quiet || git commit -q -m "feat: work-plan $WEEK (autonomous driver)"
    cargo xtask workplan mark-done "$WEEK" >>"$LOG" 2>&1
    git add "$LEDGER" && git commit -q -m "chore(workplan): mark $WEEK complete" >>"$LOG" 2>&1
    log "week $WEEK COMPLETE"
  else
    # Park the failing work on a wip branch so main stays green, then quarantine.
    git checkout -q -b "wip/$WEEK" 2>/dev/null || git checkout -q "wip/$WEEK"
    git add -A -- . ':(exclude)tests/fuzz/Cargo.lock' ':(exclude)handover.md'
    git diff --cached --quiet || git commit -q -m "wip(workplan): $WEEK failing attempt (gates red)"
    git checkout -q main
    quarantine "$WEEK"
    git add "$BLOCKED" && git commit -q -m "chore(workplan): quarantine $WEEK after $ATTEMPTS failed attempts" >>"$LOG" 2>&1
    log "week $WEEK QUARANTINED (work parked on wip/$WEEK)"
    if [ "$CONTINUE" != "1" ]; then
      log "stopping (DRM_WORKPLAN_CONTINUE=$CONTINUE)"
      break
    fi
  fi
done
log "driver v2 exit"
