// Asteroid Client's player modules: AutoEat, AutoGap, AutoFish, AutoRespawn,
// AutoTool, AutoReplenish, AutoDrop, AutoClicker, ChestSwap, ChestSteal,
// EXPThrower, FastUse, NoBreakDelay, SpeedMine, MiddleClickFriend and Rotation.
// NoHunger lives in the `hunger` hooks.
// Reach lives in the `reach` and `mob_hit` hooks; NoBreakDelay and SpeedMine also hook the
// server-side mining in `remote_mining`.
use super::{Game, Screen};
use crate::game::asteroid_core::{self as core, State};
use std::{cell::RefCell, time::Duration};

#[derive(Default)]
struct Rt {
    /// The slot to go back to once AutoEat or AutoGap is done.
    eating: Option<usize>,
    eat_ticks: u32,
    fish_out: u32,
    fish_low: f64,
    recast: u32,
    tool_back: Option<usize>,
    clicks: f64,
    chest_step: u32,
    respawn_wait: u32,
    replenish_wait: u32,
    drop_wait: u32,
}
thread_local! {
    static RT: RefCell<Rt> = RefCell::new(Rt::default());
}

pub fn toggled(g: &mut Game<'_>, s: &mut State, name: &str, on: bool) {
    RT.with(|rt| {
        let rt = &mut *rt.borrow_mut();
        match (name, on) {
            ("AutoEat" | "AutoGap", false) => stop_eating(g, rt),
            ("AutoClicker", false) => {
                core::hold(g, false, false);
            }
            ("ChestSwap", true) => rt.chest_step = 0,
            ("AutoTool", false) => {
                if let Some(slot) = rt.tool_back.take() {
                    g.selected = slot;
                }
            }
            ("AutoFish", true) => {
                rt.recast = 0;
                rt.fish_out = 0;
            }
            _ => {}
        }
    });
    let _ = s;
}

pub fn tick(g: &mut Game<'_>, s: &mut State) {
    RT.with(|rt| {
        let rt = &mut *rt.borrow_mut();
        auto_respawn(g, s, rt);
        fast(g, s);
        chest_steal(g, s);
        if !core::playing(g) {
            return;
        }
        auto_eat(g, s, rt);
        auto_fish(g, s, rt);
        auto_tool(g, s, rt);
        auto_replenish(g, s, rt);
        auto_drop(g, s, rt);
        auto_clicker(g, s, rt);
        chest_swap(g, s, rt);
        exp_thrower(g, s);
    });
}

pub fn frame(g: &mut Game<'_>, s: &mut State, _elapsed: Duration) {
    fast(g, s);
    if s.on("Rotation") && core::playing(g) {
        if s.b("Rotation", "LockYaw") {
            g.yaw = if s.b("Rotation", "SnapYaw") {
                (g.yaw / 45.0).round() * 45.0
            } else {
                s.n("Rotation", "Yaw") as f32
            };
        }
        if s.b("Rotation", "LockPitch") {
            g.pitch = s.n("Rotation", "Pitch") as f32;
        }
    }
}

fn fast(g: &mut Game<'_>, s: &State) {
    if s.on("FastUse") {
        g.use_delay = 0;
        g.clicks.right_click_delay = 0;
    }
    if s.on("NoBreakDelay") {
        g.destroy_delay = 0;
        g.clicks.destroying.delay = 0;
    }
}

/// SpeedMine while the world runs in this window (in-frame single player).
fn speed_mine_local(g: &mut Game<'_>, s: &State) {
    if !s.on("SpeedMine") || g.remote.is_some() || !g.clicks.destroying.active {
        return;
    }
    let pos = g.clicks.destroying.pos;
    let extra = g.destroy_progress(pos) * (s.n("SpeedMine", "Multiplier") as f32 - 1.0);
    g.clicks.destroying.progress = (g.clicks.destroying.progress + extra).min(1.0);
}

fn stop_eating(g: &mut Game<'_>, rt: &mut Rt) {
    if let Some(slot) = rt.eating.take() {
        core::hold(g, true, false);
        g.selected = slot;
    }
}

fn food_slot(g: &Game<'_>, gapples: bool) -> Option<usize> {
    const BAD: [&str; 5] = [
        "minecraft:rotten_flesh",
        "minecraft:spider_eye",
        "minecraft:poisonous_potato",
        "minecraft:pufferfish",
        "minecraft:chorus_fruit",
    ];
    let mut best: Option<(usize, f32)> = None;
    for i in 0..9 {
        let Some(st) = g.inventory[i].as_ref() else {
            continue;
        };
        let item = &*st.item;
        if BAD.contains(&item) || (!gapples && item.contains("golden_apple")) {
            continue;
        }
        let Some((_, saturation)) = g.res.items().get(item).and_then(|i| i.food) else {
            continue;
        };
        if best.is_none_or(|b| saturation > b.1) {
            best = Some((i, saturation));
        }
    }
    best.map(|b| b.0)
}

