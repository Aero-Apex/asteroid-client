// Asteroid Client's extra modules: NoClip, NoFog, NoBackground, AntiSpam,
// CameraDistance, TrueSight, Radar, Excavator, TreeBot, TillAura, AutoFarm,
// BonemealAura, MobEsp, PlayerEsp and ItemEsp. Original code.
//
// Some of these use the game's own hooks (fog, camera distance, entity outline,
// chat received), which the mixin bundle wires to the functions below. The
// Baritone-backed ones (Excavator, TreeBot, AutoFarm) run through Baritone's
// `#` commands. Barrier, portal and open-water ESP are drawn by render.rs.
use super::Game;
use crate::game::asteroid_core::{self as core, State};
use horizon_client::command::args::GameMode;
use horizon_client::hud::HudBatch;
use std::{
    cell::RefCell,
    collections::HashMap,
    time::{Duration, Instant},
};

#[derive(Default)]
struct Rt {
    /// NoClip: whether the player was in creative mode before it, to put back.
    creative_before: Option<bool>,
    /// The Baritone-backed module that started the current Baritone process.
    owner: Option<&'static str>,
    /// AntiSpam: when each line was last shown.
    seen: HashMap<String, Instant>,
    /// The tick count, for the aura modules' pacing.
    tick: u64,
    /// The aura modules hold the right button for a couple of ticks, as a press
    /// that is let go within the tick does not register as a use.
    right_since: Option<u64>,
}

thread_local! {
    static RT: RefCell<Rt> = RefCell::new(Rt::default());
}

/// Reach for the auras, in blocks.
const AURA_REACH: f64 = 4.5;

pub fn toggled(g: &mut Game<'_>, s: &mut State, name: &str, on: bool) {
    match (name, on) {
        ("NoClip", true) => {
            let creative = g.creative;
            RT.with(|rt| rt.borrow_mut().creative_before = Some(creative));
            // Spectator movement: through blocks, nothing in the way.
            g.set_mode(GameMode::Spectator);
        }
        ("NoClip", false) => {
            if let Some(creative) = RT.with(|rt| rt.borrow_mut().creative_before.take()) {
                g.set_mode(if creative { GameMode::Creative } else { GameMode::Survival });
            }
        }
        ("TreeBot", true) => start_baritone(g, s, "TreeBot", "mine *_log 64"),
        ("Excavator", true) => start_baritone(g, s, "Excavator", "tunnel 64 1 2"),
        ("AutoFarm", true) => start_baritone(g, s, "AutoFarm", "farm 24"),
        ("TreeBot" | "Excavator" | "AutoFarm", false) => {
            let owned = RT.with(|rt| {
                let mut rt = rt.borrow_mut();
                if rt.owner == Some(static_name(name)) {
                    rt.owner = None;
                    true
                } else {
                    false
                }
            });
            if owned {
                crate::game::asteroid_baritone::command(g, s, "stop");
            }
        }
        _ => {}
    }
}

fn static_name(name: &str) -> &'static str {
    match name {
        "TreeBot" => "TreeBot",
        "Excavator" => "Excavator",
        _ => "AutoFarm",
    }
}

/// Starts a Baritone process for `module`, and remembers that it owns it.
fn start_baritone(g: &mut Game<'_>, s: &mut State, module: &'static str, line: &str) {
    RT.with(|rt| rt.borrow_mut().owner = Some(module));
    crate::game::asteroid_baritone::command(g, s, line);
}

/// Once a tick, with a player in the world.
pub fn tick(g: &mut Game<'_>, s: &mut State) {
    let tick = RT.with(|rt| {
        let mut rt = rt.borrow_mut();
        rt.tick += 1;
        rt.tick
    });
    // Let the right button go two ticks after an aura pressed it.
    let release = RT.with(|rt| {
        let mut rt = rt.borrow_mut();
        match rt.right_since {
            Some(since) if tick >= since + 2 => {
                rt.right_since = None;
                true
            }
            _ => false,
        }
    });
    if release {
        core::hold(g, true, false);
    }
    if s.on("TillAura") && tick.is_multiple_of(4) {
        till(g, s);
    }
    if s.on("BonemealAura") && tick.is_multiple_of(6) {
        bonemeal(g, s);
    }
}

