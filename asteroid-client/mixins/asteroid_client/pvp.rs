// Asteroid Client's mace and projectile modules: ElytraSwapper, AttributeSwap,
// AutoPearlCatch, AutoMace, StunWeb, WindChargeDeflect, CartPop and Predictions.
use super::Game;
use crate::game::asteroid_core::{self as core, State, Target};
use horizon_client::{hud::HudBatch, mobs::Who, projectiles::ProjectileKind};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
};

const MACE: &str = "minecraft:mace";
const ELYTRA: &str = "minecraft:elytra";

#[derive(Default)]
struct Rt {
    /// Players that just popped a totem: entity id, tick.
    pops: VecDeque<(i32, u64)>,
    cart: Option<Cart>,
    /// The tick of the last pearl thrown by hand.
    pearl_at: u64,
    pearls_done: HashSet<u32>,
    mace_wait: u32,
    deflect_wait: u32,
    /// StunWeb: who was last hit, when, and the health they had.
    hit: Option<(u32, u64, f32)>,
    /// AutoPearlCatch's throw: pearl, tries, wind charges held, next try's tick.
    pearl_try: Option<(u32, u32, u32, u64)>,
    /// The view and hand from before AutoPearlCatch took the wind charge up.
    pearl_keep: Option<(f32, f32, usize)>,
    /// Who threw each projectile, by id: the nearest player when it first showed.
    throwers: HashMap<u32, String>,
    said: HashSet<&'static str>,
}
struct Cart {
    id: u32,
    pos: [i32; 3],
    step: u8,
    start: u64,
}
thread_local! {
    static RT: RefCell<Rt> = RefCell::new(Rt::default());
    /// A totem event from the packet reader, for the next tick.
    static POPPED: RefCell<Vec<i32>> = const { RefCell::new(Vec::new()) };
    /// Keeps the game from using what is in hand this click: ElytraSwapper did it.
    pub static SKIP_RIGHT: Cell<bool> = const { Cell::new(false) };
}

/// Entity event 35.
pub fn on_totem(id: i32) {
    POPPED.with(|p| p.borrow_mut().push(id));
}

fn broken(g: &Game<'_>, st: &horizon_client::items::ItemStack) -> bool {
    let max = g.res.items().get(&st.item).map_or(0, |i| i.max_damage);
    max > 0 && st.damage + 1 >= max
}
fn is_chest(g: &Game<'_>, item: &str) -> bool {
    item != ELYTRA && g.res.items().get(item).and_then(|i| i.equip_slot.as_deref()) == Some("chest")
}

/// Puts hotbar slot `slot` on the chest, the old piece coming back to that slot.
fn wear(g: &mut Game<'_>, slot: usize) {
    if g.remote.is_some() {
        g.remote_menu_ops(&[(Some(501), slot as i32, horizon_client::menu::Input::Swap)]);
    } else {
        let new = g.inventory[slot].take();
        g.inventory[slot] = g.armor[1].take();
        g.armor[1] = new;
    }
}

/// A mouse button, before the game acts on it.
pub fn on_mouse(g: &mut Game<'_>, s: &mut State, right: bool) {
    if !core::playing(g) {
        return;
    }
    RT.with(|rt| {
        let rt = &mut *rt.borrow_mut();
        if right {
            if g.held().is_some_and(|h| &*h.item == "minecraft:ender_pearl") {
                rt.pearl_at = s.ticks;
            }
            elytra_swapper(g, s);
        } else {
            attribute_swap(g, s);
            if let Some((i, _)) = g.mob_hit() {
                rt.hit = Some((g.mobs[i].id, s.ticks, g.mobs[i].health));
            }
        }
    });
}

