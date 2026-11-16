// Asteroid Client's world modules: Nuker, AutoMine, Timer, LiquidFiller,
// AutoBreed, AutoShearer, EndermanLook and StashFinder.
use super::Game;
use crate::game::asteroid_core::{self as core, State, Waypoint};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
};

#[derive(Default)]
struct Rt {
    nuke: Option<[i32; 3]>,
    nuke_ticks: u32,
    fed: HashMap<u32, u64>,
    reported: HashSet<(i32, i32)>,
}
thread_local! {
    static RT: RefCell<Rt> = RefCell::new(Rt::default());
}

pub fn toggled(g: &mut Game<'_>, s: &mut State, name: &str, on: bool) {
    match (name, on) {
        ("Nuker" | "AutoMine", false) => {
            RT.with(|rt| rt.borrow_mut().nuke = None);
            core::hold(g, false, false);
        }
        ("Timer", false) => g.clock.set_rate(20.0),
        ("StashFinder", false) => RT.with(|rt| rt.borrow_mut().reported.clear()),
        _ => {}
    }
    let _ = s;
}

pub fn tick(g: &mut Game<'_>, s: &mut State) {
    if s.on("Timer") {
        g.clock.set_rate((20.0 * s.n("Timer", "Speed")) as f32);
    }
    RT.with(|rt| {
        let rt = &mut *rt.borrow_mut();
        stash_finder(s, rt);
        if !core::playing(g) {
            return;
        }
        if s.on("AutoMine") && !s.on("Nuker") {
            core::hold(g, false, true);
        }
        if nuker(g, s, rt) {
            return;
        }
        if liquid_filler(g, s) {
            return;
        }
        if feed(g, s, rt) {
            return;
        }
        enderman_look(g, s);
    });
}

fn breakable(g: &Game<'_>, pos: [i32; 3]) -> bool {
    let state = g.world.block(pos[0], pos[1], pos[2]);
    let f = g.res.facts();
    if f.is_air(state) {
        return false;
    }
    let facts = f.get(state);
    facts.hardness >= 0.0
        && facts.fluid == horizon_client::blocks::Fluid::None
        && !facts.outline.is_empty()
}

fn nuker(g: &mut Game<'_>, s: &mut State, rt: &mut Rt) -> bool {
    if !s.on("Nuker") {
        return false;
    }
    let Some(eye) = g.eye_position() else {
        return false;
    };
    let range = s.n("Nuker", "Range");
    let centre = |p: [i32; 3]| p.map(|v| f64::from(v) + 0.5);
    if let Some(pos) = rt.nuke {
        rt.nuke_ticks += 1;
        let gone = !breakable(g, pos) || core::dist(eye, centre(pos)) > range + 0.5 || rt.nuke_ticks > 20 * 30;
        if !gone {
            core::look_at(g, centre(pos));
            core::hold(g, false, true);
            return true;
        }
        rt.nuke = None;
    }
    let Some(feet) = core::feet(g) else {
        return false;
    };
    let mode = s.c("Nuker", "Mode");
    let filter = s.list("Nuker", "Filter");
    let r = range.ceil() as i32;
    let mut best: Option<([i32; 3], f64)> = None;
    for dx in -r..=r {
        for dy in -r..=r {
            for dz in -r..=r {
                let p = [feet[0] + dx, feet[1] + dy, feet[2] + dz];
                if mode == "Flatten" && p[1] < feet[1] {
                    continue;
                }
                if !breakable(g, p) {
                    continue;
                }
                if mode == "Filter" && !filter.iter().any(|f| *f == *core::block_name(g, p)) {
                    continue;
                }
                let d = core::dist(eye, centre(p));
                if d > range || best.is_some_and(|b| d >= b.1) {
                    continue;
                }
                // Visible: the ray to its centre hits it first.
                let dir = [0, 1, 2].map(|a| (centre(p)[a] - eye[a]) / d.max(1e-6));
                if g.clip_ray(eye, dir, d + 0.5).is_some_and(|h| h.pos == p) {
                    best = Some((p, d));
                }
            }
        }
    }
    match best {
        Some((p, _)) => {
            rt.nuke = Some(p);
            rt.nuke_ticks = 0;
            core::look_at(g, centre(p));
            core::hold(g, false, true);
            true
        }
        None => {
            core::hold(g, false, false);
            false
        }
    }
}