/// The closest block within `range` of the player that passes `want` (its
/// name), that the eye can reach. `air_above`: the cell over it must be empty.
fn nearest(g: &Game<'_>, range: i32, air_above: bool, want: impl Fn(&str) -> bool) -> Option<[i32; 3]> {
    let feet = core::feet(g)?;
    let eye = g.eye_position()?;
    let facts = g.res.facts();
    let mut best: Option<([i32; 3], f64)> = None;
    for dx in -range..=range {
        for dz in -range..=range {
            for dy in -range..=1 {
                let p = [feet[0] + dx, feet[1] + dy, feet[2] + dz];
                let state = g.world.block(p[0], p[1], p[2]);
                if facts.is_air(state) || !want(&*facts.get(state).name) {
                    continue;
                }
                if air_above {
                    let above = g.world.block(p[0], p[1] + 1, p[2]);
                    if !facts.is_air(above) {
                        continue;
                    }
                }
                let aim = [f64::from(p[0]) + 0.5, f64::from(p[1]) + 1.0, f64::from(p[2]) + 0.5];
                let d = core::dist(eye, aim);
                if d > AURA_REACH || best.is_some_and(|b| b.1 <= d) {
                    continue;
                }
                // The ray must reach this block first: a snow layer or a fence in the way is no target.
                let dir = [0, 1, 2].map(|a| (aim[a] - eye[a]) / d.max(1e-6));
                // Plants have no collision, so the ray passes them: only solid blocks need the check.
                if air_above && g.clip_ray(eye, dir, AURA_REACH).is_none_or(|hit| hit.pos != p) {
                    continue;
                }
                best = Some((p, d));
            }
        }
    }
    best.map(|b| b.0)
}

/// The hotbar slot holding an item `want` names. An item carried in the inventory
/// is moved into the last hotbar slot first (single player: no screen needed).
fn hotbar_item(g: &mut Game<'_>, want: fn(&str) -> bool) -> Option<usize> {
    if let Some(slot) = core::hotbar_find(g, want) {
        return Some(slot);
    }
    if core::inventory_menu(g) {
        let from = core::inventory_find(g, want)?;
        core::swap_slot(g, from, 8);
        return core::hotbar_find(g, want);
    }
    None
}

fn is_hoe(item: &str) -> bool {
    item.ends_with("_hoe")
}

fn is_bone_meal(item: &str) -> bool {
    item == "minecraft:bone_meal"
}

/// Presses the right button, to be let go by `tick` two ticks on.
fn press_right(g: &mut Game<'_>) {
    let tick = RT.with(|rt| rt.borrow().tick);
    let fresh = RT.with(|rt| {
        let mut rt = rt.borrow_mut();
        if rt.right_since.is_none() {
            rt.right_since = Some(tick);
            true
        } else {
            false
        }
    });
    if fresh {
        core::hold(g, true, true);
    }
}

/// Tills the closest dirt-like block with a hoe.
fn till(g: &mut Game<'_>, s: &State) {
    let _ = s;
    let Some(slot) = hotbar_item(g, is_hoe) else {
        return;
    };
    let Some(p) = nearest(g, 4, true, |name| {
        matches!(name, "minecraft:dirt" | "minecraft:grass_block" | "minecraft:dirt_path" | "minecraft:coarse_dirt" | "minecraft:rooted_dirt")
    }) else {
        return;
    };
    g.selected = slot;
    // Aim at the top face, so the hoe's use lands on the block.
    core::look_at(g, [f64::from(p[0]) + 0.5, f64::from(p[1]) + 1.0, f64::from(p[2]) + 0.5]);
    press_right(g);
}

/// Bone-meals the closest crop or sapling with bone meal in the hotbar.
fn bonemeal(g: &mut Game<'_>, s: &State) {
    let _ = s;
    let Some(slot) = hotbar_item(g, is_bone_meal) else {
        return;
    };
    let Some(p) = nearest(g, 4, false, |name| {
        matches!(
            name,
            "minecraft:wheat" | "minecraft:carrots" | "minecraft:potatoes" | "minecraft:beetroots"
                | "minecraft:melon_stem" | "minecraft:pumpkin_stem" | "minecraft:oak_sapling"
                | "minecraft:spruce_sapling" | "minecraft:birch_sapling" | "minecraft:jungle_sapling"
                | "minecraft:acacia_sapling" | "minecraft:dark_oak_sapling" | "minecraft:cherry_sapling"
                | "minecraft:mangrove_propagule"
        ) || name.ends_with("_sapling")
    }) else {
        return;
    };
    g.selected = slot;
    core::look_at(g, [f64::from(p[0]) + 0.5, f64::from(p[1]) + 0.5, f64::from(p[2]) + 0.5]);
    press_right(g);
}

