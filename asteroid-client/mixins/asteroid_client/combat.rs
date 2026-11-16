// Asteroid Client's combat modules: KillAura, TriggerBot, AimAssist,
// BowAimbot, AutoTotem, Offhand, AutoArmor, AutoWeapon, Criticals, Velocity,
// AutoLog, Surround, SelfTrap, AutoTrap, HoleFiller, CrystalAura, BedAura and
// AutoCity. Hitboxes and the entity part of Reach live in the `mob_hit` hook.
use super::Game;
use crate::game::asteroid_core::{self as core, Kind, State, Target};
use std::{cell::RefCell, time::Duration};

#[derive(Default)]
struct Rt {
    swap_wait: u32,
    armor_wait: u32,
    crystal_wait: u32,
    bed_wait: u32,
    last_hurt: i64,
    /// AutoCity's block and whether it holds the attack button for it.
    city: Option<[i32; 3]>,
}
thread_local! {
    static RT: RefCell<Rt> = RefCell::new(Rt::default());
    static AIM_TARGET: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}

const TOTEM: &str = "minecraft:totem_of_undying";

pub fn toggled(g: &mut Game<'_>, s: &mut State, name: &str, on: bool) {
    match (name, on) {
        ("AutoCity", false) => {
            RT.with(|rt| rt.borrow_mut().city = None);
            core::hold(g, false, false);
        }
        ("Surround", true) if s.b("Surround", "Center") => {
            if let Some(p) = g.player.as_mut() {
                p.position.x = p.position.x.floor() + 0.5;
                p.position.z = p.position.z.floor() + 0.5;
                p.velocity.x = 0.0;
                p.velocity.z = 0.0;
            }
        }
        ("KillAura" | "TriggerBot" | "CrystalAura", false) => s.target = None,
        _ => {}
    }
}

pub fn tick(g: &mut Game<'_>, s: &mut State) {
    RT.with(|rt| {
        let rt = &mut *rt.borrow_mut();
        rt.swap_wait = rt.swap_wait.saturating_sub(1);
        velocity(g, s, rt);
        auto_log(g, s);
        totems(g, s, rt);
        auto_armor(g, s, rt);
        if !core::playing(g) {
            return;
        }
        // One click per tick reaches the server: the first module that acts wins.
        if kill_aura(g, s) || trigger_bot(g, s) || crystal_aura(g, s, rt) || bed_aura(g, s, rt) {
            return;
        }
        if surround(g, s) || self_trap(g, s) || auto_trap(g, s) || hole_filler(g, s) {
            return;
        }
        auto_city(g, s, rt);
    });
}

pub fn frame(g: &mut Game<'_>, s: &mut State, elapsed: Duration) {
    aim_assist(g, s, elapsed);
    bow_aimbot(g, s);
}

fn wanted(s: &State, module: &str) -> (bool, bool, bool) {
    (s.b(module, "Players"), s.b(module, "Hostiles"), s.b(module, "Animals"))
}

fn visible(g: &Game<'_>, t: &Target) -> bool {
    let Some(eye) = g.eye_position() else {
        return false;
    };
    let d = core::dist(eye, t.center);
    let dir = [0, 1, 2].map(|a| (t.center[a] - eye[a]) / d.max(1e-6));
    g.clip_ray(eye, dir, d).is_none()
}

/// Criticals: false while it waits for the fall that makes the hit critical.
fn crit_ready(g: &mut Game<'_>, s: &State) -> bool {
    if !s.on("Criticals") {
        return true;
    }
    let jump = if s.c("Criticals", "Mode") == "Jump" { 0.42 } else { 0.11 };
    let Some(p) = g.player.as_mut() else {
        return true;
    };
    if p.flying || p.fall_flying || p.swimming || g.riding.is_some() {
        return true;
    }
    if p.ground.on_ground {
        p.velocity.y = jump;
        return false;
    }
    p.velocity.y < -0.03
}

fn auto_weapon(g: &mut Game<'_>, s: &State) {
    if !s.on("AutoWeapon") {
        return;
    }
    let kind = s.c("AutoWeapon", "Weapon");
    let mut best: Option<(usize, f32)> = None;
    for i in 0..9 {
        let Some(stack) = g.inventory[i].as_ref() else {
            continue;
        };
        let ok = match kind {
            "Sword" => stack.item.ends_with("_sword"),
            "Axe" => stack.item.ends_with("_axe"),
            _ => true,
        };
        let damage = g.res.items().get(&stack.item).map_or(0.0, |i| i.attack_damage);
        if ok && damage > 0.0 && best.is_none_or(|b| damage > b.1) {
            best = Some((i, damage));
        }
    }
    if let Some((i, _)) = best {
        g.selected = i;
    }
}