fn elytra_swapper(g: &mut Game<'_>, s: &mut State) {
    if !s.on("ElytraSwapper") || !core::inventory_menu(g) {
        return;
    }
    let slot = g.selected;
    let held = g.held().map(|h| h.item.clone());
    let piece = match held.as_deref() {
        Some(i) if i == ELYTRA || is_chest(g, i) => Some(slot),
        None if s.b("ElytraSwapper", "EmptyHand") => {
            let wearing_elytra = g.armor[1].as_ref().is_some_and(|a| &*a.item == ELYTRA);
            (0..9).find(|&i| {
                g.inventory[i].as_ref().is_some_and(|st| {
                    if wearing_elytra { is_chest(g, &st.item) } else { &*st.item == ELYTRA && !broken(g, st) }
                })
            })
        }
        _ => None,
    };
    let Some(piece) = piece else {
        return;
    };
    // The game's own use would equip it a moment later, broken or not.
    SKIP_RIGHT.with(|c| c.set(true));
    if g.inventory[piece].as_ref().is_some_and(|st| &*st.item == ELYTRA && broken(g, st)) {
        s.say("ElytraSwapper: that elytra is broken, so it stays off");
        return;
    }
    wear(g, piece);
}

/// AttributeSwap: an enchanted item's bonus goes with the hit, the hand comes straight back.
pub fn attribute_swap(g: &mut Game<'_>, s: &State) {
    if !s.on("AttributeSwap") || g.mob_hit().is_none() {
        return;
    }
    let falling = g.player.as_ref().is_some_and(|p| p.fall_distance >= 1.5 && !p.fall_flying);
    let want = match s.c("AttributeSwap", "Enchant") {
        "Breach" => "minecraft:breach",
        "Density" => "minecraft:density",
        _ if falling => "minecraft:density",
        _ => "minecraft:breach",
    };
    let has = |st: &horizon_client::items::ItemStack| st.enchantments().iter().any(|(id, _)| &**id == want);
    if g.held().is_some_and(|h| has(h)) {
        return;
    }
    let Some(slot) = (0..9).find(|&i| g.inventory[i].as_ref().is_some_and(|st| has(st))) else {
        return;
    };
    if s.b("AttributeSwap", "SwapBack") {
        core::restore_after_tick(g.yaw, g.pitch, g.selected);
    }
    g.selected = slot;
}

/// One tick; true when a module used this tick's click.
pub fn tick(g: &mut Game<'_>, s: &mut State) -> bool {
    RT.with(|rt| {
        let rt = &mut *rt.borrow_mut();
        SKIP_RIGHT.with(|c| c.set(false));
        rt.mace_wait = rt.mace_wait.saturating_sub(1);
        rt.deflect_wait = rt.deflect_wait.saturating_sub(1);
        for id in POPPED.with(|p| std::mem::take(&mut *p.borrow_mut())) {
            rt.pops.push_back((id, s.ticks));
        }
        while rt.pops.front().is_some_and(|p| s.ticks.saturating_sub(p.1) > 10) {
            rt.pops.pop_front();
        }
        note_throwers(g, rt);
        if !core::playing(g) {
            return false;
        }
        auto_pearl_catch(g, s, rt)
            || wind_deflect(g, s, rt)
            || auto_mace(g, s, rt)
            || stun_web(g, s, rt)
            || cart_pop(g, s, rt)
    })
}

/// The keep-the-view-and-hand wrapper every click below goes through.
fn keep(g: &Game<'_>) -> (f32, f32, usize) {
    (g.yaw, g.pitch, g.selected)
}

