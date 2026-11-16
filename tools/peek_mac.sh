#!/usr/bin/env bash
# Quick look at a running test: last monitor lines, new mod/chat lines, a small screenshot.
# Usage: peek_mac.sh RUN VERSION_DIR [SHOT]
R="$HOME/asteroid-project/test-runs/$1"; V="$2"
tail -3 "$R/monitor.out"
/usr/bin/grep -E '"kind": "(mod|chat|event)"' "$R/log.jsonl" | tail -${4:-8} | cut -c1-220
if [ -n "${3:-}" ]; then
  "$V/horizon-ctl" --socket "$HOME/asteroid-project/test-home/control-$1.sock" screenshot "$R/peek.png" >/dev/null && sips -Z 1280 "$R/peek.png" --out "$R/peek-small.png" >/dev/null && echo "shot: $R/peek-small.png"
fi
