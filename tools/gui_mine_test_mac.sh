#!/usr/bin/env bash
# Checks that Baritone keeps mining with the ClickGUI open.
# Usage: gui_mine_test_mac.sh VERSION_DIR SEED
# Mines 4 minutes, opens the panel with .gui, mines 3 more minutes, and logs
# the diamond count every 20 s to gui-mine-<seed>.log in the test-runs folder.
set -u
V="$1"; SEED="$2"
H="$HOME/asteroid-project/test-home-gui"; mkdir -p "$H"; S="$H/control.sock"; LOG="$HOME/asteroid-project/test-runs/gui-mine-$SEED.log"
export HORIZON_HOME="$H" HORIZON_NO_AUDIO=1
c(){ "$V/horizon-ctl" --socket "$S" "$@"; }
d(){ c inventory 2>/dev/null | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["counts"].get("minecraft:diamond",0))' 2>/dev/null || echo "?"; }
c quit >/dev/null 2>&1; rm -f "$S"
"$V/horizon-ctl" --socket "$S" launch --game "$V/horizon-client" --world "GuiMine-$SEED-$RANDOM" --seed "$SEED" --wait --offscreen >/dev/null 2>&1
c options-set soundCategory_master 0.0 >/dev/null
c command "gamemode survival @s" >/dev/null; c command "give @s minecraft:diamond_pickaxe 1" >/dev/null; c command "give @s minecraft:bread 10" >/dev/null
c key t tap >/dev/null; c type "#mine diamond 128" >/dev/null; c key enter tap >/dev/null
: > "$LOG"
for i in $(seq 1 12); do sleep 20; echo "t=$((i*20))s panel=closed diamonds=$(d)" >> "$LOG"; done
c key t tap >/dev/null; c type ".gui" >/dev/null; c key enter tap >/dev/null
echo "panel opened" >> "$LOG"
for i in $(seq 1 9); do sleep 20; echo "t=$((240+i*20))s panel=open diamonds=$(d)" >> "$LOG"; done
c quit >/dev/null 2>&1
echo done >> "$LOG"