fn eat(g: &mut Game<'_>, rt: &mut Rt, slot: usize) {
    if rt.eating.is_none() {
        rt.eating = Some(g.selected);
    }
    rt.eat_ticks = 0;
    g.selected = slot;
    core::hold(g, true, true);
}

fn auto_eat(g: &mut Game<'_>, s: &State, rt: &mut Rt) {
    let Some(p) = g.player.as_ref() else {
        return;
    };
    let (food, health) = (p.health.food.food, p.health.health());
    if rt.eating.is_some() {
        rt.eat_ticks += 1;
        let held_food = g
            .held()
            .and_then(|st| g.res.items().get(&st.item))
            .is_some_and(|i| i.food.is_some());
        let full = food >= 20 && !g.held().is_some_and(|st| st.item.contains("golden_apple"));
        if !held_food || (full && g.using.is_none()) || rt.eat_ticks > 80 {
            stop_eating(g, rt);
        } else {
            core::hold(g, true, true);
        }
        return;
    }
    if s.on("AutoGap") && f64::from(health) <= s.n("AutoGap", "Health") {
        let slot = core::hotbar_find(g, |i| i == "minecraft:enchanted_golden_apple")
            .or_else(|| core::hotbar_find(g, |i| i == "minecraft:golden_apple"));
        if let Some(slot) = slot {
            eat(g, rt, slot);
            return;
        }
    }
    if !s.on("AutoEat") {
        return;
    }
    let low_health = s.n("AutoEat", "Health") > 0.0 && f64::from(health) <= s.n("AutoEat", "Health");
    if (f64::from(food) < s.n("AutoEat", "Hunger") || low_health) && food < 20 {
        if let Some(slot) = food_slot(g, s.b("AutoEat", "Gapples")) {
            eat(g, rt, slot);
        }
    }
}

fn auto_fish(g: &mut Game<'_>, s: &State, rt: &mut Rt) {
    if !s.on("AutoFish") {
        return;
    }
    if !g.held().is_some_and(|st| &*st.item == "minecraft:fishing_rod") {
        return;
    }
    match g.hook.as_ref() {
        Some(hook) => {
            rt.fish_out += 1;
            let y = hook.position.y;
            let dip = hook.previous.y - y;
            // The bobber settles first; a bite pulls it under.
            if rt.fish_out > 40 && dip > 0.12 && y < rt.fish_low + 0.05 {
                core::click(g, true);
                rt.recast = s.n("AutoFish", "RecastDelay") as u32;
                rt.fish_out = 0;
            } else if rt.fish_out <= 40 {
                rt.fish_low = y;
            } else {
                rt.fish_low = rt.fish_low.min(y + 0.05).max(y - 0.5);
            }
            if rt.fish_out > 20 * 60 {
                // Nothing for a minute: cast again.
                core::click(g, true);
                rt.recast = 10;
                rt.fish_out = 0;
            }
        }
        None => {
            if rt.recast > 0 {
                rt.recast -= 1;
            } else {
                core::click(g, true);
                rt.recast = 20;
                rt.fish_out = 0;
            }
        }
    }
}

fn auto_respawn(g: &mut Game<'_>, s: &State, rt: &mut Rt) {
    if !s.on("AutoRespawn") || g.screen != Screen::Dead {
        rt.respawn_wait = 0;
        return;
    }
    rt.respawn_wait += 1;
    if rt.respawn_wait == 5 || rt.respawn_wait.is_multiple_of(40) {
        let _ = g.respawn_player();
    }
}

fn auto_tool(g: &mut Game<'_>, s: &State, rt: &mut Rt) {
    if !s.on("AutoTool") {
        return;
    }
    let mining = g.left && g.using.is_none();
    let target = if mining { g.pick() } else { None };
    let Some(hit) = target else {
        if s.b("AutoTool", "SwitchBack")
            && let Some(slot) = rt.tool_back.take()
        {
            g.selected = slot;
        }
        return;
    };
    let current = g.selected;
    let mut best = (current, g.destroy_progress(hit.pos));
    for i in 0..9 {
        // A tool about to break is left alone.
        if g.inventory[i].as_ref().is_some_and(|st| {
            let max = g.res.items().get(&st.item).map_or(0, |i| i.max_damage);
            max > 0 && st.damage + 3 >= max
        }) {
            continue;
        }
        g.selected = i;
        let p = g.destroy_progress(hit.pos);
        if p > best.1 + 1e-4 {
            best = (i, p);
        }
    }
    g.selected = current;
    if best.0 != current {
        if rt.tool_back.is_none() {
            rt.tool_back = Some(current);
        }
        g.selected = best.0;
    }
    speed_mine_local(g, s);
}

