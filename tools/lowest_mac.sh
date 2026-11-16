#!/usr/bin/env bash
# Per run: result, diamonds, minutes, lowest health seen (monitor ~3 s and debug log ~1 s), pickaxe wear.
for r in "$@"; do
  R="$HOME/asteroid-project/test-runs/$r"
  python3 -I - "$R" <<'PY'
import json, os, re, sys
R = sys.argv[1]
low = 20.0
for l in open(os.path.join(R, "log.jsonl")):
    d = json.loads(l)
    if d["kind"] == "state" and isinstance(d.get("hp"), (int, float)):
        low = min(low, d["hp"])
dbg = os.path.join(R, "baritone-debug.log")
dlow = None
if os.path.exists(dbg):
    v = [float(m) for m in re.findall(r" hp=(\S+)", open(dbg).read())]
    dlow = min(v) if v else None
s = json.load(open(os.path.join(R, "summary.json"))) if os.path.exists(os.path.join(R, "summary.json")) else {}
print(f"{os.path.basename(R):22} {s.get('result','running'):8} diamonds={s.get('diamonds')} min={s.get('minutes')} lowest_hp(monitor)={low:.1f} lowest_hp(debug)={dlow} pick={s.get('pick_damage')}")
PY
done
