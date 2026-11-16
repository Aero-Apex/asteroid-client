// Asteroid Client's render modules and HUD: ESP, Tracers, Nametags,
// StorageESP, BlockESP, Xray (ores through walls), HoleESP, Fullbright,
// NoRender, CustomFOV, TimeChanger, Breadcrumbs, Trajectories, LogoutSpots,
// BlockSelection, BreakIndicators, Waypoints, and the HUD module (watermark,
// module list, info lines). Everything 3D is projected onto the HUD with the
// last frame's camera.
use super::Game;
use crate::game::asteroid_core::{self as core, State, fill, line, rect_outline, text};
use horizon_client::{extras::Corner, extras_hud::Corners, hud::HudBatch};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    time::Duration,
};

/// What the block scanner found: storage, a listed block, an ore.
const STORAGE: u8 = 1;
const LISTED: u8 = 2;
const ORE: u8 = 4;

#[derive(Default)]
struct Rt {
    /// Block state id → the kinds it counts as, per settings revision.
    kinds: HashMap<u32, u8>,
    lists: (Vec<String>, Vec<String>),
    /// Chunk → the matching blocks found in it.
    found: HashMap<(i32, i32), Vec<([i32; 3], u8)>>,
    queue: VecDeque<(i32, i32)>,
    trail: VecDeque<[f64; 3]>,
    /// Players seen last tick: id → (name, box min, box max).
    seen: HashMap<u32, (String, [f64; 3], [f64; 3])>,
    logouts: Vec<(String, [f64; 3], [f64; 3])>,
    /// Settings the modules changed, to put back.
    fov: Option<f32>,
    bobbing: Option<bool>,
    holes: Vec<[i32; 3]>,
}
thread_local! {
    static RT: RefCell<Rt> = RefCell::new(Rt::default());
}

pub fn toggled(g: &mut Game<'_>, s: &mut State, name: &str, on: bool) {
    RT.with(|rt| {
        let rt = &mut *rt.borrow_mut();
        match (name, on) {
            ("Fullbright", false) => {
                let (gamma, full) = (g.settings.gamma, g.settings.extras.fullbright);
                if let Some(r) = g.renderer.as_mut() {
                    r.gamma = gamma;
                    r.fullbright = full;
                }
            }
            ("CustomFOV", true) => rt.fov = Some(g.settings.fov),
            ("CustomFOV", false) => {
                if let Some(f) = rt.fov.take() {
                    g.settings.fov = f;
                }
            }
            ("NoRender", true) => rt.bobbing = Some(g.settings.view_bobbing),
            ("NoRender", false) => {
                if let Some(b) = rt.bobbing.take() {
                    g.settings.view_bobbing = b;
                }
            }
            ("Breadcrumbs", false) => rt.trail.clear(),
            ("LogoutSpots", false) => rt.logouts.clear(),
            ("StorageESP" | "BlockESP" | "Xray", true) => {
                rt.found.clear();
                rt.queue.clear();
                rt.kinds.clear();
            }
            _ => {}
        }
    });
    let _ = s;
}

pub fn tick(g: &mut Game<'_>, s: &mut State) {
    RT.with(|rt| {
        let rt = &mut *rt.borrow_mut();
        scan(g, s, rt);
        holes(g, s, rt);
        if s.on("Breadcrumbs")
            && let Some(p) = g.player.as_ref()
        {
            let now = [p.position.x, p.position.y + 0.1, p.position.z];
            if rt.trail.back().is_none_or(|l| core::dist(*l, now) > 0.3) {
                rt.trail.push_back(now);
            }
            let max = s.n("Breadcrumbs", "MaxPoints") as usize;
            while rt.trail.len() > max {
                rt.trail.pop_front();
            }
        }
        logout_spots(g, s, rt);
    });
}

pub fn frame(g: &mut Game<'_>, s: &mut State, _elapsed: Duration) {
    if s.on("Fullbright")
        && let Some(r) = g.renderer.as_mut()
    {
        r.fullbright = true;
        r.gamma = r.gamma.max(1.0);
    }
    if s.on("CustomFOV") {
        g.settings.fov = s.n("CustomFOV", "FOV") as f32;
    }
    if s.on("NoRender") {
        if s.b("NoRender", "Weather") && g.remote.is_some() {
            g.world.weather.rain_level = (0.0, 0.0);
            g.world.weather.thunder_level = (0.0, 0.0);
        }
        if s.b("NoRender", "ViewBobbing") {
            g.settings.view_bobbing = false;
        }
    }
    if s.on("TimeChanger") && g.remote.is_some() {
        g.world.day_time = s.n("TimeChanger", "Time") as i64;
    }
}

// ------------------------------------------------------------------ scanner