fn auto_mace(g: &mut Game<'_>, s: &mut State, rt: &mut Rt) -> bool {
    if !s.on("AutoMace") || rt.mace_wait > 0 || g.using.is_some() {
        return false;
    }
    let Some(p) = g.player.as_ref() else {
        return false;
    };
    // A glide isn't a fall: the mace's bonus goes by the fall, which an elytra resets.
    if p.fall_flying || p.flying || p.ground.on_ground || p.velocity.y >= 0.0 || p.fall_distance < s.n("AutoMace", "MinFall") {
        return false;
    }
    let (pl, h, a) = (s.b("AutoMace", "Players"), s.b("AutoMace", "Hostiles"), s.b("AutoMace", "Animals"));
    let Some(t) = core::targets(g, s, s.n("AutoMace", "Range"), pl, h, a).into_iter().next() else {
        return false;
    };
    let Some(slot) = core::hotbar_find(g, |i| i == MACE) else {
        if rt.said.insert("mace") {
            s.say("AutoMace: no mace in your hotbar");
        }
        return false;
    };
    let aim = s.b("AutoMace", "AutoAim");
    if !aim && g.mob_hit().is_none_or(|(i, _)| i != t.index) {
        return false;
    }
    let (yaw, pitch, hand) = keep(g);
    s.target = Some(t.id);
    if aim {
        core::look_at(g, t.center);
        if g.mob_hit().is_none_or(|(i, _)| i != t.index) {
            g.yaw = yaw;
            g.pitch = pitch;
            return false;
        }
    }
    g.selected = slot;
    g.clicks.miss_time = 0;
    core::click(g, false);
    let back = if s.b("AutoMace", "SwapBack") { hand } else { slot };
    core::restore_after_tick(yaw, pitch, back);
    rt.hit = Some((t.id, s.ticks, t.health));
    rt.mace_wait = 2;
    true
}

fn stun_web(g: &mut Game<'_>, s: &mut State, rt: &mut Rt) -> bool {
    if !s.on("StunWeb") {
        return false;
    }
    let Some((id, at, before)) = rt.hit else {
        return false;
    };
    if s.ticks.saturating_sub(at) > 6 {
        rt.hit = None;
        return false;
    }
    let Some(m) = g.mobs.iter().find(|m| m.id == id && m.alive()) else {
        return false;
    };
    // The hit has landed once the target shows it.
    if m.hurt_time <= 0 && m.health >= before {
        return false;
    }
    let p = [m.position.x.floor() as i32, (m.position.y + 0.2).floor() as i32, m.position.z.floor() as i32];
    rt.hit = None;
    let Some(slot) = core::hotbar_find(g, |i| i == "minecraft:cobweb") else {
        if rt.said.insert("web") {
            s.say("StunWeb: no cobweb in your hotbar");
        }
        return false;
    };
    let (yaw, pitch, hand) = keep(g);
    if core::place(g, p, slot) {
        g.pitch = -90.0;
        core::restore_after_tick(yaw, pitch, hand);
        return true;
    }
    false
}

fn wind_deflect(g: &mut Game<'_>, s: &mut State, rt: &mut Rt) -> bool {
    if !s.on("WindChargeDeflect") || rt.deflect_wait > 0 {
        return false;
    }
    let Some(eye) = g.eye_position() else {
        return false;
    };
    let range = s.n("WindChargeDeflect", "Range");
    let me = own_name(g);
    let found = g.creatures.projectiles.iter().find(|p| {
        matches!(p.kind, ProjectileKind::WindCharge)
            && !p.stuck
            && p.owner != Some(Who::Player)
            && rt.throwers.get(&p.id).is_none_or(|t| *t != me)
            && core::dist(eye, [p.position.x, p.position.y, p.position.z]) <= range
    });
    let Some(p) = found else {
        return false;
    };
    let at = [p.position.x, p.position.y + 0.15, p.position.z];
    let (yaw, pitch, hand) = keep(g);
    core::look_at(g, at);
    g.clicks.miss_time = 0;
    core::click(g, false);
    core::restore_after_tick(yaw, pitch, hand);
    rt.deflect_wait = 4;
    true
}

/// A pearl's next step, as `ThrowableProjectile.tick` moves it.
fn pearl_step(pos: &mut [f64; 3], vel: &mut [f64; 3]) {
    for a in 0..3 {
        pos[a] += vel[a];
        vel[a] *= 0.99;
    }
    vel[1] -= 0.03;
}

