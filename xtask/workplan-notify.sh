#!/usr/bin/env bash
# Work-plan notifier: prints a line ONLY when the plan state changes or the
# driver has stopped before completion. Intended to be run by a cron job whose
# `deliver` targets a gateway (e.g. telegram), so a silent run = no message.
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
STATE=.workplan-notify.state

status="$(cargo xtask workplan status 2>/dev/null | tr '\n' ' ' | sed 's/  */ /g')"
# Detect the driver by interpreter + script argument, never `pgrep -f <substring>`:
# that substring also matches any `bash -c` wrapper (including this script's own
# command line), so it reports "up" while the driver is dead. Match a bash process
# whose script argument is the driver (`ps -eo comm,args` puts comm in $1, the
# interpreter in $2, and the script path in $3).
if ps -eo comm,args --no-headers 2>/dev/null |
  awk '$1=="bash" && $3 ~ /workplan-driver(-v2)?\.sh$/' |
  grep -q .; then
  alive="up"
else
  alive="down"
fi
sig="$alive|$status"
prev="$(cat "$STATE" 2>/dev/null)"
printf '%s\n' "$sig" >"$STATE"

[ "$sig" = "$prev" ] && exit 0

# Alert whenever the driver is down and the plan is not complete. Derive
# "complete" from the status line (0 remaining) rather than hardcoding a week
# count, which goes stale every time a tranche lands.
if [ "$alive" = "down" ] && ! printf '%s' "$status" | grep -q '0 remaining'; then
  printf 'DRM work-plan driver STOPPED. %s\n' "$status"
else
  printf 'DRM work plan: %s (driver %s)\n' "$status" "$alive"
fi