/// Entities the outline hook colours: MobEsp, PlayerEsp, ItemEsp, and TrueSight
/// (every entity). Glowing outlines show through walls.
pub fn outline_color(kind: &str) -> Option<u32> {
    let k = kind.trim_start_matches("minecraft:");
    let player = k == "player";
    let item = k == "item";
    let other = matches!(
        k,
        "experience_orb" | "arrow" | "spectral_arrow" | "trident" | "snowball" | "egg" | "ender_pearl"
            | "potion" | "splash_potion" | "lingering_potion" | "fireball" | "small_fireball"
            | "firework_rocket" | "wither_skull" | "tnt" | "falling_block" | "area_effect_cloud"
            | "marker" | "interaction" | "text_display" | "item_display" | "block_display"
    );
    core::with(|s| {
        if player && s.on("PlayerEsp") {
            Some(0xFF_55FF55)
        } else if item && s.on("ItemEsp") {
            Some(0xFF_55AAFF)
        } else if !player && !item && !other && s.on("MobEsp") {
            Some(0xFF_FF5555)
        } else if s.on("TrueSight") {
            Some(0xFF_FFFFFF)
        } else {
            None
        }
    })
    .flatten()
}

/// The world fog, before it is drawn: NoFog pushes it out of reach, NoBackground
/// takes the sky and clouds away.
pub fn apply_fog(fog: &mut horizon_client_render::fog::FogData) {
    let (nofog, nobg) = core::with(|s| (s.on("NoFog"), s.on("NoBackground"))).unwrap_or((false, false));
    if nofog {
        const FAR: f32 = 1.0e6;
        fog.environmental_start = FAR;
        fog.environmental_end = FAR + 1.0;
        fog.render_distance_start = FAR;
        fog.render_distance_end = FAR + 1.0;
    }
    if nobg {
        fog.sky_end = 0.0;
        fog.cloud_end = 0.0;
    }
}

/// The third-person camera distance, when CameraDistance is on.
pub fn camera_distance(distance: &mut f32) {
    if let Some(d) = core::with(|s| s.on("CameraDistance").then(|| s.n("CameraDistance", "Distance") as f32)).flatten() {
        *distance = d;
    }
}

/// AntiSpam: true when `text` was shown within the last few seconds, so this
/// copy is dropped.
pub fn drop_repeat(text: &str) -> bool {
    let window = core::with(|s| s.on("AntiSpam").then(|| s.n("AntiSpam", "Seconds"))).flatten();
    let Some(window) = window else {
        return false;
    };
    let now = Instant::now();
    RT.with(|rt| {
        let mut rt = rt.borrow_mut();
        let span = Duration::from_secs_f64(window);
        if let Some(last) = rt.seen.get(text)
            && now.duration_since(*last) < span
        {
            return true;
        }
        rt.seen.insert(text.to_owned(), now);
        if rt.seen.len() > 256 {
            rt.seen.retain(|_, t| now.duration_since(*t) < span);
        }
        false
    })
}

/// Radar: the nearest players, with their distance, down the left of the HUD.
pub fn hud(g: &Game<'_>, s: &State, batch: &mut HudBatch) {
    if !s.on("Radar") {
        return;
    }
    let Some(me) = g.player.as_ref().map(|p| [p.position.x, p.position.y, p.position.z]) else {
        return;
    };
    let range = s.n("Radar", "Range");
    let names = core::player_names(g);
    let own = g.remote.as_ref().map(|r| r.profile_name.clone());
    let mut rows: Vec<(f64, String)> = g
        .mobs
        .iter()
        .filter(|m| m.alive())
        .filter_map(|m| {
            let name = names.get(&m.id)?;
            if Some(name) == own.as_ref() {
                return None;
            }
            let d = core::dist(me, [m.position.x, m.position.y, m.position.z]);
            (d <= range).then(|| (d, name.clone()))
        })
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut y = 120.0;
    core::text_k(g, batch, "Radar", 6.0, y, 0x9A_95A6, 0.75);
    y += 10.0;
    for (d, name) in rows.iter().take(10) {
        core::text_k(g, batch, &format!("{name}  {d:.0} m"), 6.0, y, 0xF4_F2F6, 0.75);
        y += 9.0;
    }
}