fn auto_pearl_catch(g: &mut Game<'_>, s: &mut State, rt: &mut Rt) -> bool {
    if !s.on("AutoPearlCatch") {
        rt.pearls_done.clear();
        return false;
    }
    let Some(eye) = g.eye_position() else {
        return false;
    };
    let recent = s.ticks.saturating_sub(rt.pearl_at) <= 30;
    let mine = g.creatures.projectiles.iter().find(|p| {
        matches!(p.kind, ProjectileKind::EnderPearl)
            && !p.stuck
            && !rt.pearls_done.contains(&p.id)
            && (p.owner == Some(Who::Player) || recent)
    });
    let Some(pearl) = mine else {
        return false;
    };
    let id = pearl.id;
    let mut pos = [pearl.position.x, pearl.position.y, pearl.position.z];
    let mut vel = [pearl.velocity.x, pearl.velocity.y, pearl.velocity.z];
    let Some(slot) = core::hotbar_find(g, |i| i == "minecraft:wind_charge") else {
        if rt.said.insert("wind") {
            s.say("AutoPearlCatch: no wind charge in your hotbar");
        }
        return false;
    };
    let have = core::count(g, |i| i == "minecraft:wind_charge");
    let mut tries = 0;
    if let Some((pearl_id, done, count, next)) = rt.pearl_try {
        if pearl_id == id {
            if have < count || done >= 5 {
                rt.pearls_done.insert(id);
                rt.pearl_try = None;
                return false;
            }
            if s.ticks < next {
                return false;
            }
            tries = done;
        }
    }
    // A wind charge flies straight at 1.5 blocks a tick: where it meets the pearl.
    let start = [eye[0], eye[1] - 0.1, eye[2]];
    let mut meet = None;
    for t in 1..(s.n("AutoPearlCatch", "MaxTicks") as u32) {
        pearl_step(&mut pos, &mut vel);
        if core::solid(g, pos.map(|v| v.floor() as i32)) {
            break;
        }
        if core::dist(start, pos) <= 1.5 * f64::from(t) {
            meet = Some(pos);
            break;
        }
    }
    let Some(at) = meet else {
        rt.pearls_done.insert(id);
        rt.pearl_try = None;
        return false;
    };
    rt.pearl_try = Some((id, tries + 1, have, s.ticks + 3));
    if g.selected != slot {
        rt.pearl_keep = Some(keep(g));
        rt.pearl_try = Some((id, tries, have, s.ticks));
        g.selected = slot;
        return true;
    }
    let (yaw, pitch, hand) = rt.pearl_keep.take().unwrap_or_else(|| keep(g));
    let (want_yaw, want_pitch) = core::angles(start, at);
    g.yaw += core::wrap(want_yaw - g.yaw);
    g.pitch = want_pitch;
    core::click(g, true);
    core::restore_after_tick(yaw, pitch, if s.b("AutoPearlCatch", "SwapBack") { hand } else { slot });
    true
}

fn cart_pop(g: &mut Game<'_>, s: &mut State, rt: &mut Rt) -> bool {
    if !s.on("CartPop") {
        rt.cart = None;
        return false;
    }
    if rt.cart.is_none() {
        let range = s.n("CartPop", "Range");
        let list: Vec<Target> = core::targets(g, s, range, true, false, false);
        let found = rt.pops.iter().find_map(|(id, _)| list.iter().find(|t| t.id as i32 == *id).cloned());
        if let Some(t) = found {
            rt.pops.clear();
            let pos = [
                ((t.min[0] + t.max[0]) / 2.0).floor() as i32,
                (t.min[1] + 0.2).floor() as i32,
                ((t.min[2] + t.max[2]) / 2.0).floor() as i32,
            ];
            rt.cart = Some(Cart { id: t.id, pos, step: 0, start: s.ticks });
        }
    }
    let Some(cart) = rt.cart.as_mut() else {
        return false;
    };
    if s.ticks.saturating_sub(cart.start) > 15 {
        rt.cart = None;
        return false;
    }
    s.target = Some(cart.id);
    let (yaw, pitch, hand) = keep(g);
    let pos = cart.pos;
    let step = cart.step;
    let done = match step {
        // A rail that a redstone signal arms the cart on.
        0 => match core::hotbar_find(g, |i| i == "minecraft:activator_rail") {
            Some(slot) => core::place(g, pos, slot),
            None => {
                s.say("CartPop: needs activator rails, a TNT minecart and a redstone block or torch in your hotbar");
                rt.cart = None;
                return false;
            }
        },
        1 => match core::hotbar_find(g, |i| i == "minecraft:tnt_minecart") {
            Some(slot) => core::use_on(g, pos, [0, 1, 0], slot),
            None => {
                s.say("CartPop: no TNT minecart in your hotbar");
                rt.cart = None;
                return false;
            }
        },
        _ => {
            let power = core::hotbar_find(g, |i| i == "minecraft:redstone_block" || i == "minecraft:redstone_torch");
            let Some(slot) = power else {
                s.say("CartPop: no redstone block or torch in your hotbar");
                rt.cart = None;
                return false;
            };
            [[1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1]].iter().any(|d: &[i32; 3]| {
                core::place(g, [pos[0] + d[0], pos[1], pos[2] + d[2]], slot)
            })
        }
    };
    if done {
        g.pitch = -90.0;
        core::restore_after_tick(yaw, pitch, hand);
        if let Some(c) = rt.cart.as_mut() {
            c.step += 1;
            if c.step > 2 {
                rt.cart = None;
            }
        }
    }
    done
}

