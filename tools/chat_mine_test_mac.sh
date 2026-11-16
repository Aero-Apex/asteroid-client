#!/usr/bin/env bash
# Checks that Baritone keeps mining with the chat open, and that the chat is still open.
# Usage: chat_mine_test_mac.sh VERSION_DIR SEED
# Mines until 5 diamonds, opens chat (T) and types a line without sending, mines 3 more min,
# logs diamonds and the screen every 20 s to chat-mine-<seed>.log.
set -u
V="$1"; SEED="$2"
H="$HOME/asteroid-project/test-home-gui"; mkdir -p "$H"; S="$H/control.sock"; LOG="$HOME/asteroid-project/test-runs/chat-mine-$SEED.log"
export HORIZON_HOME="$H" HORIZON_NO_AUDIO=1
c(){ "$V/horizon-ctl" --socket "$S" "$@"; }
d(){ c inventory 2>/dev/null | python3 -I -c 'import json,sys; print(json.load(sys.stdin)["counts"].get("minecraft:diamond",0))' 2>/dev/null || echo "?"; }
scr(){ c state 2>/dev/null | python3 -I -c 'import json,sys; print(json.load(sys.stdin).get("screen"))' 2>/dev/null || echo "?"; }
c quit >/dev/null 2>&1; rm -f "$S"
"$V/horizon-ctl" --socket "$S" launch --game "$V/horizon-client" --world "ChatMine-$SEED-$RANDOM" --seed "$SEED" --wait --offscreen >/dev/null 2>&1
c options-set soundCategory_master 0.0 >/dev/null
c command "gamemode survival @s" >/dev/null; c command "give @s minecraft:diamond_pickaxe 1" >/dev/null; c command "give @s minecraft:bread 10" >/dev/null
c key t tap >/dev/null; c type "#mine diamond 128" >/dev/null; c key enter tap >/dev/null
: > "$LOG"
# Wait (up to 8 min) until Baritone has its first 5 diamonds, then open the chat.
for i in $(seq 1 24); do sleep 20; n=$(d); echo "t=$((i*20))s screen=$(scr) diamonds=$n" >> "$LOG"; [ "$n" != "?" ] && [ "$n" -ge 5 ] 2>/dev/null && break; done
c key t tap >/dev/null; sleep 1; c type "still mining" >/dev/null
echo "chat opened, typed a line (not sent): screen=$(scr)" >> "$LOG"
for i in $(seq 1 9); do sleep 20; echo "chat t=$((i*20))s screen=$(scr) diamonds=$(d)" >> "$LOG"; done
c screenshot "$HOME/asteroid-project/test-runs/chat-mine-$SEED.png" >/dev/null 2>&1
c key esc tap >/dev/null; sleep 1; echo "after Esc: screen=$(scr)" >> "$LOG"
c quit >/dev/null 2>&1
echo done >> "$LOG"
