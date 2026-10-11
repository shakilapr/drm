#!/usr/bin/env bash
# Work-plan notifier: prints a line ONLY when the plan state changes or the
# driver has stopped before completion. Intended to be run by a cron job whose
# `deliver` targets a gateway (e.g. telegram), so a silent run = no message.
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
STATE=.workplan-notify.state

status="$(cargo xtask workplan status 2>/dev/null | tr '\n' ' ' | sed 's/  */ /g')"
if pgrep -f 'workplan-driver' >/dev/null 2>&1; then
  alive="up"
else
  alive="down"
fi
sig="$alive|$status"
prev="$(cat "$STATE" 2>/dev/null)"
printf '%s\n' "$sig" >"$STATE"

[ "$sig" = "$prev" ] && exit 0

if [ "$alive" = "down" ] && ! printf '%s' "$status" | grep -q '68/96\|96/96'; then
  printf 'DRM work-plan driver STOPPED. %s\n' "$status"
else
  printf 'DRM work plan: %s (driver %s)\n' "$status" "$alive"
fi