fn kill_aura(g: &mut Game<'_>, s: &mut State) -> bool {
    if !s.on("KillAura") || g.using.is_some() {
        return false;
    }
    let (p, h, a) = wanted(s, "KillAura");
    let mut list = core::targets(g, s, s.n("KillAura", "Range"), p, h, a);
    if !s.b("KillAura", "ThroughWalls") {
        list.retain(|t| visible(g, t));
    }
    match s.c("KillAura", "Priority") {
        "Health" => list.sort_by(|a, b| a.health.total_cmp(&b.health)),
        "Angle" => {
            let eye = g.eye_position().unwrap_or_default();
            let yaw = g.yaw;
            let off = |t: &Target| core::wrap(core::angles(eye, t.center).0 - yaw).abs();
            list.sort_by(|a, b| off(a).total_cmp(&off(b)));
        }
        _ => {}
    }
    let Some(t) = list.into_iter().next() else {
        if s.target.is_some() && !s.on("TriggerBot") {
            s.target = None;
        }
        return false;
    };
    s.target = Some(t.id);
    if s.b("KillAura", "Rotate") {
        core::look_at(g, t.center);
    }
    if s.b("KillAura", "WaitCooldown") && !core::charged(g) {
        return false;
    }
    if !crit_ready(g, s) {
        return false;
    }
    auto_weapon(g, s);
    // The attack goes to what the crosshair is on.
    core::look_at(g, t.center);
    crate::game::asteroid_pvp::attribute_swap(g, s);
    core::click(g, false);
    true
}

fn trigger_bot(g: &mut Game<'_>, s: &mut State) -> bool {
    if !s.on("TriggerBot") || g.using.is_some() {
        return false;
    }
    let Some((i, _)) = g.mob_hit() else {
        return false;
    };
    let (p, h, a) = wanted(s, "TriggerBot");
    let id = g.mobs[i].id;
    let ok = core::targets(g, s, 8.0, p, h, a).iter().any(|t| t.id == id);
    if !ok {
        return false;
    }
    s.target = Some(id);
    if s.b("TriggerBot", "WaitCooldown") && !core::charged(g) {
        return false;
    }
    if !crit_ready(g, s) {
        return false;
    }
    auto_weapon(g, s);
    core::click(g, false);
    true
}

/// AimAssist's point on a target: where it is headed, on the part chosen.
fn aim_point(g: &Game<'_>, s: &State, t: &Target, eye: [f64; 3]) -> [f64; 3] {
    let mut at = match s.c("AimAssist", "Aim") {
        "Head" => [t.center[0], t.max[1] - 0.2, t.center[2]],
        "Chest" => t.center,
        "Feet" => [t.center[0], t.min[1] + 0.2, t.center[2]],
        // The nearest point of the box: the view moves as little as it can.
        _ => {
            let (yaw, pitch) = (f64::from(g.yaw).to_radians(), f64::from(g.pitch).to_radians());
            let dir = [-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos()];
            let reach = core::dist(eye, t.center);
            let along = [0, 1, 2].map(|a| eye[a] + dir[a] * reach);
            [0, 1, 2].map(|a| along[a].clamp(t.min[a] + 0.05, t.max[a] - 0.05))
        }
    };
    if s.b("AimAssist", "Predict") {
        let v = g.mobs[t.index].velocity;
        at = [at[0] + v.x * 2.0, at[1], at[2] + v.z * 2.0];
    }
    at
}

