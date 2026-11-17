# Asteroid Client on the Mac: build, test, and the Baritone survival work

Continues `HANDOFF.md`. Everything here was done on an Apple Silicon Mac (macOS 27.2).

## Setup on this Mac

- **Launcher:** Horizon launcher 0.2.65, already installed at `/Applications/Horizon.app`.
  Its command-line tool is `/Applications/Horizon.app/Contents/MacOS/horizon-launcher`.
- **Rust:** the launcher's own toolchain (`horizon-launcher rust install`, Rust 1.99.1),
  in the launcher's data folder. Nothing was installed system-wide.
- **Horizon 66f3981:** the release `nightly-2026-11-05-66f3981` no longer exists
  (`install` says no release has that tag). The commit itself is still on GitHub,
  and `native-build` compiles a mod on the commit it pins. So builds use
  `--version nightly-2026-11-13-9f42064` (installed, for the game files) and the
  launcher fetches and compiles 66f3981's source. `native-mods.json` in each build
  confirms `"horizon": "66f3981…"`.
- **Mixins tool:** `horizon-native.json` pins `"mixins": "5a64af9…"` again. With it,
  the `god-hit` injection applies cleanly (no `E_UNSUPPORTED_MACRO`).
- **Separate instance:** builds go into a new launcher instance, **Asteroid Test**
  (`asteroid-test`). The player's own `horizon` instance and the launcher's
  settings were not changed.
- **Separate game home:** tests run with `HORIZON_HOME=~/asteroid-project/test-home`,
  `HORIZON_NO_AUDIO=1`, `--offscreen`, and `soundCategory_master:0.0`, `maxFps:60`
  in that home's `options.txt`.

## Scripts (in `tools/`)

| Script | What it does |
|---|---|
| `build_mac.sh` | Builds `asteroid-client/` on 66f3981 into `asteroid-test` (performance profile). Prints compile errors. |
| `run_test_mac.sh VERSION_DIR RUN [MIN] [TARGET] [SEED]` | Fresh world in the test home, offscreen and muted. Sets difficulty normal and survival mode, then gives only a diamond pickaxe and 10 bread. Types `.set Baritone DebugLog true` and `#mine diamond TARGET` into the chat box, then runs the monitor. |
| `monitor_mac.py` | Logs health, food, position, diamonds and pickaxe wear (state about every 3 s, inventory every 10 s), plus the mod's and the game's chat and events. Stops at a death, at TARGET diamonds, or at the time limit. Writes `log.jsonl` and `summary.json`. |
| `peek_mac.sh RUN VERSION_DIR [shot]` | Quick look at a running test. |

Results go to `~/asteroid-project/test-runs/<run>/`, together with the mod's debug log.

## What changed in the mod (all in `baritone.rs`, plus one setting in `core.rs`)

The survival layer from the handoff compiled after one fix (`ItemStack::damage` is
`u32`). Testing then turned up these problems, all fixed:

**Stalls (stuck for ever, no death):**
- A snow layer or grass in front of a block took the crosshair, so the block was never
  mined, and the stuck check never fired. Now the thin block is broken first, and
  a block that can't be aimed at doesn't count as progress.
- In a 1-wide shaft, at the top of each jump an ore counted as in reach, so it dug
  mid-air (5× slower), never placed the pillar block, fell, and repeated. Now digging
  starts only when standing, and an ore is given up after 15 s.
- "Place a block, then step up" failed while the player stood partly in that cell
  (the game refuses the block). Now it steps back to the middle first, and walling in
  does the same.
- `place()` reports success when it clicks, even when the game refuses the block.
  New watchdog: 20 s in the same cell on the same step bans that step from later plans.
- A "pillar up" step counted as done mid-jump, before the block was placed. Now it
  counts only when standing on the block.
- Last resort: 2 minutes without mining a block (time spent staying alive excluded),
  and the current targets are skipped.
- A creeper parked outside the wall kept it walled in for ever. After 3 minutes it now
  tunnels away, 16 blocks from the creeper.

**Dangers:**
- Lava: the search never stands where lava touches the body, the floor under the
  feet, or the cell above the head.
- Skeletons: the old rule walked up to a shooter to melee it, which took 11 damage in
  one test. Now any shooter that sees the player gets walled off at once, and the wall
  stays while a shooter is within 14 blocks. After 3 minutes it fights at full health.
- Walling in also starts at 2 melee mobs (was 3) and at health ≤ 12 with anything near
  (was 10). A creeper counts from 7 blocks if it can see the player, or 4 if not.
