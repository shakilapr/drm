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
EXHAUSTED=.workplan-exhausted.json

log() { printf '[%s] %s\n' "$(date -Is)" "$*" >>"$LOG"; }

# Whether a week stem is currently in the quarantine ledger.
is_blocked() {
  python3 - "$BLOCKED" "$1" <<'PY'
import json, os, sys
p, w = sys.argv[1], sys.argv[2]
d = json.load(open(p)) if os.path.exists(p) else {"blocked": []}
print("1" if w in d.get("blocked", []) else "0")
PY
}

gates() {
  cargo xtask fmt --check &&
    cargo xtask lint &&
    cargo clippy --workspace --target x86_64-unknown-linux-gnu --all-targets -- -D warnings &&
    cargo xtask test &&
    cargo xtask check-layering &&
    cargo xtask audit-traceability
}

# Print the first week that is neither done nor quarantined.
#
# Two tiers, so a quarantined week is not abandoned forever:
#   tier 1 — weeks that are neither done, quarantined, nor exhausted;
#   tier 2 — quarantined weeks that have not yet failed a RETRY pass.
# A week that fails a retry pass is written to the exhausted ledger and never picked again, so
# the loop always terminates. Without this the driver declared "ALL WEEKS DONE OR QUARANTINED"
# with m10w2/m15w3/m22w1 silently missing from a plan it reported as complete.
#
# Termination does NOT depend on an external retry flag: `attempt` decides per week whether the
# week was ALREADY quarantined when the pass started. A week that is retried and fails again is
# exhausted (it has now failed ATTEMPTS attempts once and ATTEMPTS again on retry); a week that
# fails for the first time is quarantined and left for a later retry pass. Requiring an operator
# to create a flag file meant a re-failing quarantined week was re-picked MAX times in one run.
pick() {
  python3 - "$LEDGER" "$BLOCKED" "$EXHAUSTED" <<'PY'
import json, os, re, sys
led, blk, exh = sys.argv[1], sys.argv[2], sys.argv[3]

def load(path, key):
    if not os.path.exists(path):
        return set()
    try:
        return set(json.load(open(path)).get(key, []))
    except (OSError, ValueError):
        return set()

done = {re.sub(r"-w", "w", s.strip().lower().removeprefix("work-plan-").removesuffix(".md"))
        for s in load(led, "done")}
blocked = load(blk, "blocked")
exhausted = load(exh, "exhausted")

weeks = []
for name in os.listdir("docs/work-plan"):
    m = re.match(r"work-plan-m(\d+)-w(\d+)\.md$", name)
    if m:
        weeks.append((int(m.group(1)), int(m.group(2)), f"m{int(m.group(1)):02d}w{int(m.group(2))}"))
weeks.sort()

for _, _, stem in weeks:
    if stem not in done and stem not in blocked and stem not in exhausted:
        print(stem)
        raise SystemExit
for _, _, stem in weeks:
    if stem in blocked and stem not in exhausted and stem not in done:
        print(stem)
        raise SystemExit
PY
}

# A week that failed a retry pass is exhausted: retrying it forever would be an infinite loop.
exhaust() {
  python3 - "$EXHAUSTED" "$1" <<'PY'
import json, os, sys
p, w = sys.argv[1], sys.argv[2]
d = json.load(open(p)) if os.path.exists(p) else {"exhausted": []}
if w not in d["exhausted"]:
    d["exhausted"].append(w)
open(p, "w").write(json.dumps(d, indent=2) + "\n")
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
  WAS_BLOCKED="$(is_blocked "$WEEK")"
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
    # A retried week that lands must leave the quarantine ledger, or it is picked forever.
    if [ "$WAS_BLOCKED" = "1" ]; then
      python3 - "$BLOCKED" "$WEEK" <<'PY'
import json, sys
p, w = sys.argv[1], sys.argv[2]
d = json.load(open(p)) if __import__("os").path.exists(p) else {"blocked": []}
d["blocked"] = [x for x in d["blocked"] if x != w]
open(p, "w").write(json.dumps(d, indent=2) + "\n")
PY
      git add "$BLOCKED" && git commit -q -m "chore(workplan): clear quarantine for $WEEK after a green retry" >>"$LOG" 2>&1
    fi
    log "week $WEEK COMPLETE"
  else
    if [ "$WAS_BLOCKED" = "1" ]; then
      # This week was already quarantined and has now failed its retry pass: exhaust it so the
      # loop can terminate instead of picking it forever.
      exhaust "$WEEK"
      git add "$EXHAUSTED" && git commit -q -m "chore(workplan): exhaust $WEEK after a failed retry pass" >>"$LOG" 2>&1
      log "week $WEEK EXHAUSTED (failed a retry pass; not picked again)"
    else
      # Park the failing work on a wip branch so main stays green, then quarantine.
      git checkout -q -b "wip/$WEEK" 2>/dev/null || git checkout -q "wip/$WEEK"
      git add -A -- . ':(exclude)tests/fuzz/Cargo.lock' ':(exclude)handover.md'
      git diff --cached --quiet || git commit -q -m "wip(workplan): $WEEK failing attempt (gates red)"
      git checkout -q main
      quarantine "$WEEK"
      git add "$BLOCKED" && git commit -q -m "chore(workplan): quarantine $WEEK after $ATTEMPTS failed attempts" >>"$LOG" 2>&1
      log "week $WEEK QUARANTINED (work parked on wip/$WEEK)"
    fi
    if [ "$CONTINUE" != "1" ]; then
      log "stopping (DRM_WORKPLAN_CONTINUE=$CONTINUE)"
      break
    fi
  fi
done
log "driver v2 exit"