fn kinds_of(g: &Game<'_>, s: &State, rt: &mut Rt, state: horizon_world::registry::BlockStateId) -> u8 {
    if let Some(k) = rt.kinds.get(&state.0) {
        return *k;
    }
    let name = &*g.res.facts().get(state).name;
    let mut k = 0;
    if s.on("StorageESP") || s.on("StashFinder") {
        let storage = name.ends_with("chest")
            || name.ends_with("shulker_box")
            || matches!(
                name,
                "minecraft:barrel" | "minecraft:hopper" | "minecraft:dispenser" | "minecraft:dropper"
                    | "minecraft:furnace" | "minecraft:blast_furnace" | "minecraft:smoker"
                    | "minecraft:brewing_stand" | "minecraft:crafter" | "minecraft:decorated_pot"
            );
        if storage {
            k |= STORAGE;
        }
    }
    if s.on("BlockESP") && rt.lists.0.iter().any(|b| b == name) {
        k |= LISTED;
    }
    if s.on("Xray") && rt.lists.1.iter().any(|b| b == name) {
        k |= ORE;
    }
    rt.kinds.insert(state.0, k);
    k
}

fn scan(g: &Game<'_>, s: &State, rt: &mut Rt) {
    let any = s.on("StorageESP") || s.on("BlockESP") || s.on("Xray") || s.on("StashFinder");
    if !any {
        if !rt.found.is_empty() {
            rt.found.clear();
            rt.queue.clear();
        }
        return;
    }
    let lists = (s.list("BlockESP", "Blocks"), s.list("Xray", "Ores"));
    if lists != rt.lists {
        rt.lists = lists;
        rt.kinds.clear();
        rt.found.clear();
        rt.queue.clear();
    }
    let Some(feet) = core::feet(g) else {
        return;
    };
    let (cx, cz) = (feet[0].div_euclid(16), feet[2].div_euclid(16));
    const R: i32 = 4;
    rt.found.retain(|&(x, z), _| (x - cx).abs() <= R + 1 && (z - cz).abs() <= R + 1);
    if rt.queue.is_empty() {
        let mut order: Vec<(i32, i32)> = Vec::new();
        for dx in -R..=R {
            for dz in -R..=R {
                order.push((cx + dx, cz + dz));
            }
        }
        order.sort_by_key(|(x, z)| (x - cx).abs().max((z - cz).abs()));
        rt.queue.extend(order);
    }
    let (y0, y1) = ((feet[1] - 40).max(-64), (feet[1] + 40).min(319));
    for _ in 0..2 {
        let Some((x, z)) = rt.queue.pop_front() else {
            break;
        };
        let mut hits = Vec::new();
        for bx in 0..16 {
            for bz in 0..16 {
                let (wx, wz) = (x * 16 + bx, z * 16 + bz);
                for wy in y0..=y1 {
                    let state = g.world.block(wx, wy, wz);
                    let k = kinds_of(g, s, rt, state);
                    if k != 0 {
                        hits.push(([wx, wy, wz], k));
                        if hits.len() > 512 {
                            break;
                        }
                    }
                }
            }
        }
        rt.found.insert((x, z), hits);
    }
}

/// Containers per chunk, for StashFinder.
pub fn stashes() -> Vec<((i32, i32), usize)> {
    RT.with(|rt| {
        let rt = rt.borrow();
        rt.found
            .iter()
            .map(|(c, v)| (*c, v.iter().filter(|(_, k)| k & STORAGE != 0).count()))
            .collect()
    })
}

fn holes(g: &Game<'_>, s: &State, rt: &mut Rt) {
    if !s.on("HoleESP") {
        rt.holes.clear();
        return;
    }
    if !s.ticks.is_multiple_of(10) {
        return;
    }
    let Some(feet) = core::feet(g) else {
        return;
    };
    let r = s.n("HoleESP", "Range") as i32;
    rt.holes.clear();
    for dx in -r..=r {
        for dy in -4..=2 {
            for dz in -r..=r {
                let p = [feet[0] + dx, feet[1] + dy, feet[2] + dz];
                if crate::game::asteroid_combat::is_hole(g, p) {
                    rt.holes.push(p);
                }
            }
        }
    }
}

fn logout_spots(g: &Game<'_>, s: &State, rt: &mut Rt) {
    if !s.on("LogoutSpots") {
        rt.seen.clear();
        return;
    }
    let names = core::player_names(g);
    let mut now: HashMap<u32, (String, [f64; 3], [f64; 3])> = HashMap::new();
    for m in &g.mobs {
        if let Some(name) = names.get(&m.id) {
            let half = m.kind.width / 2.0;
            let p = m.position;
            now.insert(
                m.id,
                (name.clone(), [p.x - half, p.y, p.z - half], [p.x + half, p.y + f64::from(m.height()), p.z + half]),
            );
        }
    }
    let present: HashSet<&String> = now.values().map(|v| &v.0).collect();
    rt.logouts.retain(|l| !present.contains(&l.0));
    let listed: HashSet<&String> = names.values().collect();
    for (id, v) in &rt.seen {
        // Gone from the world and from the player list: logged out.
        if !now.contains_key(id) && !listed.contains(&v.0) {
            rt.logouts.retain(|l| l.0 != v.0);
            rt.logouts.push(v.clone());
        }
    }
    rt.seen = now;
}