fn aim_assist(g: &mut Game<'_>, s: &State, elapsed: Duration) {
    if !s.on("AimAssist") || !core::playing(g) {
        AIM_TARGET.with(|c| c.set(None));
        return;
    }
    if s.b("AimAssist", "OnlyWhenAttacking") && !g.left {
        return;
    }
    let Some(eye) = g.eye_position() else {
        return;
    };
    let (p, h, a) = wanted(s, "AimAssist");
    let mut list = core::targets(g, s, s.n("AimAssist", "Range"), p, h, a);
    if s.b("AimAssist", "OnlyVisible") {
        list.retain(|t| visible(g, t));
    }
    let (yaw, pitch) = (g.yaw, g.pitch);
    let off = |t: &Target| {
        let (y, pi) = core::angles(eye, t.center);
        let (dy, dp) = (core::wrap(y - yaw), pi - pitch);
        (dy * dy + dp * dp).sqrt()
    };
    let fov = s.n("AimAssist", "FOV") as f32;
    list.retain(|t| off(t) <= fov);
    match s.c("AimAssist", "Priority") {
        "Health" => list.sort_by(|a, b| a.health.total_cmp(&b.health)),
        "Distance" => {}
        _ => list.sort_by(|a, b| off(a).total_cmp(&off(b))),
    }
    // It stays on the target it has, so it doesn't flick between two close ones.
    let keep = AIM_TARGET.with(|c| c.get());
    let chosen = list.iter().find(|t| Some(t.id) == keep).or(list.first());
    let Some(t) = chosen else {
        AIM_TARGET.with(|c| c.set(None));
        return;
    };
    AIM_TARGET.with(|c| c.set(Some(t.id)));
    // Already on it: your own aim is left alone.
    if g.mob_hit().is_some_and(|(i, _)| i == t.index) {
        return;
    }
    let (want_yaw, want_pitch) = core::angles(eye, aim_point(g, s, t, eye));
    // Eases in: a share of what is left each moment, whatever the frame rate.
    let share = 1.0 - (-s.n("AimAssist", "Speed") * elapsed.as_secs_f64() * 2.0).exp();
    let dy = core::wrap(want_yaw - g.yaw) * share as f32;
    let dp = (want_pitch - g.pitch) * share as f32;
    g.yaw += dy;
    g.pitch = (g.pitch + dp).clamp(-90.0, 90.0);
}

fn bow_aimbot(g: &mut Game<'_>, s: &State) {
    if !s.on("BowAimbot") || !core::playing(g) {
        return;
    }
    let Some((slot, ticks)) = g.using else {
        return;
    };
    let item = g.inventory.get(slot).and_then(|s| s.as_ref()).map(|s| s.item.clone());
    let item = item.or_else(|| g.offhand.as_ref().map(|s| s.item.clone()));
    let Some(item) = item else {
        return;
    };
    let speed = match &*item {
        "minecraft:bow" => {
            let f = ticks as f64 / 20.0;
            ((f * f + 2.0 * f) / 3.0).min(1.0) * 3.0
        }
        "minecraft:crossbow" => 3.15,
        "minecraft:trident" => 2.5,
        _ => return,
    };
    if speed < 0.1 {
        return;
    }
    let (p, h, a) = wanted(s, "BowAimbot");
    let Some(t) = core::targets(g, s, s.n("BowAimbot", "Range"), p, h, a).into_iter().next() else {
        return;
    };
    let Some(eye) = g.eye_position() else {
        return;
    };
    // Lead the target by its velocity over the flight time.
    let v = g.mobs[t.index].velocity;
    let time = t.distance / speed;
    let at = [t.center[0] + v.x * time, t.center[1], t.center[2] + v.z * time];
    let dx = ((at[0] - eye[0]).powi(2) + (at[2] - eye[2]).powi(2)).sqrt();
    let dy = at[1] - eye[1];
    let grav = if &*item == "minecraft:trident" { 0.05 } else { 0.05 };
    let v2 = speed * speed;
    let root = v2 * v2 - grav * (grav * dx * dx + 2.0 * dy * v2);
    if root < 0.0 {
        return;
    }
    let pitch = -((v2 - root.sqrt()) / (grav * dx)).atan().to_degrees();
    let (yaw, _) = core::angles(eye, at);
    g.yaw += core::wrap(yaw - g.yaw);
    g.pitch = (pitch as f32).clamp(-90.0, 90.0);
}

fn velocity(g: &mut Game<'_>, s: &State, rt: &mut Rt) {
    let Some(p) = g.player.as_mut() else {
        return;
    };
    let hurt = p.health.hurt_animation_time as i64;
    if s.on("Velocity") && hurt > rt.last_hurt {
        let h = s.n("Velocity", "Horizontal") / 100.0;
        let v = s.n("Velocity", "Vertical") / 100.0;
        p.velocity.x *= h;
        p.velocity.z *= h;
        if p.velocity.y > 0.0 {
            p.velocity.y *= v;
        }
    }
    rt.last_hurt = hurt;
}