fn own_name(g: &Game<'_>) -> String {
    g.remote.as_ref().map(|r| r.profile_name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| g.options.player_name())
}

/// Remembers who threw what: the nearest player where it first shows.
fn note_throwers(g: &Game<'_>, rt: &mut Rt) {
    let names = core::player_names(g);
    let me = own_name(g);
    let own = g.player.as_ref().map(|p| [p.position.x, p.position.y + 1.2, p.position.z]);
    let known: HashSet<u32> = g.creatures.projectiles.iter().map(|p| p.id).chain(g.tridents.iter().map(|t| t.id)).collect();
    rt.throwers.retain(|id, _| known.contains(id));
    let nearest = |at: [f64; 3]| -> Option<String> {
        let mut best: Option<(f64, String)> = own.map(|o| (core::dist(o, at), me.clone()));
        for m in &g.mobs {
            if let Some(n) = names.get(&m.id) {
                let d = core::dist([m.position.x, m.position.y + 1.2, m.position.z], at);
                if best.as_ref().is_none_or(|b| d < b.0) {
                    best = Some((d, n.clone()));
                }
            }
        }
        best.filter(|b| b.0 < 6.0).map(|b| b.1)
    };
    for p in &g.creatures.projectiles {
        if !rt.throwers.contains_key(&p.id)
            && let Some(n) = nearest([p.position.x, p.position.y, p.position.z])
        {
            rt.throwers.insert(p.id, n);
        }
    }
    for t in &g.tridents {
        if !rt.throwers.contains_key(&t.id)
            && let Some(n) = names.get(&t.owner)
        {
            rt.throwers.insert(t.id, n.clone());
        }
    }
}

// ------------------------------------------------------------- Predictions

/// A flight, ticked as its projectile moves: gravity, air drag, and whether it hits.
struct Flight {
    points: Vec<[f64; 3]>,
    ticks: u32,
    hit: Option<[f64; 3]>,
    block: Option<[i32; 3]>,
    who: Option<String>,
}