fn liquid_filler(g: &mut Game<'_>, s: &mut State) -> bool {
    if !s.on("LiquidFiller") {
        return false;
    }
    let Some(feet) = core::feet(g) else {
        return false;
    };
    let Some(eye) = g.eye_position() else {
        return false;
    };
    let range = s.n("LiquidFiller", "Range");
    let r = range.ceil() as i32;
    let mut spots = Vec::new();
    for dx in -r..=r {
        for dy in -r..=r {
            for dz in -r..=r {
                let p = [feet[0] + dx, feet[1] + dy, feet[2] + dz];
                let state = g.world.block(p[0], p[1], p[2]);
                let f = g.res.facts().get(state);
                if f.fluid == horizon_client::blocks::Fluid::None || f.fluid_falling || f.waterlogged {
                    continue;
                }
                // Not where the player stands.
                if p[0] == feet[0] && p[2] == feet[2] && (p[1] == feet[1] || p[1] == feet[1] + 1) {
                    continue;
                }
                let d = core::dist(eye, p.map(|v| f64::from(v) + 0.5));
                if d <= range {
                    spots.push((p, d));
                }
            }
        }
    }
    spots.sort_by(|a, b| a.1.total_cmp(&b.1));
    let Some(slot) = core::hotbar_find(g, |i| core::placeable(g, i, "Any solid")) else {
        return false;
    };
    for (p, _) in spots {
        if core::place(g, p, slot) {
            return true;
        }
    }
    false
}

/// AutoBreed and AutoShearer: right-click animals near the player.
fn feed(g: &mut Game<'_>, s: &mut State, rt: &mut Rt) -> bool {
    let breed = s.on("AutoBreed");
    let shear = s.on("AutoShearer");
    if !breed && !shear {
        return false;
    }
    let now = s.ticks;
    rt.fed.retain(|_, t| now < *t);
    let Some(eye) = g.eye_position() else {
        return false;
    };
    let shears = core::hotbar_find(g, |i| i == "minecraft:shears");
    let mut pick: Option<(u32, [f64; 3], bool)> = None;
    for m in g.mobs.iter().filter(|m| m.alive() && !m.is_baby() && !rt.fed.contains_key(&m.id)) {
        let c = m.center();
        let is_sheep = m.kind.id.ends_with("sheep");
        let want_shear = shear && is_sheep && shears.is_some() && core::dist(eye, c) <= s.n("AutoShearer", "Range");
        let want_breed = breed && m.kind.category == "creature" && core::dist(eye, c) <= s.n("AutoBreed", "Range");
        if want_shear {
            pick = Some((m.id, c, true));
            break;
        }
        if want_breed && pick.is_none() {
            pick = Some((m.id, c, false));
        }
    }
    let Some((id, c, shearing)) = pick else {
        return false;
    };
    if shearing {
        if let Some(slot) = shears {
            g.selected = slot;
        }
        rt.fed.insert(id, now + 20 * 60);
    } else {
        if g.held().is_none() {
            return false;
        }
        rt.fed.insert(id, now + 20 * 30);
    }
    core::look_at(g, c);
    core::click(g, true);
    true
}

fn enderman_look(g: &mut Game<'_>, s: &State) {
    if !s.on("EndermanLook") {
        return;
    }
    let Some(eye) = g.eye_position() else {
        return;
    };
    let view = crate::game::view_vector(g.yaw, g.pitch);
    for m in g.mobs.iter().filter(|m| m.alive() && m.kind.id.ends_with("enderman")) {
        let head = [m.position.x, m.position.y + m.kind.eye_height, m.position.z];
        let d = core::dist(eye, head);
        if d > 64.0 {
            continue;
        }
        let to = [0, 1, 2].map(|a| (head[a] - eye[a]) / d.max(1e-6));
        let cos = view[0] * to[0] + view[1] * to[1] + view[2] * to[2];
        if cos > 0.97 {
            g.pitch = (g.pitch + 35.0).min(90.0);
            return;
        }
    }
}

fn stash_finder(s: &mut State, rt: &mut Rt) {
    if !s.on("StashFinder") || !s.ticks.is_multiple_of(100) {
        return;
    }
    let min = s.n("StashFinder", "Minimum") as usize;
    for ((x, z), n) in crate::game::asteroid_render::stashes() {
        if n >= min && rt.reported.insert((x, z)) {
            let (bx, bz) = (x * 16 + 8, z * 16 + 8);
            s.say(format!("StashFinder: {n} containers in chunk {x}, {z} (around {bx}, {bz})"));
            let name = format!("stash_{x}_{z}");
            if !s.waypoints.iter().any(|w| w.name == name) {
                s.waypoints.push(Waypoint {
                    name,
                    pos: [f64::from(bx), 64.0, f64::from(bz)],
                    dim: "minecraft:overworld".to_owned(),
                });
                s.dirty = true;
            }
        }
    }
}