fn auto_log(g: &mut Game<'_>, s: &mut State) {
    if !s.on("AutoLog") || g.remote.is_none() {
        return;
    }
    let Some(p) = g.player.as_ref() else {
        return;
    };
    let health = p.health.health();
    if f64::from(health) > s.n("AutoLog", "Health") || health <= 0.0 {
        return;
    }
    if s.b("AutoLog", "OnlyNoTotems") && core::count(g, |i| i == TOTEM) > 0 {
        return;
    }
    if s.b("AutoLog", "ToggleOff") {
        s.turn_off("AutoLog");
    }
    let _ = g.leave_server(Some(format!("[Asteroid] AutoLog: health was {health:.1}")));
}

/// AutoTotem and Offhand.
fn totems(g: &mut Game<'_>, s: &State, rt: &mut Rt) {
    if rt.swap_wait > 0 || !core::inventory_menu(g) {
        return;
    }
    let Some(health) = g.player.as_ref().map(|p| p.health.health() + p.health.absorption()) else {
        return;
    };
    let want: Option<&[&str]> = if s.on("AutoTotem") {
        let smart = s.c("AutoTotem", "Mode") == "Smart";
        (!smart || f64::from(health) <= s.n("AutoTotem", "Health")).then_some(&[TOTEM][..])
    } else {
        None
    };
    let want = want.or_else(|| {
        if !s.on("Offhand") {
            return None;
        }
        if f64::from(health) <= s.n("Offhand", "TotemHealth") && core::count(g, |i| i == TOTEM) > 0 {
            return Some(&[TOTEM][..]);
        }
        Some(match s.c("Offhand", "Item") {
            "Golden Apple" => &["minecraft:enchanted_golden_apple", "minecraft:golden_apple"][..],
            "Shield" => &["minecraft:shield"][..],
            "End Crystal" => &["minecraft:end_crystal"][..],
            _ => &[TOTEM][..],
        })
    });
    let Some(want) = want else {
        return;
    };
    if g.offhand.as_ref().is_some_and(|o| want.contains(&&*o.item)) {
        return;
    }
    for item in want {
        if let Some(slot) = core::inventory_find(g, |i| i == *item) {
            core::swap_slot(g, slot, 40);
            rt.swap_wait = 3;
            return;
        }
    }
}

const ARMOR: [&str; 4] = ["head", "chest", "legs", "feet"];

fn armor_score(g: &Game<'_>, item: &str, slot: usize, s: &State) -> f64 {
    if item == "minecraft:elytra" {
        return if slot == 1 && s.b("AutoArmor", "PreferElytra") { 1000.0 } else { 0.5 };
    }
    let Some(info) = g.res.items().get(item) else {
        return 0.0;
    };
    info.modifiers
        .iter()
        .map(|(attr, _, amount, _, _)| match attr.trim_start_matches("minecraft:") {
            "armor" => *amount,
            "armor_toughness" => amount * 0.5,
            "knockback_resistance" => amount * 2.0,
            _ => 0.0,
        })
        .sum::<f64>()
        + 0.01
}

fn auto_armor(g: &mut Game<'_>, s: &State, rt: &mut Rt) {
    if !s.on("AutoArmor") || !core::inventory_menu(g) {
        return;
    }
    if rt.armor_wait > 0 {
        rt.armor_wait -= 1;
        return;
    }
    for (k, part) in ARMOR.iter().enumerate() {
        let current = g.armor[k].as_ref().map_or(0.0, |st| armor_score(g, &st.item, k, s));
        let mut best: Option<(usize, f64)> = None;
        for i in 0..36 {
            let Some(st) = g.inventory[i].as_ref() else {
                continue;
            };
            let fits = st.item.as_ref() == "minecraft:elytra" && k == 1
                || g.res.items().get(&st.item).and_then(|i| i.equip_slot.as_deref()) == Some(*part);
            if !fits {
                continue;
            }
            let score = armor_score(g, &st.item, k, s);
            if score > current && best.is_none_or(|b| score > b.1) {
                best = Some((i, score));
            }
        }
        let Some((i, _)) = best else {
            continue;
        };
        rt.armor_wait = s.n("AutoArmor", "Delay") as u32;
        if g.remote.is_some() {
            // Take the old piece off first; the new one goes on next time.
            if g.armor[k].is_some() {
                core::quick_move(g, 500 + k);
            } else {
                core::quick_move(g, i);
            }
        } else {
            let new = g.inventory[i].take();
            g.inventory[i] = g.armor[k].take();
            g.armor[k] = new;
        }
        return;
    }
}