// -------------------------------------------------------------------- draw

const PLAYER: u32 = 0xFF_FF_55_55;
const FRIEND: u32 = 0xFF_55_FF_FF;
const HOSTILE: u32 = 0xFF_FF_AA_00;
const ANIMAL: u32 = 0xFF_55_FF_55;
const ITEM: u32 = 0xFF_E0_E0_E0;
const ACCENT: u32 = 0xFF_91_3D_E8;

/// Corner brackets around a screen box: short arms on each corner.
fn brackets(g: &Game<'_>, batch: &mut HudBatch, x: f32, y: f32, w: f32, h: f32, argb: u32) {
    let arm = (w.min(h) / 3.0).clamp(3.0, 14.0);
    let (r, b) = (x + w, y + h);
    let arms = [
        (x, y, arm, 1.0), (x, y, 1.0, arm),
        (r - arm, y, arm, 1.0), (r - 1.0, y, 1.0, arm),
        (x, b - 1.0, arm, 1.0), (x, b - arm, 1.0, arm),
        (r - arm, b - 1.0, arm, 1.0), (r - 1.0, b - arm, 1.0, arm),
    ];
    for (ax, ay, aw, ah) in arms {
        fill(g, batch, ax, ay, aw, ah, argb);
    }
}

pub fn hud(g: &mut Game<'_>, s: &mut State, batch: &mut HudBatch, corners: &mut Corners) {
    RT.with(|rt| {
        let rt = &*rt.borrow();
        if let Some(cam) = core::camera(g, batch) {
            world_overlays(g, s, rt, batch, &cam);
        }
        // The ClickGUI has the screen while it is open.
        if s.on("HUD") && !s.gui.open {
            hud_text(g, s, batch, corners);
        }
    });
}