fn auto_replenish(g: &mut Game<'_>, s: &State, rt: &mut Rt) {
    if !s.on("AutoReplenish") || !core::inventory_menu(g) {
        return;
    }
    if rt.replenish_wait > 0 {
        rt.replenish_wait -= 1;
        return;
    }
    let threshold = s.n("AutoReplenish", "Threshold") as u32;
    for i in 0..9 {
        let Some(st) = g.inventory[i].clone() else {
            continue;
        };
        let max = g.res.items().get(&st.item).map_or(1, |i| i.max_stack);
        if max <= 1 || st.count >= threshold.min(max) {
            continue;
        }
        let Some(j) = (9..36).find(|&j| {
            g.inventory[j]
                .as_ref()
                .is_some_and(|o| o.item == st.item && o.extra.is_none() && st.extra.is_none())
        }) else {
            continue;
        };
        rt.replenish_wait = 4;
        if g.remote.is_some() {
            core::quick_move(g, j);
        } else if let Some(from) = g.inventory[j].as_mut() {
            let moved = from.count.min(max - st.count);
            from.count -= moved;
            if from.count == 0 {
                g.inventory[j] = None;
            }
            if let Some(to) = g.inventory[i].as_mut() {
                to.count += moved;
            }
        }
        return;
    }
}

fn auto_clicker(g: &mut Game<'_>, s: &State, rt: &mut Rt) {
    if !s.on("AutoClicker") {
        return;
    }
    let right = s.c("AutoClicker", "Button") == "Right";
    if s.b("AutoClicker", "OnlyWhenHeld") && !(if right { g.right } else { g.left }) {
        return;
    }
    rt.clicks += s.n("AutoClicker", "CPS") / 20.0;
    while rt.clicks >= 1.0 {
        rt.clicks -= 1.0;
        core::click(g, right);
    }
}

fn chest_swap(g: &mut Game<'_>, s: &mut State, rt: &mut Rt) {
    if !s.on("ChestSwap") || !core::inventory_menu(g) {
        return;
    }
    let wearing_elytra = g.armor[1].as_ref().is_some_and(|st| &*st.item == "minecraft:elytra");
    let want = |item: &str, g: &Game<'_>| {
        if wearing_elytra {
            g.res.items().get(item).and_then(|i| i.equip_slot.as_deref()) == Some("chest")
                && item != "minecraft:elytra"
        } else {
            item == "minecraft:elytra"
        }
    };
    let Some(slot) = (0..36).find(|&i| g.inventory[i].as_ref().is_some_and(|st| want(&st.item, g))) else {
        s.say(if wearing_elytra { "ChestSwap: no chestplate" } else { "ChestSwap: no elytra" });
        s.turn_off("ChestSwap");
        return;
    };
    if g.remote.is_some() {
        if g.armor[1].is_some() && rt.chest_step == 0 {
            core::quick_move(g, 501);
            rt.chest_step = 1;
            return;
        }
        core::quick_move(g, slot);
    } else {
        let new = g.inventory[slot].take();
        g.inventory[slot] = g.armor[1].take();
        g.armor[1] = new;
    }
    s.turn_off("ChestSwap");
}

fn exp_thrower(g: &mut Game<'_>, s: &State) {
    if !s.on("EXPThrower") || !s.ticks.is_multiple_of(2) {
        return;
    }
    let Some(slot) = core::hotbar_find(g, |i| i == "minecraft:experience_bottle") else {
        return;
    };
    g.selected = slot;
    if s.b("EXPThrower", "LookDown") {
        g.pitch = 90.0;
    }
    core::click(g, true);
}

/// The middle mouse button: MiddleClickFriend.
pub fn middle_click(g: &mut Game<'_>, s: &mut State) {
    if !s.on("MiddleClickFriend") || !g.native_playing() {
        return;
    }
    let Some((i, ..)) = g.mob_hit() else {
        return;
    };
    let id = g.mobs[i].id;
    let names = core::player_names(g);
    let Some(name) = names.get(&id).cloned() else {
        return;
    };
    if s.is_friend(&name) {
        s.friends.retain(|f| !f.eq_ignore_ascii_case(&name));
        s.say(format!("Removed {name} from friends"));
    } else {
        s.friends.insert(name.clone());
        s.say(format!("Added {name} to friends"));
    }
    s.dirty = true;
}

/// ChestSteal: shift-clicks the stacks of the open container into the
/// inventory, one per tick.
fn chest_steal(g: &mut Game<'_>, s: &State) {
    if !s.on("ChestSteal") || g.container.is_none() {
        return;
    }
    for n in 0..g.container_size() {
        if g.container_slot_mut(n).is_some() {
            g.menu_ops(&[(Some(300 + n), 0, horizon_client::menu::Input::QuickMove)]);
            return;
        }
    }
}

/// AutoDrop: throws away whole stacks of the listed items, a few ticks apart.
fn auto_drop(g: &mut Game<'_>, s: &State, rt: &mut Rt) {
    if !s.on("AutoDrop") || !core::inventory_menu(g) {
        return;
    }
    if rt.drop_wait > 0 {
        rt.drop_wait -= 1;
        return;
    }
    let filter = s.list("AutoDrop", "Filter");
    let Some(slot) = core::inventory_find(g, |id| filter.iter().any(|f| f == id)) else {
        return;
    };
    core::throw_stack(g, slot);
    rt.drop_wait = 3;
}