/// The hotbar slot of a block the placing modules use; says so when none.
fn block_slot(g: &Game<'_>, s: &mut State, module: &str) -> Option<usize> {
    let choice = s.c(module, "Block");
    let slot = core::hotbar_find(g, |i| core::placeable(g, i, choice));
    if slot.is_none() && s.ticks.is_multiple_of(100) {
        s.say(format!("{module}: no {} in your hotbar", choice.to_lowercase()));
    }
    slot
}

/// Places the first block of `positions` that is still missing.
fn place_first(g: &mut Game<'_>, s: &mut State, module: &str, positions: &[[i32; 3]]) -> bool {
    let missing: Vec<[i32; 3]> = positions.iter().copied().filter(|p| core::replaceable(g, *p)).collect();
    if missing.is_empty() {
        return false;
    }
    let Some(slot) = block_slot(g, s, module) else {
        return false;
    };
    let (yaw, pitch, keep) = (g.yaw, g.pitch, g.selected);
    for pos in missing {
        if core::place(g, pos, slot) {
            // The click goes out this tick; the view and the hand go back after it.
            core::restore_after_tick(yaw, pitch, keep);
            return true;
        }
    }
    false
}

const SIDES: [[i32; 3]; 4] = [[1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1]];
fn add(a: [i32; 3], b: [i32; 3]) -> [i32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn surround(g: &mut Game<'_>, s: &mut State) -> bool {
    if !s.on("Surround") {
        return false;
    }
    let Some(feet) = core::feet(g) else {
        return false;
    };
    if s.b("Surround", "DisableOnJump")
        && g.player.as_ref().is_some_and(|p| p.velocity.y > 0.25 && !p.ground.on_ground)
    {
        s.turn_off("Surround");
        return false;
    }
    let mut want = Vec::new();
    for d in SIDES {
        let p = add(feet, d);
        let below = add(p, [0, -1, 0]);
        if core::replaceable(g, below) {
            want.push(below);
        }
        want.push(p);
    }
    place_first(g, s, "Surround", &want)
}

fn trap_positions(feet: [i32; 3]) -> Vec<[i32; 3]> {
    let mut want = Vec::new();
    for d in SIDES {
        want.push(add(feet, d));
        want.push(add(feet, [d[0], 1, d[2]]));
    }
    want.push(add(feet, [1, 2, 0]));
    want.push(add(feet, [0, 2, 0]));
    want
}

fn self_trap(g: &mut Game<'_>, s: &mut State) -> bool {
    if !s.on("SelfTrap") {
        return false;
    }
    let Some(feet) = core::feet(g) else {
        return false;
    };
    let mut want = trap_positions(feet);
    // Not where the player stands.
    want.retain(|p| !(p[0] == feet[0] && p[2] == feet[2] && p[1] < feet[1] + 2));
    place_first(g, s, "SelfTrap", &want)
}

fn nearest_player(g: &Game<'_>, s: &State, range: f64) -> Option<Target> {
    core::targets(g, s, range, true, false, false).into_iter().find(|t| t.kind == Kind::Player)
}

fn target_feet(t: &Target) -> [i32; 3] {
    [
        ((t.min[0] + t.max[0]) / 2.0).floor() as i32,
        (t.min[1] + 0.2).floor() as i32,
        ((t.min[2] + t.max[2]) / 2.0).floor() as i32,
    ]
}

fn auto_trap(g: &mut Game<'_>, s: &mut State) -> bool {
    if !s.on("AutoTrap") {
        return false;
    }
    let Some(t) = nearest_player(g, s, s.n("AutoTrap", "Range") + 2.0) else {
        return false;
    };
    let want = trap_positions(target_feet(&t));
    place_first(g, s, "AutoTrap", &want)
}

/// A 1x1 hole: air with solid blocks on its four sides and below, and room above.
pub fn is_hole(g: &Game<'_>, p: [i32; 3]) -> bool {
    core::is_air(g, p)
        && core::is_air(g, add(p, [0, 1, 0]))
        && core::is_air(g, add(p, [0, 2, 0]))
        && core::solid(g, add(p, [0, -1, 0]))
        && SIDES.iter().all(|d| core::solid(g, add(p, *d)))
}

fn hole_filler(g: &mut Game<'_>, s: &mut State) -> bool {
    if !s.on("HoleFiller") {
        return false;
    }
    let Some(feet) = core::feet(g) else {
        return false;
    };
    let r = s.n("HoleFiller", "Range").ceil() as i32;
    let mut holes = Vec::new();
    for dx in -r..=r {
        for dy in -2..=1 {
            for dz in -r..=r {
                let p = add(feet, [dx, dy, dz]);
                if (dx, dz) != (0, 0) && is_hole(g, p) {
                    holes.push(p);
                }
            }
        }
    }
    place_first(g, s, "HoleFiller", &holes)
}

fn crystal_aura(g: &mut Game<'_>, s: &mut State, rt: &mut Rt) -> bool {
    if !s.on("CrystalAura") {
        return false;
    }
    if rt.crystal_wait > 0 {
        rt.crystal_wait -= 1;
        return false;
    }
    let range = s.n("CrystalAura", "Range");
    let Some(eye) = g.eye_position() else {
        return false;
    };
    let (p, h) = (s.b("CrystalAura", "Players"), s.b("CrystalAura", "Hostiles"));
    let Some(target) = core::targets(g, s, range + 6.0, p, h, false).into_iter().next() else {
        return false;
    };
    s.target = Some(target.id);
    let delay = s.n("CrystalAura", "Delay") as u32;
    // Break the crystal nearest the target.
    if s.b("CrystalAura", "Break") {
        let crystal = g
            .crystals
            .iter()
            .map(|c| [c.position.x, c.position.y + 1.0, c.position.z])
            .filter(|c| core::dist(eye, *c) <= range)
            .min_by(|a, b| core::dist(*a, target.center).total_cmp(&core::dist(*b, target.center)));
        if let Some(c) = crystal {
            core::look_at(g, c);
            core::click(g, false);
            rt.crystal_wait = delay;
            return true;
        }
    }
    if !s.b("CrystalAura", "Place") {
        return false;
    }
    let Some(slot) = core::hotbar_find(g, |i| i == "minecraft:end_crystal") else {
        return false;
    };
    let tf = target_feet(&target);
    let mut best: Option<([i32; 3], f64)> = None;
    for dx in -3..=3 {
        for dy in -2..=1 {
            for dz in -3..=3 {
                let base = add(tf, [dx, dy - 1, dz]);
                let name = core::block_name(g, base);
                if &*name != "minecraft:obsidian" && &*name != "minecraft:bedrock" {
                    continue;
                }
                let up = add(base, [0, 1, 0]);
                if !core::is_air(g, up) || !core::is_air(g, add(up, [0, 1, 0])) {
                    continue;
                }
                // Not inside the target or the player.
                let centre = [f64::from(up[0]) + 0.5, f64::from(up[1]), f64::from(up[2]) + 0.5];
                if (0..3).all(|a| centre[a] + 0.5 > target.min[a] && centre[a] - 0.5 < target.max[a]) {
                    continue;
                }
                let top = [f64::from(base[0]) + 0.5, f64::from(base[1]) + 1.0, f64::from(base[2]) + 0.5];
                if core::dist(eye, top) > range {
                    continue;
                }
                let d = core::dist(top, target.center);
                if d < 6.0 && best.is_none_or(|b| d < b.1) {
                    best = Some((base, d));
                }
            }
        }
    }
    let Some((base, _)) = best else {
        return false;
    };
    let top = [f64::from(base[0]) + 0.5, f64::from(base[1]) + 1.0, f64::from(base[2]) + 0.5];
    let (yaw, pitch) = core::angles(eye, top);
    let dir = crate::game::view_vector(yaw, pitch);
    if g.clip_ray(eye, dir, range + 1.0).is_none_or(|hit| hit.pos != base) {
        return false;
    }
    g.selected = slot;
    g.yaw += core::wrap(yaw - g.yaw);
    g.pitch = pitch;
    core::click(g, true);
    rt.crystal_wait = delay;
    true
}

fn bed_aura(g: &mut Game<'_>, s: &mut State, rt: &mut Rt) -> bool {
    if !s.on("BedAura") || g.dim == crate::game::Dim::Overworld {
        return false;
    }
    if rt.bed_wait > 0 {
        rt.bed_wait -= 1;
        return false;
    }
    let range = s.n("BedAura", "Range");
    let Some(t) = nearest_player(g, s, range + 2.0) else {
        return false;
    };
    let feet = target_feet(&t);
    let delay = s.n("BedAura", "Delay") as u32;
    // A bed already beside the target: blow it up.
    for d in SIDES.iter().chain(std::iter::once(&[0, 1, 0])) {
        let p = add(feet, *d);
        if core::block_name(g, p).ends_with("_bed") {
            let c = [f64::from(p[0]) + 0.5, f64::from(p[1]) + 0.3, f64::from(p[2]) + 0.5];
            core::look_at(g, c);
            core::click(g, true);
            rt.bed_wait = delay;
            return true;
        }
    }
    let Some(slot) = core::hotbar_find(g, |i| i.ends_with("_bed")) else {
        return false;
    };
    for d in SIDES {
        let p = add(feet, d);
        let foot = add(p, d);
        if core::replaceable(g, p) && core::replaceable(g, foot) && core::solid(g, add(p, [0, -1, 0])) {
            if core::place(g, p, slot) {
                rt.bed_wait = delay;
                return true;
            }
        }
    }
    false
}

fn auto_city(g: &mut Game<'_>, s: &mut State, rt: &mut Rt) {
    if !s.on("AutoCity") {
        return;
    }
    if let Some(pos) = rt.city {
        if core::is_air(g, pos) {
            rt.city = None;
            core::hold(g, false, false);
            s.say("AutoCity: broke the block");
            s.turn_off("AutoCity");
            return;
        }
        let c = pos.map(|v| f64::from(v) + 0.5);
        core::look_at(g, c);
        core::hold(g, false, true);
        return;
    }
    let range = s.n("AutoCity", "Range");
    let Some(t) = nearest_player(g, s, range + 2.0) else {
        return;
    };
    let Some(eye) = g.eye_position() else {
        return;
    };
    let feet = target_feet(&t);
    let pick = SIDES
        .iter()
        .map(|d| add(feet, *d))
        .filter(|p| matches!(&*core::block_name(g, *p), "minecraft:obsidian" | "minecraft:crying_obsidian" | "minecraft:ender_chest"))
        .filter(|p| core::dist(eye, p.map(|v| f64::from(v) + 0.5)) <= range)
        .min_by(|a, b| {
            let d = |p: &[i32; 3]| core::dist(eye, p.map(|v| f64::from(v) + 0.5));
            d(a).total_cmp(&d(b))
        });
    match pick {
        Some(p) => {
            if let Some(slot) = core::hotbar_find(g, |i| i.ends_with("_pickaxe")) {
                g.selected = slot;
            }
            rt.city = Some(p);
        }
        None => {
            s.say(format!("AutoCity: {} has nothing to mine", t.name));
            s.turn_off("AutoCity");
        }
    }
}

/// `Game::mob_hit` with Reach and Hitboxes: the entity under the crosshair.
pub fn mob_hit(g: &Game<'_>) -> Option<Option<(usize, f64)>> {
    let reach = core::REACH_ENTITIES.with(|c| c.get());
    let grow = core::HITBOX.with(|c| c.get());
    if !core::ACTIVE.with(|c| c.get()) || (reach <= 0.0 && grow <= 0.0) {
        return None;
    }
    let eye = g.eye_position()?;
    let dir = crate::game::view_vector(g.yaw, g.pitch);
    let base: f64 = if g.creative { 5.0 } else { 3.0 };
    let reach = if reach > 0.0 { reach.max(base) } else { base };
    let mut best: Option<(usize, f64)> = None;
    for (i, m) in g.mobs.iter().enumerate() {
        if !m.alive() || Some(m.id) == g.riding {
            continue;
        }
        let b = m.bounds();
        let (lo, hi) = (b.min_position(), b.max_position());
        let lo = [lo.x - grow, lo.y - grow, lo.z - grow];
        let hi = [hi.x + grow, hi.y + grow, hi.z + grow];
        let mut t0: f64 = 0.0;
        let mut t1: f64 = reach;
        let mut hit = true;
        for a in 0..3 {
            if dir[a].abs() < 1e-9 {
                if eye[a] < lo[a] || eye[a] > hi[a] {
                    hit = false;
                    break;
                }
                continue;
            }
            let (ta, tb) = ((lo[a] - eye[a]) / dir[a], (hi[a] - eye[a]) / dir[a]);
            t0 = t0.max(ta.min(tb));
            t1 = t1.min(ta.max(tb));
        }
        if hit && t0 <= t1 && best.is_none_or(|b| t0 < b.1) {
            best = Some((i, t0));
        }
    }
    Some(best)
}
