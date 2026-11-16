#!/usr/bin/env bash
# Runs run_test_mac.sh on several seeds in turn and prints a table of the results.
# Usage: batch_mac.sh VERSION_DIR PREFIX MINUTES SEED...
V="$1"; P="$2"; M="$3"; shift 3
for s in "$@"; do "$(dirname "$0")/run_test_mac.sh" "$V" "$P-seed$s" "$M" 128 "$s" >/dev/null 2>&1; done
for s in "$@"; do printf "seed %-7s " "$s"; tr -d '\n ' < "$HOME/asteroid-project/test-runs/$P-seed$s/summary.json"; echo; done