fn world_overlays(g: &Game<'_>, s: &State, rt: &Rt, batch: &mut HudBatch, cam: &core::Cam) {
    let eye = g.eye_position().unwrap_or_default();
    let names = core::player_names(g);
    let centre = cam.center();
    // Breadcrumbs
    if s.on("Breadcrumbs") {
        let pts: Vec<[f64; 3]> = rt.trail.iter().copied().collect();
        for w in pts.windows(2) {
            if let Some((a, b)) = cam.segment(w[0], w[1]) {
                line(g, batch, a, b, 1.0, ACCENT);
            }
        }
    }
    // Scanned blocks
    let storage = s.on("StorageESP");
    let listed = s.on("BlockESP");
    let ores = s.on("Xray");
    if storage || listed || ores {
        for hits in rt.found.values() {
            for (p, k) in hits {
                let (color, tracer) = if listed && k & LISTED != 0 {
                    (0xFF_00_FF_FF, s.b("BlockESP", "Tracers"))
                } else if ores && k & ORE != 0 {
                    (ore_color(&core::block_name(g, *p)), false)
                } else if storage && k & STORAGE != 0 {
                    (0xFF_FF_A5_00, s.b("StorageESP", "Tracers"))
                } else {
                    continue;
                };
                let min = p.map(f64::from);
                let max = [min[0] + 1.0, min[1] + 1.0, min[2] + 1.0];
                if core::dist(eye, [min[0] + 0.5, min[1] + 0.5, min[2] + 0.5]) > 96.0 {
                    continue;
                }
                cam.box_edges(g, batch, min, max, color);
                if tracer
                    && let Some(at) = cam.project([min[0] + 0.5, min[1] + 0.5, min[2] + 0.5])
                {
                    line(g, batch, centre, at, 1.0, color & 0x90_FF_FF_FF);
                }
            }
        }
    }
    // HoleESP
    for h in &rt.holes {
        let min = h.map(f64::from);
        let bedrock = [[1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1], [0, -1, 0]]
            .iter()
            .all(|d: &[i32; 3]| &*core::block_name(g, [h[0] + d[0], h[1] + d[1], h[2] + d[2]]) == "minecraft:bedrock");
        let color = if bedrock { 0xFF_00_FF_00 } else { 0xFF_FF_33_33 };
        cam.box_edges(g, batch, min, [min[0] + 1.0, min[1] + 0.05, min[2] + 1.0], color);
    }
    // Entities: ESP, Tracers, Nametags
    let esp = s.on("ESP");
    let tracers = s.on("Tracers");
    let tags = s.on("Nametags");
    if esp || tracers || tags {
        for m in g.mobs.iter().filter(|m| m.alive() && Some(m.id) != g.riding) {
            let name = names.get(&m.id);
            let (color, kind) = match name {
                Some(n) if s.is_friend(n) => (FRIEND, "Players"),
                Some(_) => (PLAYER, "Players"),
                None if m.kind.category == "monster" => (HOSTILE, "Hostiles"),
                None => (ANIMAL, "Animals"),
            };
            let half = m.kind.width / 2.0;
            let p = m.position;
            let h = f64::from(m.height());
            let min = [p.x - half, p.y, p.z - half];
            let max = [p.x + half, p.y + h, p.z + half];
            if esp && s.b("ESP", kind) {
                let d = core::dist(eye, [p.x, p.y + h / 2.0, p.z]);
                if d <= s.n("ESP", "MaxRange") {
                    let frac = (m.health / m.kind.max_health.max(1.0)).clamp(0.0, 1.0);
                    let health_color = if frac > 0.5 { 0xFF_55_FF_55 } else if frac > 0.25 { 0xFF_FF_FF_55 } else { 0xFF_FF_55_55 };
                    let base = match s.c("ESP", "Color") {
                        "Health" => health_color,
                        "Theme" => 0xFF00_0000 | core::accent(s),
                        _ => color | 0xFF_00_00_00,
                    };
                    // Far things fade, so a crowd doesn't hide what's near.
                    let alpha = if s.b("ESP", "Fade") { (1.0 - d / s.n("ESP", "MaxRange").max(1.0) * 0.7).clamp(0.3, 1.0) } else { 1.0 };
                    let tint = (base & 0xFF_FF_FF) | (((alpha * 255.0) as u32) << 24);
                    if s.c("ESP", "Style") == "3D" {
                        cam.box_edges(g, batch, min, max, tint);
                    }
                    if let Some([x, y, w, hh]) = cam.rect(min, max) {
                        if s.b("ESP", "Fill") {
                            fill(g, batch, x, y, w, hh, (base & 0xFF_FF_FF) | 0x30_00_00_00);
                        }
                        match s.c("ESP", "Style") {
                            "Outline" => {
                                rect_outline(g, batch, x - 1.0, y - 1.0, w + 2.0, hh + 2.0, 0xC0_00_00_00);
                                rect_outline(g, batch, x, y, w, hh, tint);
                            }
                            "3D" => {}
                            _ => {
                                brackets(g, batch, x - 1.0, y - 1.0, w + 2.0, hh + 2.0, 0xC0_00_00_00);
                                brackets(g, batch, x, y, w, hh, tint);
                            }
                        }
                        if s.b("ESP", "HealthBar") {
                            fill(g, batch, x - 4.0, y, 2.0, hh, 0xA0_00_00_00);
                            fill(g, batch, x - 4.0, y + hh * (1.0 - frac), 2.0, hh * frac, health_color);
                        }
                        let mut label = String::new();
                        if s.b("ESP", "Names") {
                            label = name.cloned().unwrap_or_else(|| core::pretty(&m.kind.id));
                        }
                        if s.b("ESP", "Distance") {
                            if !label.is_empty() {
                                label.push_str("  ");
                            }
                            label.push_str(&format!("{d:.0}m"));
                        }
                        if !label.is_empty() {
                            let tw = g.native_text_width(&label) * 0.75;
                            core::text_k(g, batch, &label, x + w / 2.0 - tw / 2.0, y + hh + 1.5, 0xFF_FF_FF, 0.75);
                        }
                    }
                }
            }
            if tracers
                && s.b("Tracers", kind)
                && let Some(at) = cam.project([p.x, p.y + h / 2.0, p.z])
            {
                line(g, batch, centre, at, 1.0, color);
            }
            let show_tag = tags && if name.is_some() { s.b("Nametags", "Players") } else { s.b("Nametags", "Mobs") };
            if show_tag && let Some((x, y)) = cam.project([p.x, p.y + h + 0.3, p.z]) {
                let mut label = name.cloned().unwrap_or_else(|| core::pretty(&m.kind.id));
                if s.b("Nametags", "Health") {
                    label.push_str(&format!(" {:.1}", m.health));
                }
                if s.b("Nametags", "Distance") {
                    label.push_str(&format!(" {:.0}m", core::dist(eye, [p.x, p.y, p.z])));
                }
                let w = g.native_text_width(&label);
                fill(g, batch, x - w / 2.0 - 2.0, y - 10.0, w + 4.0, 10.0, 0x90_00_00_00);
                text(g, batch, &label, x - w / 2.0, y - 9.0, color & 0xFF_FF_FF);
            }
        }
        for item in &g.dropped {
            let p = item.position;
            let min = [p.x - 0.125, p.y, p.z - 0.125];
            let max = [p.x + 0.125, p.y + 0.25, p.z + 0.125];
            if esp && s.b("ESP", "Items") {
                cam.box_edges(g, batch, min, max, ITEM);
            }
            if tracers && s.b("Tracers", "Items") && let Some(at) = cam.project([p.x, p.y + 0.1, p.z]) {
                line(g, batch, centre, at, 1.0, ITEM);
            }
            if tags && s.b("Nametags", "Items") && let Some((x, y)) = cam.project([p.x, p.y + 0.5, p.z]) {
                let label = format!("{} x{}", core::pretty(&item.stack.item), item.stack.count);
                let w = g.native_text_width(&label);
                text(g, batch, &label, x - w / 2.0, y - 9.0, 0xE0_E0_E0);
            }
        }
    }
    // LogoutSpots
    for (name, min, max) in &rt.logouts {
        cam.box_edges(g, batch, *min, *max, 0xFF_FF_FF_55);
        if let Some((x, y)) = cam.project([(min[0] + max[0]) / 2.0, max[1] + 0.3, (min[2] + max[2]) / 2.0]) {
            let label = format!("{name} (logged out)");
            let w = g.native_text_width(&label);
            text(g, batch, &label, x - w / 2.0, y - 9.0, 0xFF_FF_55);
        }
    }
    // Waypoints
    if s.on("Waypoints") {
        let dim = g.dim.id();
        for w in s.waypoints.iter().filter(|w| w.dim == dim) {
            let at = [w.pos[0], w.pos[1], w.pos[2]];
            if let Some((x, y)) = cam.project(at) {
                let mut label = w.name.clone();
                if s.b("Waypoints", "Distance") {
                    label.push_str(&format!(" ({:.0}m)", core::dist(eye, at)));
                }
                let tw = g.native_text_width(&label);
                fill(g, batch, x - 2.0, y - 2.0, 4.0, 4.0, ACCENT);
                fill(g, batch, x - tw / 2.0 - 2.0, y - 14.0, tw + 4.0, 10.0, 0x90_00_00_00);
                text(g, batch, &label, x - tw / 2.0, y - 13.0, 0xFF_FF_FF);
            }
        }
    }
    // BlockSelection
    if s.on("BlockSelection")
        && let Some(hit) = g.pick()
    {
        for b in g.outline_boxes(hit.pos) {
            cam.box_edges(g, batch, [b[0], b[1], b[2]], [b[3], b[4], b[5]], ACCENT);
        }
    }
    // BreakIndicators
    if s.on("BreakIndicators") {
        let mut breaking: Vec<([i32; 3], f32)> = g
            .destroying
            .values()
            .map(|(pos, stage, _)| (*pos, f32::from(*stage + 1) / 10.0))
            .collect();
        if let Some(m) = g.mining.as_ref() {
            breaking.push((m.pos, m.progress));
        }
        for (pos, progress) in breaking {
            let c = pos.map(|v| f64::from(v) + 0.5);
            if let Some((x, y)) = cam.project(c) {
                let label = format!("{:.0}%", (progress * 100.0).clamp(0.0, 100.0));
                let w = g.native_text_width(&label);
                text(g, batch, &label, x - w / 2.0, y - 4.0, 0xFF_FF_FF);
            }
        }
    }
    // Trajectories
    if s.on("Trajectories") {
        trajectory(g, batch, cam);
    }
    crate::game::asteroid_pvp::predictions(g, s, batch, cam);
    crate::game::asteroid_base::esp(g, s, batch, cam);
}

