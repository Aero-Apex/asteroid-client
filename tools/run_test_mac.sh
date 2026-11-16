#!/usr/bin/env bash
# The 128-diamond survival test on macOS: a fresh world in its own game home
# (HORIZON_HOME), offscreen and muted, then `#mine diamond N` typed into the chat
# box, watched by monitor_mac.py until a death, N diamonds or the time limit.
# Usage: run_test_mac.sh VERSION_DIR RUN_NAME [MINUTES] [TARGET] [SEED]
#   VERSION_DIR  the launcher's versions/<build> folder with horizon-client and horizon-ctl
set -u
VER="$1"; RUN="$2"; MIN="${3:-240}"; TARGET="${4:-128}"; SEED="${5:-4242}"
ROOT="$HOME/asteroid-project"
HOME_DIR="${TEST_HOME:-$ROOT/test-home}"   # never the player's own game folder (TEST_HOME: a second lane)
RUN_DIR="$ROOT/test-runs/$RUN"
SOCK="$HOME_DIR/control-$RUN.sock"
CTL="$VER/horizon-ctl"
WORLD="Baritone-$RUN"
mkdir -p "$RUN_DIR"
DBG="$HOME_DIR/horizon-mods/.horizon-mod-data/asteroid_client/baritone-debug.log"
[ -f "$DBG" ] && mv "$DBG" "$DBG.$(date +%s).old"   # moved aside, not deleted
# Always a fresh world: an old save of the same name is moved aside, not reused.
[ -d "$HOME_DIR/saves/$WORLD" ] && mv "$HOME_DIR/saves/$WORLD" "$HOME_DIR/saves/$WORLD.old.$(date +%s)"
c() { "$CTL" --socket "$SOCK" "$@"; }

export HORIZON_HOME="$HOME_DIR" HORIZON_NO_AUDIO=1
c launch --game "$VER/horizon-client" --world "$WORLD" --seed "$SEED" --wait --offscreen > "$RUN_DIR/launch.json" 2>&1 \
  || { echo "launch failed"; cat "$RUN_DIR/launch.json"; exit 1; }
c options-set soundCategory_master 0.0 >/dev/null
c wait-ready 300 > /dev/null
# Survival, normal difficulty, a bare diamond pickaxe and 10 bread, nothing else.
for cmd in "difficulty normal" "gamemode survival @s" "clear @s" "effect clear @s" \
           "give @s minecraft:diamond_pickaxe 1" "give @s minecraft:bread 10" "time set 0"; do
  c command "$cmd" >> "$RUN_DIR/setup.log"; echo >> "$RUN_DIR/setup.log"
done
c select 1 > /dev/null
# `#` and `.` commands go through the chat box (the control tool's chat skips the mod).
c key t tap >/dev/null; sleep 0.5
c type ".set Baritone DebugLog true" >/dev/null; sleep 0.2
c key enter tap >/dev/null; sleep 0.5
c key t tap >/dev/null; sleep 0.5
c type "#mine diamond $TARGET" >/dev/null; sleep 0.2
c key enter tap >/dev/null
python3 -I "$ROOT/asteroid-project/tools/monitor_mac.py" "$CTL" "$SOCK" "$RUN_DIR" "$MIN" "$TARGET" > "$RUN_DIR/monitor.out" 2>&1
c screenshot "$RUN_DIR/end.png" > /dev/null 2>&1
c logs > "$RUN_DIR/game-log-tail.txt" 2>&1
cp "$DBG" "$RUN_DIR/baritone-debug.log" 2>/dev/null
c quit > /dev/null 2>&1
tail -1 "$RUN_DIR/monitor.out"
