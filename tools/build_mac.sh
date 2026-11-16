#!/usr/bin/env bash
# Builds Asteroid Client on Horizon 66f3981 (the commit horizon-native.json pins)
# into the separate test instance "asteroid-test", so the player's own instance
# keeps its version. Prints the result and any compile errors.
LOG="$HOME/asteroid-project/builds/build-$(date +%Y%m%d-%H%M%S).log"
mkdir -p "$(dirname "$LOG")"
/Applications/Horizon.app/Contents/MacOS/horizon-launcher native-build --trust-native-code \
  --instance asteroid-test --version nightly-2026-11-13-9f42064 --profile performance \
  "$HOME/asteroid-project/asteroid-project/asteroid-client" > "$LOG" 2>&1
echo "exit=$?"
tail -n 2 "$LOG" | cut -c1-300
/usr/bin/grep -E "^(error|warning: unused)" -A12 "$HOME/Library/Application Support/horizon-launcher/native/build.log" | cut -c1-200 | head -${1:-150}