fn ore_color(name: &str) -> u32 {
    let n = name.trim_start_matches("minecraft:").trim_start_matches("deepslate_");
    match n {
        "diamond_ore" => 0xFF_55_FF_FF,
        "emerald_ore" => 0xFF_55_FF_55,
        "gold_ore" | "nether_gold_ore" => 0xFF_FF_D7_00,
        "iron_ore" => 0xFF_D8_AF_93,
        "redstone_ore" => 0xFF_FF_33_33,
        "lapis_ore" => 0xFF_33_55_FF,
        "ancient_debris" => 0xFF_A0_52_2D,
        "copper_ore" => 0xFF_E0_7B_39,
        "coal_ore" => 0xFF_40_40_40,
        _ => 0xFF_FF_FF_FF,
    }
}

fn trajectory(g: &Game<'_>, batch: &mut HudBatch, cam: &core::Cam) {
    let Some(item) = g.held().map(|s| s.item.clone()) else {
        return;
    };
    let (speed, gravity, drag) = match &*item {
        "minecraft:bow" => {
            let f = g.using.map_or(1.0, |(_, t)| f64::from(t) / 20.0);
            (((f * f + 2.0 * f) / 3.0).clamp(0.1, 1.0) * 3.0, 0.05, 0.99)
        }
        "minecraft:crossbow" => (3.15, 0.05, 0.99),
        "minecraft:trident" => (2.5, 0.05, 0.99),
        "minecraft:snowball" | "minecraft:egg" | "minecraft:ender_pearl" => (1.5, 0.03, 0.99),
        "minecraft:experience_bottle" => (0.7, 0.07, 0.99),
        i if i.ends_with("splash_potion") || i.ends_with("lingering_potion") => (0.5, 0.05, 0.99),
        _ => return,
    };
    let Some(eye) = g.eye_position() else {
        return;
    };
    let dir = crate::game::view_vector(g.yaw, g.pitch);
    let mut pos = [eye[0], eye[1] - 0.1, eye[2]];
    let mut vel = dir.map(|d| d * speed);
    let mut last = pos;
    for _ in 0..300 {
        pos = [pos[0] + vel[0], pos[1] + vel[1], pos[2] + vel[2]];
        vel = [vel[0] * drag, vel[1] * drag - gravity, vel[2] * drag];
        let block = pos.map(|v| v.floor() as i32);
        if let Some((a, b)) = cam.segment(last, pos) {
            line(g, batch, a, b, 1.0, 0xFF_FF_FF_FF);
        }
        if core::solid(g, block) || pos[1] < -64.0 {
            let min = block.map(f64::from);
            cam.box_edges(g, batch, min, [min[0] + 1.0, min[1] + 1.0, min[2] + 1.0], 0xFF_FF_33_33);
            break;
        }
        last = pos;
    }
}