fn fly(g: &Game<'_>, mut pos: [f64; 3], mut vel: [f64; 3], gravity: f64, drag: f64, max: u32, skip: Option<u32>) -> Flight {
    let mut f = Flight { points: vec![pos], ticks: 0, hit: None, block: None, who: None };
    let names = core::player_names(g);
    for t in 1..=max {
        let next = [pos[0] + vel[0], pos[1] + vel[1], pos[2] + vel[2]];
        // Blocks along the step, at a quarter of a block at a time.
        let len = core::dist(pos, next);
        let steps = (len / 0.25).ceil().max(1.0) as u32;
        for i in 1..=steps {
            let u = f64::from(i) / f64::from(steps);
            let at = [pos[0] + vel[0] * u, pos[1] + vel[1] * u, pos[2] + vel[2] * u];
            let block = at.map(|v| v.floor() as i32);
            if core::solid(g, block) {
                f.points.push(at);
                f.hit = Some(at);
                f.block = Some(block);
                f.ticks = t;
                return f;
            }
            for m in g.mobs.iter().filter(|m| m.alive() && Some(m.id) != skip) {
                let half = m.kind.width / 2.0 + 0.15;
                let p = m.position;
                if (at[0] - p.x).abs() < half && (at[2] - p.z).abs() < half && at[1] >= p.y - 0.15 && at[1] <= p.y + f64::from(m.height()) + 0.15 {
                    f.points.push(at);
                    f.hit = Some(at);
                    f.ticks = t;
                    f.who = Some(names.get(&m.id).cloned().unwrap_or_else(|| core::pretty(&m.kind.id)));
                    return f;
                }
            }
        }
        pos = next;
        for v in &mut vel {
            *v *= drag;
        }
        vel[1] -= gravity;
        f.points.push(pos);
        if pos[1] < -80.0 {
            break;
        }
        f.ticks = t;
    }
    f
}

fn draw_flight(g: &Game<'_>, s: &State, batch: &mut HudBatch, cam: &core::Cam, f: &Flight, color: u32, label: &str) {
    for w in f.points.windows(2) {
        if let Some((a, b)) = cam.segment(w[0], w[1]) {
            core::line(g, batch, a, b, 1.0, color);
        }
    }
    if let Some(b) = f.block {
        let min = b.map(f64::from);
        cam.box_edges(g, batch, min, [min[0] + 1.0, min[1] + 1.0, min[2] + 1.0], color);
    }
    let end = f.hit.or(f.points.last().copied());
    if s.b("Predictions", "Labels")
        && let Some(end) = end
        && let Some((x, y)) = cam.project(end)
    {
        let mut line = format!("{label}  {:.1}s", f64::from(f.ticks) / 20.0);
        if let Some(who) = &f.who {
            line.push_str(&format!(" > {who}"));
        }
        let k = 0.75;
        let w = g.native_text_width(&line) * k;
        core::fill(g, batch, x - w / 2.0 - 2.0, y - 12.0, w + 4.0, 8.5, 0xB0_10_0A_14);
        core::text_k(g, batch, &line, x - w / 2.0, y - 11.0, color & 0xFF_FF_FF, k);
    }
}

fn potion_label(stack: &horizon_client::items::ItemStack, kind: &str) -> String {
    let base = if kind.contains("experience") { "XP bottle" } else if kind.contains("lingering") { "Lingering" } else { "Splash" };
    match stack.data().and_then(|d| d.potion.as_ref()) {
        Some(p) => format!("{base} {}", core::pretty(p)),
        None => base.to_owned(),
    }
}

