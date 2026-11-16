# Watches a Baritone survival run through horizon-ctl and logs what happens (macOS port of monitor.py).
# Usage: python3 monitor_mac.py CTL SOCKET RUN_DIR MAX_MINUTES TARGET
#   CTL     the horizon-ctl next to the built horizon-client
#   SOCKET  the control socket the test game was launched with
# Stops at the first death, when TARGET diamonds are held, or after MAX_MINUTES.
# Logs state every ~3 s, inventory every ~10 s, and every hurt/death/chat event
# (with its cause) from `horizon-ctl events` to RUN_DIR/log.jsonl.
import json, os, subprocess, sys, time

ctl_path, sock, run_dir, minutes, target = sys.argv[1], sys.argv[2], sys.argv[3], float(sys.argv[4]), int(sys.argv[5])
os.makedirs(run_dir, exist_ok=True)
log = open(os.path.join(run_dir, "log.jsonl"), "a", encoding="utf-8")
FOODS = ("bread", "cooked_beef", "baked_potato", "cooked_porkchop", "cooked_chicken", "cooked_mutton", "apple", "carrot", "golden_carrot")


def ctl(*args, timeout=20):
    try:
        out = subprocess.run([ctl_path, "--socket", sock, *args], capture_output=True, text=True, timeout=timeout).stdout
        return json.loads(out) if out.strip().startswith("{") else {}
    except Exception as e:
        return {"error": str(e)}


def write(kind, **data):
    data.update(kind=kind, t=round(time.time() - start, 1))
    log.write(json.dumps(data) + "\n")
    log.flush()


start = time.time()
low_hp, low_food, diamonds, last_inv, since = 20.0, 20, 0, 0, None
hurts, causes = 0, {}
result = "timeout"
since = ctl("events").get("last")
seen_mod = set()
pick_damage = None
while time.time() - start < minutes * 60:
    st = ctl("state")
    p = st.get("player") or st
    hp, food = p.get("health"), p.get("food")
    pos = [round(v, 1) for v in (p.get("position") or [])]
    screen = st.get("screen")
    if isinstance(hp, (int, float)):
        low_hp = min(low_hp, hp)
    if isinstance(food, (int, float)):
        low_food = min(low_food, food)
    # Events: hurt (with cause), death, chat.
    ev = ctl("events", "--since", str(since)) if since is not None else ctl("events")
    died = None
    since = ev.get("last", since)
    for e in ev.get("events") or []:
        k = e.get("event") or e.get("kind") or e.get("type")
        if k in ("chat", "message"):
            write("chat", text=e.get("text") or e.get("message"))
        elif k != "item":
            write("event", **{x: y for x, y in e.items() if x not in ("event", "kind", "t")}, event=k)
            if k == "hurt":
                hurts += 1
                c = str(e.get("cause"))
                causes[c] = causes.get(c, 0) + 1
            if k == "death":
                died = e
    # The mod's own lines ([Asteroid] ...), newest first in `messages`.
    for m in reversed((ctl("messages", "30").get("styled") or [])):
        text = m.get("text") if isinstance(m, dict) else None
        if text and text.startswith("[Asteroid]") and text not in seen_mod:
            seen_mod.add(text)
            write("mod", text=text)
    if time.time() - last_inv > 10:
        last_inv = time.time()
        inv = ctl("inventory")
        counts = inv.get("counts", {})
        diamonds = counts.get("minecraft:diamond", 0)
        slots = (inv.get("hotbar") or []) + (inv.get("main") or [])
        pick = next((s for s in slots if (s or {}).get("item") == "minecraft:diamond_pickaxe"), None)
        pick_damage = (pick or {}).get("damage")
        food_left = sum(v for k, v in counts.items() if k.removeprefix("minecraft:") in FOODS)
        free = sum(1 for s in slots if not (s or {}).get("item"))
        write("inv", diamonds=diamonds, food_items=food_left, free_slots=free, pick_damage=pick_damage, pick=bool(pick), items=counts)
    write("state", hp=hp, food=food, sat=p.get("saturation"), pos=pos, screen=screen)
    print(f"{time.time()-start:6.0f}s hp={hp} food={food} pos={pos} diamonds={diamonds} pick={pick_damage} screen={screen}", flush=True)
    if died or screen in ("dead", "death") or (isinstance(screen, dict) and "death" in json.dumps(screen).lower()):
        result = "died"
        break
    if diamonds >= target:
        result = "done"
        break
    time.sleep(3)

summary = dict(result=result, minutes=round((time.time() - start) / 60, 1), diamonds=diamonds, lowest_hp=low_hp,
               lowest_food=low_food, hurt_events=hurts, hurt_causes=causes, pick_damage=pick_damage)
write("summary", **summary)
json.dump(summary, open(os.path.join(run_dir, "summary.json"), "w"), indent=1)
print("SUMMARY", json.dumps(summary))