/// One rounded-looking chip of the modern HUD.
fn chip(g: &Game<'_>, batch: &mut HudBatch, x: f32, y: f32, w: f32, h: f32) {
    fill(g, batch, x + 1.0, y, w - 2.0, h, 0xB8_10_0A_14);
    fill(g, batch, x, y + 1.0, 1.0, h - 2.0, 0xB8_10_0A_14);
    fill(g, batch, x + w - 1.0, y + 1.0, 1.0, h - 2.0, 0xB8_10_0A_14);
}

/// The modern HUD, top left: stat chips, key binds, potions and resources.
fn hud_modern(g: &Game<'_>, s: &State, batch: &mut HudBatch) {
    let k = 0.75;
    let accent = core::accent(s);
    let dim = 0xA0_9A_A8;
    let mut x = 3.0;
    let y = 3.0;
    if s.b("HUD", "Pills") {
        let t = g.world.day_time.rem_euclid(24000);
        let clock = format!("{:02}:{:02}", (t / 1000 + 6) % 24, (t % 1000) * 60 / 1000);
        let mut chips: Vec<(&str, String)> = vec![("✦", String::new())];
        if s.b("HUD", "Time") {
            chips.push(("◷", clock));
        }
        if s.b("HUD", "FPS") {
            chips.push(("FPS", format!("{}", g.native_frame_rate().0)));
        }
        if let Some(ms) = crate::game::remote::asteroid_tab::own_ping(g) {
            chips.push(("MS", format!("{ms}")));
        }
        if s.b("HUD", "Speed") {
            chips.push(("B/S", format!("{:.1}", s.speed)));
        }
        for (icon, value) in chips {
            let iw = g.native_text_width(icon) * k;
            let vw = g.native_text_width(&value) * k;
            let w = iw + vw + if value.is_empty() { 8.0 } else { 11.0 };
            chip(g, batch, x, y, w, 11.0);
            core::text_k(g, batch, icon, x + 4.0, y + 3.0, accent, k);
            if !value.is_empty() {
                core::text_k(g, batch, &value, x + 7.0 + iw, y + 3.0, 0xFF_FF_FF, k);
            }
            x += w + 2.0;
        }
    }
    // The panels under them, in a column.
    let mut py = y + 14.0;
    let panel = |g: &Game<'_>, batch: &mut HudBatch, py: &mut f32, title: &str, rows: &[(String, String, u32)]| {
        let w = 84.0;
        let h = 11.0 + rows.len() as f32 * 7.5;
        chip(g, batch, 3.0, *py, w, h);
        core::text_k(g, batch, title, 7.0, *py + 3.0, 0xFF_FF_FF, k);
        let n = g.native_text_width(&rows.len().to_string()) * k;
        core::text_k(g, batch, &rows.len().to_string(), 3.0 + w - n - 4.0, *py + 3.0, dim, k);
        for (i, (a, b, c)) in rows.iter().enumerate() {
            let ry = *py + 11.0 + i as f32 * 7.5;
            core::text_k(g, batch, &core::clip_text(g, a, 74.0), 7.0, ry, dim, k);
            let bw = g.native_text_width(b) * k;
            core::text_k(g, batch, b, 3.0 + w - bw - 4.0, ry, *c, k);
        }
        *py += h + 2.0;
    };
    if s.b("HUD", "Keybinds") {
        let rows: Vec<(String, String, u32)> = s
            .modules
            .iter()
            .filter(|m| !m.bind.is_empty())
            .take(8)
            .map(|m| (m.name.to_owned(), core::key_label(&m.bind), if m.on { accent } else { dim }))
            .collect();
        if !rows.is_empty() {
            panel(g, batch, &mut py, "Keybinds", &rows);
        }
    }
    if s.b("HUD", "Potions") {
        let rows: Vec<(String, String, u32)> = g
            .potions
            .effects
            .list
            .iter()
            .map(|e| {
                let secs = e.duration.max(0) / 20;
                let level = if e.amplifier > 0 { format!(" {}", e.amplifier + 1) } else { String::new() };
                (format!("{}{level}", core::pretty(e.id)), format!("{}:{:02}", secs / 60, secs % 60), 0xFF_FF_FF)
            })
            .collect();
        if !rows.is_empty() {
            panel(g, batch, &mut py, "Potions", &rows);
        }
    }
    if s.b("HUD", "Resources") {
        let count = |name: &str, items: &[&str]| {
            let n = core::count(g, |i| items.contains(&i));
            (name.to_owned(), n.to_string(), if n > 0 { 0xFF_FF_FF } else { 0xFF_55_55 })
        };
        let rows = [
            count("Gapples", &["minecraft:golden_apple", "minecraft:enchanted_golden_apple"]),
            count("Totems", &["minecraft:totem_of_undying"]),
            count("Crystals", &["minecraft:end_crystal"]),
            count("Pearls", &["minecraft:ender_pearl"]),
            count("Wind charges", &["minecraft:wind_charge"]),
            count("Exp", &["minecraft:experience_bottle"]),
        ];
        panel(g, batch, &mut py, "Resources", &rows);
    }
    core::LEFT_Y.with(|c| c.set(py));
}