/// Predictions: every flying projectile's path, and what is in hand.
pub fn predictions(g: &Game<'_>, s: &State, batch: &mut HudBatch, cam: &core::Cam) {
    if !s.on("Predictions") {
        return;
    }
    let max = s.n("Predictions", "MaxTicks") as u32;
    let accent = 0xFF00_0000 | core::accent(s);
    RT.with(|rt| {
        let rt = rt.borrow();
        let by = |id: u32, label: &str| match rt.throwers.get(&id) {
            Some(n) => format!("{label} ({n})"),
            None => label.to_owned(),
        };
        for p in &g.creatures.projectiles {
            if p.stuck {
                continue;
            }
            let (label, gravity, drag, color) = match &p.kind {
                ProjectileKind::EnderPearl => ("Pearl", 0.03, 0.99, 0xFF_40_E0_C0u32),
                ProjectileKind::Snowball => ("Snowball", 0.03, 0.99, 0xFF_E0_F0_FF),
                ProjectileKind::Egg => ("Egg", 0.03, 0.99, 0xFF_F0_E0_B0),
                ProjectileKind::Arrow { .. } => ("Arrow", 0.05, 0.99, 0xFF_FF_FF_FF),
                ProjectileKind::WindCharge | ProjectileKind::BreezeWindCharge => ("Wind charge", 0.0, 1.0, 0xFF_A0_D8_FF),
                ProjectileKind::SmallFireball | ProjectileKind::LargeFireball { .. } => ("Fireball", 0.0, 1.0, 0xFF_FF_90_30),
                ProjectileKind::WitherSkull { .. } => ("Wither skull", 0.0, 1.0, 0xFF_80_80_90),
                ProjectileKind::LlamaSpit => ("Spit", 0.06, 0.99, 0xFF_E0_E0_A0),
                ProjectileKind::Potion { .. } => ("Potion", 0.05, 0.99, 0xFF_E0_70_FF),
                ProjectileKind::Trident => ("Trident", 0.05, 0.99, 0xFF_60_E0_D0),
                // Horizon 66f3981 split these out; gravity as the game's getDefaultGravity.
                ProjectileKind::ExperienceBottle => ("XP bottle", 0.07, 0.99, 0xFF_80_FF_40),
                ProjectileKind::Thrown(t) => (t.id, 0.03, 0.99, 0xFF_FF_FF_FF),
                ProjectileKind::ShulkerBullet { .. } => continue,
            };
            let at = [p.position.x, p.position.y, p.position.z];
            let vel = [p.velocity.x, p.velocity.y, p.velocity.z];
            let f = fly(g, at, vel, gravity, drag, max, None);
            draw_flight(g, s, batch, cam, &f, color, &by(p.id, label));
        }
        for t in &g.tridents {
            if t.in_ground {
                continue;
            }
            let at = [t.position.x, t.position.y, t.position.z];
            let vel = [t.velocity.x, t.velocity.y, t.velocity.z];
            let f = fly(g, at, vel, 0.05, 0.99, max, None);
            draw_flight(g, s, batch, cam, &f, 0xFF_60_E0_D0, &by(t.id, "Trident"));
        }
        for (id, kind, stack, position, velocity) in g.potions.thrown_tracked() {
            let at = [position.x, position.y, position.z];
            let vel = [velocity.x, velocity.y, velocity.z];
            let gravity = if kind.contains("experience") { 0.07 } else { 0.05 };
            let f = fly(g, at, vel, gravity, 0.99, max, None);
            draw_flight(g, s, batch, cam, &f, 0xFF_E0_70_FF, &by(id, &potion_label(stack, kind)));
        }
    });
    if !s.b("Predictions", "Held") {
        return;
    }
    let Some(held) = g.held() else {
        return;
    };
    let item = &*held.item;
    let (speed, gravity, drag, label) = match item {
        "minecraft:bow" => {
            let f = g.using.map_or(1.0, |(_, t)| f64::from(t) / 20.0);
            (((f * f + 2.0 * f) / 3.0).clamp(0.1, 1.0) * 3.0, 0.05, 0.99, "Arrow")
        }
        "minecraft:crossbow" => (3.15, 0.05, 0.99, "Arrow"),
        "minecraft:trident" => (2.5, 0.05, 0.99, "Trident"),
        "minecraft:snowball" => (1.5, 0.03, 0.99, "Snowball"),
        "minecraft:egg" | "minecraft:blue_egg" | "minecraft:brown_egg" => (1.5, 0.03, 0.99, "Egg"),
        "minecraft:ender_pearl" => (1.5, 0.03, 0.99, "Pearl"),
        "minecraft:wind_charge" => (1.5, 0.0, 1.0, "Wind charge"),
        "minecraft:experience_bottle" => (0.7, 0.07, 0.99, "XP bottle"),
        i if i.ends_with("splash_potion") || i.ends_with("lingering_potion") => (0.5, 0.05, 0.99, "Potion"),
        _ => return,
    };
    let Some(eye) = g.eye_position() else {
        return;
    };
    let dir = crate::game::view_vector(g.yaw, g.pitch);
    let f = fly(g, [eye[0], eye[1] - 0.1, eye[2]], dir.map(|d| d * speed), gravity, drag, max, None);
    draw_flight(g, s, batch, cam, &f, accent, label);
}