- Caves: a step with 3 or more open sides at head height costs more, so it prefers
  its own 1×2 tunnels.
- Gravel or sand: one test died ("suffocated in a wall"). It mined a diamond above
  its head, and gravel on that ore fell into the cells the player stood in. The
  dig-out then aimed one block too high. Now it digs out the real eye and feet cells,
  never breaks a block with sand or gravel on top, and never mines an ore above its
  head with sand or gravel on it.
- Never places a block into a cell its own body is in.
- Infested stone: silverfish poured into a shaft and took 5.5 hearts. Now it never
  breaks `infested_*` blocks.

**Other:**
- Planning takes 12 ms per tick (was 5) and stops at 1.5 s (was 3 s). Before, a long
  search meant about 30 s of standing still.
- The ore list sorts only the 64 nearest of the thousands known, each tick.
- New setting `DebugLog` (off by default) writes one line per second to
  `baritone-debug.log` in the mod's data folder: path step, digging, shelter, fight
  and why ores were skipped. New `#debug` command says the same line in chat.

## Test results

See `test-runs/` and the summary in the final report. Health is sampled about every
3 s by the monitor and every 1 s by the debug log. Damage healed between samples
could be missed, but a death could not, since the monitor also reads the game's
death event.

## Not done / still open

- Only Survival on the default settings was tested, on 5 seeds. Not tested: other
  difficulties (hard: hunger can kill), the Nether, servers, or targets other than diamonds.
- Pickaxe wear for 128 diamonds was 640–940 of 1561. Above about 1553 it stops itself
  before the pickaxe breaks.
- Endermen weren't seen in the tests. The mod has no special handling beyond
  "neutral within 3 blocks: fight".
- The build is `performance` profile. The handoff's command used `release`, which is
  not built here.

## Final results (build of commit a76892e, fresh world per seed, 60 min limit)

| Seed | Result | Diamonds | Minutes | Lowest health |
|---|---|---|---|---|
| 31337 | passed | 128 | 17.4 | 16.0 |
| 7 | passed | 130 | 13.8 | 20.0 |
| 2026 | passed | 128 | 20.1 | 14.8 |
| 90210 | passed | 128 | 14.3 | 20.0 |
| 4242 | passed | 128 | 14.7 | 20.0 |

Pickaxe wear at the end: 794–1025 of 1561. No deaths. Raw logs: `test-runs/v8-seed*`.
Earlier runs that died or stalled are kept in `test-runs/` under the prefixes
`pre-*`, `precave-*`, `precreeper-*`, `preshelter-*`, `prespawner-*` and `interrupted-*`.

## v1.1: Horizon 9f42064, Freecam, Fullbright, HUD

- **Horizon 9f42064** (`nightly-2026-11-13-9f42064`): the mod now pins it (`horizon-native.json`,
  `[compatibility]`), mod version 2.2.0. Three hook targets moved file in this Horizon
  (`apply_mouse_look` to `game::pointer_capture`, `mapped_key` and `play_mouse` to
  `game::play_input`); `mob_hit` now also returns the hit point, and there is a new
  `ProjectileKind::DragonFireball` for Trajectories. 9f42064 is the first Horizon whose shader
  packs can use API 0.2.
- **Freecam** is a camera of its own (`native_camera_pose`). The body keeps its game mode and keeps
  sending its movement, so Baritone, KillAura and the rest work with the body while the camera
  looks around: W A S D, Space and Shift move the camera (Ctrl faster), the mouse turns it, and
  neither reaches the body (`native_movement_input`). Turning the body (Baritone, KillAura) never
  turns the camera. The body is drawn once the camera is 1.5 blocks out of its head. Freecam no
  longer holds movement packets back as Blink does.
- **Fullbright** gives full night vision (a `night-vision` hook on `night_vision_intensity`), which
  the lightmap and shader packs (Iris `nightVision`, Horizon packs) brighten from, so it works with
  Complementary and other packs, in caves too.
- **HUD:** the bottom-right lines (Baritone's status, Armor, Facing, XYZ...) move above the hotbar,
  hearts and food when a narrow window would put them over the hotbar.

Tested offscreen on 9f42064: Baritone mined with Freecam on (camera still, body survival, body
didn't move while W and Space were held); KillAura turned the body toward a zombie while the
camera's picture stayed the same; Fullbright lit a sealed cave at midnight; the HUD lines sat
above the hotbar on a 640-wide window. Not tested on a real server or with a real keyboard.