fn hud_text(g: &Game<'_>, s: &State, batch: &mut HudBatch, corners: &mut Corners) {
    let ticks = s.ticks;
    let modern = s.c("Theme", "HudStyle") != "Classic";
    if modern {
        hud_modern(g, s, batch);
    }
    let rainbow = s.b("HUD", "Rainbow");
    // Watermark and welcome, top left.
    if !modern && (s.b("HUD", "Watermark") || s.b("HUD", "Welcome")) && g.native_corner_shown(Corner::TopLeft) {
        let mut lines: Vec<(String, u32)> = Vec::new();
        if s.b("HUD", "Watermark") {
            lines.push((format!("{} {}", core::NAME, core::VERSION), 0x91_3D_E8));
        }
        if s.b("HUD", "Welcome") {
            let name = core::shown_name().unwrap_or_else(|| {
                g.remote.as_ref().map_or_else(|| "player".to_owned(), |r| r.profile_name.clone())
            });
            lines.push((format!("Welcome to {}, {name}!", core::NAME), 0xE0_E0_E0));
        }
        let w = lines.iter().map(|l| g.native_text_width(&l.0)).fold(0.0, f32::max);
        let (x, y) = corners.place(Corner::TopLeft, w, lines.len() as f32 * 10.0);
        for (i, (l, c)) in lines.iter().enumerate() {
            let color = if i == 0 && rainbow { core::rainbow(0.0, ticks) } else { *c };
            text(g, batch, l, x, y + i as f32 * 10.0, color);
        }
    }
    // The module list, top right.
    if s.b("HUD", "ArrayList") {
        let mut on: Vec<&str> = s
            .modules
            .iter()
            .filter(|m| m.on && m.visible && m.name != "HUD")
            .map(|m| m.name)
            .collect();
        on.sort_by(|a, b| g.native_text_width(b).total_cmp(&g.native_text_width(a)));
        if !on.is_empty() {
            let w = on.iter().map(|n| g.native_text_width(n)).fold(0.0, f32::max) + 6.0;
            let (_, y) = corners.place(Corner::TopRight, w, on.len() as f32 * 10.0);
            let right = batch.width - 2.0;
            for (i, name) in on.iter().enumerate() {
                let tw = g.native_text_width(name);
                let ly = y + i as f32 * 10.0;
                let color = if rainbow { core::rainbow(i as f64 * 0.05, ticks) } else { core::accent(s) };
                fill(g, batch, right - tw - 4.0, ly - 1.0, tw + 4.0, 10.0, 0x70_00_00_00);
                fill(g, batch, right, ly - 1.0, 1.0, 10.0, 0xFF_00_00_00 | color);
                text(g, batch, name, right - tw - 2.0, ly, color);
            }
        }
    }
    // Info lines, bottom right.
    let mut info: Vec<(String, u32)> = Vec::new();
    let white = 0xE0_E0_E0;
    if let Some(line) = crate::game::asteroid_baritone::status() {
        info.push((line, 0x55_FF_88));
    }
    if let Some(p) = g.player.as_ref() {
        if !modern && s.b("HUD", "Potions") {
            for e in &g.potions.effects.list {
                let secs = e.duration.max(0) / 20;
                let name = core::pretty(e.id);
                let level = if e.amplifier > 0 { format!(" {}", e.amplifier + 1) } else { String::new() };
                info.push((format!("{name}{level} {}:{:02}", secs / 60, secs % 60), 0xC9_A0_FF));
            }
        }
        if s.b("HUD", "Armor") {
            let parts: Vec<String> = g
                .armor
                .iter()
                .flatten()
                .map(|st| {
                    let max = g.res.items().get(&st.item).map_or(0, |i| i.max_damage);
                    if max == 0 {
                        "-".to_owned()
                    } else {
                        format!("{}%", (max - st.damage.min(max)) * 100 / max)
                    }
                })
                .collect();
            if !parts.is_empty() {
                info.push((format!("Armor {}", parts.join(" ")), white));
            }
        }
        if !modern && s.b("HUD", "Totems") {
            let n = core::count(g, |i| i == "minecraft:totem_of_undying");
            if n > 0 {
                info.push((format!("Totems {n}"), 0xFF_D7_00));
            }
        }
        if !modern && s.b("HUD", "Time") {
            let t = g.world.day_time.rem_euclid(24000);
            let hours = (t / 1000 + 6) % 24;
            let minutes = (t % 1000) * 60 / 1000;
            info.push((format!("Time {hours:02}:{minutes:02}"), white));
        }
        if !modern && s.b("HUD", "FPS") {
            info.push((format!("FPS {}", g.native_frame_rate().0), white));
        }
        if !modern && s.b("HUD", "Speed") {
            info.push((format!("Speed {:.1} b/s", s.speed), white));
        }
        if s.b("HUD", "Direction") {
            let yaw = core::wrap(g.yaw);
            let dir = match ((yaw + 180.0 + 22.5) / 45.0).floor() as i32 % 8 {
                0 => "North (-Z)",
                1 => "North East",
                2 => "East (+X)",
                3 => "South East",
                4 => "South (+Z)",
                5 => "South West",
                6 => "West (-X)",
                _ => "North West",
            };
            info.push((format!("Facing {dir}"), white));
        }
        if s.b("HUD", "Coords") {
            let (x, y, z) = (p.position.x, p.position.y, p.position.z);
            let other = match g.dim {
                crate::game::Dim::Nether => format!(" [{:.0}, {:.0}]", x * 8.0, z * 8.0),
                crate::game::Dim::Overworld => format!(" [{:.0}, {:.0}]", x / 8.0, z / 8.0),
                crate::game::Dim::End => String::new(),
            };
            info.push((format!("XYZ {x:.1} {y:.1} {z:.1}{other}"), white));
        }
    }
    if !info.is_empty() {
        let w = info.iter().map(|l| g.native_text_width(&l.0)).fold(0.0, f32::max);
        let (_, y) = corners.place(Corner::BottomRight, w, info.len() as f32 * 10.0);
        for (i, (l, c)) in info.iter().enumerate() {
            let tw = g.native_text_width(l);
            text(g, batch, l, batch.width - tw - 3.0, y + i as f32 * 10.0, *c);
        }
    }
    // The combat target, under the crosshair.
    if s.b("HUD", "Target")
        && let Some(id) = s.target
        && let Some(m) = g.mobs.iter().find(|m| m.id == id && m.alive())
    {
        let names = core::player_names(g);
        let name = names.get(&id).cloned().unwrap_or_else(|| core::pretty(&m.kind.id));
        let eye = g.eye_position().unwrap_or_default();
        let label = format!("{name}  {:.1} hp  {:.1}m", m.health, core::dist(eye, m.center()));
        let w = g.native_text_width(&label);
        let (x, y) = (batch.width / 2.0 - w / 2.0, batch.height / 2.0 + 14.0);
        fill(g, batch, x - 3.0, y - 2.0, w + 6.0, 12.0, 0x90_00_00_00);
        fill(g, batch, x - 3.0, y + 9.0, w + 6.0, 1.0, ACCENT);
        text(g, batch, &label, x, y, 0xFF_FF_FF);
    }
}
