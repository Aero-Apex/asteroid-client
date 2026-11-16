// Asteroid Client's movement modules: Flight, Speed, Sprint, NoFall, Step,
// Spider, Jesus, BHop, AutoWalk, Sneak, SafeWalk, Parkour, NoSlow, ElytraFly,
// HighJump, LongJump, AirJump, AntiLevitation, FastClimb, Anchor,
// ReverseStep, BoatFly, GUIMove, Scaffold, and Freecam's body. Blink and
// NoFall's packet mode hook `remote_send_state`.
use super::{Game, Screen};
use crate::game::asteroid_core::{self as core, State};
use horizon_core::geometry::Vec3;
use std::{cell::RefCell, time::Duration};
use winit::keyboard::KeyCode;

#[derive(Default)]
struct Rt {
    was_on_ground: bool,
    jump_was_down: bool,
    fly_ticks: u32,
    /// Freecam: where the body stays, and how it was.
    freecam: Option<Freecam>,
}
struct Freecam {
    position: Vec3,
    yaw: f32,
    pitch: f32,
    spectator: bool,
    may_fly: bool,
    flying: bool,
}
thread_local! {
    static RT: RefCell<Rt> = RefCell::new(Rt::default());
}

const WALK: f64 = 0.2158;

/// Water or lava at `p`.
fn fluid_at(g: &Game<'_>, p: [i32; 3]) -> bool {
    let state = g.world.block(p[0], p[1], p[2]);
    g.res.facts().get(state).fluid != horizon_client::blocks::Fluid::None
}

/// A solid block under (x, y, z) within `depth` blocks.
fn ground_below(g: &Game<'_>, x: f64, y: f64, z: f64, depth: i32) -> bool {
    let top = (y - 0.01).floor() as i32;
    (1..=depth).any(|d| core::solid(g, [x.floor() as i32, top - d, z.floor() as i32]))
}
const SPRINT: f64 = 0.2806;

fn release_key(g: &mut Game<'_>, key: KeyCode) {
    if !g.physical_keys.contains(&key) {
        g.keys.remove(&key);
    }
}

pub fn toggled(g: &mut Game<'_>, s: &mut State, name: &str, on: bool) {
    RT.with(|rt| {
        let rt = &mut *rt.borrow_mut();
        match (name, on) {
            ("Flight", false) => {
                let creative = g.creative;
                if let Some(p) = g.player.as_mut() {
                    p.flying = false;
                    p.may_fly = creative || p.spectator;
                    p.flying_speed = 0.05;
                }
            }
            ("AutoWalk", false) => release_key(g, KeyCode::KeyW),
            ("Sneak", false) => release_key(g, KeyCode::ShiftLeft),
            ("Sprint", false) => release_key(g, KeyCode::ControlLeft),
            ("Step", on) => {
                if let Some(p) = g.player.as_mut() {
                    p.auto_jump_enabled = on;
                }
            }
            ("Freecam", true) => {
                if let Some(p) = g.player.as_mut() {
                    rt.freecam = Some(Freecam {
                        position: p.position,
                        yaw: g.yaw,
                        pitch: g.pitch,
                        spectator: p.spectator,
                        may_fly: p.may_fly,
                        flying: p.flying,
                    });
                    p.spectator = true;
                    p.may_fly = true;
                    p.flying = true;
                }
            }
            ("Freecam", false) => {
                if let (Some(f), Some(p)) = (rt.freecam.take(), g.player.as_mut()) {
                    p.position = f.position;
                    p.old_position = f.position;
                    p.velocity = Vec3::new(0.0, 0.0, 0.0);
                    p.spectator = f.spectator;
                    p.may_fly = f.may_fly;
                    p.flying = f.flying;
                    g.yaw = f.yaw;
                    g.pitch = f.pitch;
                }
            }
            _ => {}
        }
    });
    let _ = s;
}

pub fn tick(g: &mut Game<'_>, s: &mut State) {
    RT.with(|rt| {
        let rt = &mut *rt.borrow_mut();
        let (forward, left, jump, sneak) = core::input(g);
        let playing = core::playing(g);
        let yaw = g.yaw;
        let dir = if playing { core::direction(yaw, forward, left) } else { None };
        keys(g, s, forward);
        if s.on("Freecam") {
            freecam(g, s, dir, jump, sneak);
            rt.jump_was_down = jump;
            return;
        }
        gui_move(g, s);
        boat_fly(g, s, dir, jump, sneak);
        let jump_edge = jump && !rt.jump_was_down;
        rt.jump_was_down = jump;
        let has_levitation = g.potions.effects.list.iter().any(|e| e.id.ends_with("levitation"));
        let using = g.using.is_some();
        let feet = core::feet(g);
        // The block the feet are in: a player standing on water is inside its
        // top block (the surface sits below the block's top edge).
        let (fluid_in, fluid_below) = match g.player.as_ref() {
            Some(p) => {
                let (x, z) = (p.position.x.floor() as i32, p.position.z.floor() as i32);
                (
                    fluid_at(g, [x, p.position.y.floor() as i32, z]),
                    fluid_at(g, [x, (p.position.y - 0.1).floor() as i32, z]),
                )
            }
            None => (false, false),
        };
        let climbing = feet.is_some_and(|f| {
            let name = core::block_name(g, f);
            name.contains("ladder") || name.contains("vine") || name.contains("scaffolding")
        });
        let in_web = feet.is_some_and(|f| core::block_name(g, f).contains("cobweb"));
        let hole_below = feet.and_then(|f| {
            (1..=3).map(|d| [f[0], f[1] - d, f[2]]).find(|p| crate::game::asteroid_combat::is_hole(g, *p))
        });
        let food = g.player.as_ref().map_or(20, |p| p.health.food.food);
        let antikick = s.b("Flight", "AntiKick");
        rt.fly_ticks += 1;
        let fly_ticks = rt.fly_ticks;
        let Some(pos) = g.player.as_ref().map(|p| p.position) else {
            return;
        };
        let vel_xz = g.player.as_ref().map(|p| (p.velocity.x, p.velocity.z)).unwrap_or_default();
        let safe_x = ground_below(&*g, pos.x + vel_xz.0 * 1.5 + vel_xz.0.signum() * 0.3, pos.y, pos.z, 1);
        let safe_z = ground_below(&*g, pos.x, pos.y, pos.z + vel_xz.1 * 1.5 + vel_xz.1.signum() * 0.3, 1);
        let edge_ahead = !ground_below(&*g, pos.x + vel_xz.0 * 2.0, pos.y, pos.z + vel_xz.1 * 2.0, 1);
        let drop_small = ground_below(&*g, pos.x, pos.y, pos.z, 3);
        let was_on_ground = rt.was_on_ground;
        let Some(p) = g.player.as_mut() else {
            return;
        };
        let on_ground = p.ground.on_ground;
        rt.was_on_ground = on_ground;
        let jumped = was_on_ground && !on_ground && p.velocity.y > 0.3;

        // Flight
        if s.on("Flight") {
            p.fall_distance = 0.0;
            if s.c("Flight", "Mode") == "Vanilla" {
                p.may_fly = true;
                p.flying = true;
                p.flying_speed = (0.05 * s.n("Flight", "Speed")) as f32;
            } else {
                let speed = s.n("Flight", "Speed");
                let (x, z) = dir.map_or((0.0, 0.0), |(x, z)| (x * speed, z * speed));
                let mut y = if jump {
                    speed * 0.6
                } else if sneak {
                    -speed * 0.6
                } else {
                    0.0
                };
                if antikick && fly_ticks.is_multiple_of(40) && !jump {
                    y = -0.04;
                }
                p.velocity = Vec3::new(x, y, z);
            }
        }
        // ElytraFly
        if s.on("ElytraFly") && p.fall_flying {
            let speed = s.n("ElytraFly", "Speed");
            let vert = s.n("ElytraFly", "Vertical");
            let (x, z) = dir.map_or((0.0, 0.0), |(x, z)| (x * speed, z * speed));
            let y = if jump {
                vert
            } else if sneak {
                -vert
            } else {
                0.0
            };
            p.velocity = Vec3::new(x, y, z);
        }
        // Glide: a slow fall with nothing else flying the player.
        if s.on("Glide") && !on_ground && !p.fall_flying && !s.on("Flight") && !s.on("Jetpack") && p.velocity.y < 0.0 {
            p.velocity.y = p.velocity.y.max(-s.n("Glide", "Speed"));
        }
        // Jetpack: jump rises, sneak drops, nothing hovers.
        if s.on("Jetpack") && !on_ground && !p.fall_flying && !s.on("Flight") {
            let speed = s.n("Jetpack", "Speed");
            p.velocity.y = if jump {
                speed
            } else if sneak {
                -speed
            } else {
                0.0
            };
        }
        // CreativeFlight: the game's own creative flight, with the chosen speed.
        if s.on("CreativeFlight") && !s.on("Flight") {
            p.may_fly = true;
            p.flying_speed = (0.05 * s.n("CreativeFlight", "Speed")) as f32;
        }
        // Dolphin: a swim at the chosen speed in the way you move.
        if s.on("Dolphin") && fluid_in
            && let Some((x, z)) = dir
        {
            let speed = s.n("Dolphin", "Speed");
            p.velocity.x = x * speed;
            p.velocity.z = z * speed;
        }
        // Fish: in water, sink unless jump is held.
        if s.on("Fish") && fluid_in && !jump {
            p.velocity.y = p.velocity.y.min(-s.n("Fish", "Speed"));
        }
        // AutoSwim: in water, rise to the surface unless sneaking.
        if s.on("AutoSwim") && fluid_in && !sneak {
            p.velocity.y = p.velocity.y.max(s.n("AutoSwim", "Speed"));
        }
        // ExtraElytra: a push while gliding, up to a speed cap so it can't run away.
        if s.on("ExtraElytra") && p.fall_flying {
            let boost = s.n("ExtraElytra", "Boost");
            let h = (p.velocity.x * p.velocity.x + p.velocity.z * p.velocity.z).sqrt();
            if h > 0.0 && h < 2.5 {
                p.velocity.x *= boost;
                p.velocity.z *= boost;
            }
        }
        // SpeedHack: on the ground, a multiple of sprint speed in the way you move.
        if s.on("SpeedHack") && on_ground && !sneak
            && let Some((x, z)) = dir
        {
            let target = SPRINT * s.n("SpeedHack", "Multiplier");
            p.velocity.x = x * target;
            p.velocity.z = z * target;
        }
        // NoWeb: a cobweb doesn't hold you back.
        if s.on("NoWeb") && in_web && !p.flying
            && let Some((x, z)) = dir
        {
            p.velocity.x = x * SPRINT * 0.8;
            p.velocity.z = z * SPRINT * 0.8;
        }
        let flying = p.flying || p.fall_flying || s.on("Flight");
        // Speed and BHop
        if s.on("Speed") && !flying && !sneak {
            if let Some((x, z)) = dir {
                let target = SPRINT * s.n("Speed", "Speed");
                let strafe = s.c("Speed", "Mode") == "Strafe";
                if on_ground || strafe {
                    let current = (p.velocity.x.powi(2) + p.velocity.z.powi(2)).sqrt();
                    let v = if on_ground { target } else { current.max(target * 0.9) };
                    p.velocity.x = x * v;
                    p.velocity.z = z * v;
                }
                if on_ground && s.b("Speed", "Jump") {
                    p.velocity.y = 0.42;
                }
            }
        }
        if s.on("BHop") && on_ground && dir.is_some() && !flying {
            p.velocity.y = 0.42;
        }
        // Sprint (Omni)
        if s.on("Sprint") && s.b("Sprint", "Omni") && dir.is_some() && food > 6 && !sneak {
            p.sprinting = true;
        }
        // NoFall
        if s.on("NoFall") && s.c("NoFall", "Mode") == "Reset" {
            p.fall_distance = 0.0;
        }
        // Step
        if s.on("Step") {
            p.auto_jump_enabled = true;
            if p.collision.horizontal && on_ground && dir.is_some() {
                p.velocity.y = 0.42;
            }
        }
        // Spider
        if s.on("Spider") && p.collision.horizontal && dir.is_some() {
            p.velocity.y = s.n("Spider", "Speed");
        }
        // FastClimb
        if s.on("FastClimb") && climbing && p.collision.horizontal && forward > 0.0 {
            p.velocity.y = s.n("FastClimb", "Speed");
        }
        // Jesus: in a fluid the player rises to the surface; on it, the fall
        // stops and walking and jumping work as on ground. Sneak to go under.
        let mut on_surface = false;
        if s.on("Jesus") && !sneak && !flying {
            if fluid_in {
                p.velocity.y = p.velocity.y.max(if jump { 0.3 } else { 0.1 });
                p.fall_distance = 0.0;
            } else if fluid_below && p.velocity.y <= 0.0 {
                on_surface = true;
                p.velocity.y = if jump { 0.42 } else { 0.0 };
                p.fall_distance = 0.0;
                if let Some((x, z)) = dir {
                    let v = if p.sprinting { SPRINT } else { WALK };
                    p.velocity.x = x * v;
                    p.velocity.z = z * v;
                }
            }
        }
        core::JESUS_SURFACE.with(|c| c.set(on_surface));
        // NoSlow
        if s.on("NoSlow") && using && on_ground {
            if let Some((x, z)) = dir {
                p.velocity.x = x * WALK;
                p.velocity.z = z * WALK;
            }
        }
        // HighJump, LongJump
        if jumped && s.on("HighJump") {
            p.velocity.y *= s.n("HighJump", "Multiplier");
        }
        if jumped && s.on("LongJump") {
            let boost = s.n("LongJump", "Boost");
            p.velocity.x *= boost;
            p.velocity.z *= boost;
        }
        // AirJump
        if s.on("AirJump") && jump_edge && !on_ground {
            p.velocity.y = 0.42;
        }
        // AntiLevitation
        if s.on("AntiLevitation") && has_levitation && !flying {
            p.velocity.y = if jump && on_ground { 0.42 } else { p.velocity.y.min(-0.0784) };
        }
        // SafeWalk
        if s.on("SafeWalk") && on_ground && !jump {
            if !safe_x {
                p.velocity.x = 0.0;
            }
            if !safe_z {
                p.velocity.z = 0.0;
            }
        }
        // Parkour
        if s.on("Parkour") && on_ground && dir.is_some() && edge_ahead && !sneak {
            p.velocity.y = 0.42;
        }
        // Anchor
        if s.on("Anchor") && !on_ground && p.velocity.y < 0.0 {
            if let Some(h) = hole_below {
                let (cx, cz) = (f64::from(h[0]) + 0.5, f64::from(h[2]) + 0.5);
                p.velocity.x = (cx - p.position.x) * 0.5;
                p.velocity.z = (cz - p.position.z) * 0.5;
            }
        }
        // ReverseStep
        if s.on("ReverseStep") && was_on_ground && !on_ground && p.velocity.y <= 0.0 && drop_small && !flying {
            p.velocity.y = -1.0;
        }
    });
    scaffold(g, s);
}

pub fn frame(_g: &mut Game<'_>, _s: &mut State, _elapsed: Duration) {}

/// AutoWalk, Sneak and Sprint hold the keys a player would.
fn keys(g: &mut Game<'_>, s: &State, forward: f64) {
    if !core::playing(g) {
        return;
    }
    if s.on("AutoWalk") {
        g.keys.insert(KeyCode::KeyW);
    }
    if s.on("Sneak") {
        g.keys.insert(KeyCode::ShiftLeft);
    }
    if s.on("Sprint") {
        if forward > 0.0 || s.on("AutoWalk") {
            g.keys.insert(KeyCode::ControlLeft);
        } else {
            release_key(g, KeyCode::ControlLeft);
        }
    }
}

fn gui_move(g: &mut Game<'_>, s: &State) {
    if !s.on("GUIMove") || matches!(g.screen, Screen::Playing | Screen::Chat | Screen::Dead) {
        return;
    }
    let k = &g.physical_keys;
    let f = f64::from(i8::from(k.contains(&KeyCode::KeyW)) - i8::from(k.contains(&KeyCode::KeyS)));
    let l = f64::from(i8::from(k.contains(&KeyCode::KeyA)) - i8::from(k.contains(&KeyCode::KeyD)));
    let jump = k.contains(&KeyCode::Space);
    let sprint = k.contains(&KeyCode::ControlLeft);
    let yaw = g.yaw;
    let Some(p) = g.player.as_mut() else {
        return;
    };
    if let Some((x, z)) = core::direction(yaw, f, l) {
        let v = if sprint { SPRINT } else { WALK };
        p.velocity.x = x * v;
        p.velocity.z = z * v;
    }
    if jump && p.ground.on_ground {
        p.velocity.y = 0.42;
    }
}

fn boat_fly(g: &mut Game<'_>, s: &State, dir: Option<(f64, f64)>, jump: bool, sneak: bool) {
    if !s.on("BoatFly") {
        return;
    }
    let Some(id) = g.riding else {
        return;
    };
    let speed = s.n("BoatFly", "Speed");
    let vert = s.n("BoatFly", "Vertical");
    let Some(boat) = g.boats.iter_mut().find(|b| b.id == id) else {
        return;
    };
    if let Some((x, z)) = dir {
        boat.velocity.x = x * speed;
        boat.velocity.z = z * speed;
    }
    boat.velocity.y = if jump {
        vert
    } else if sneak {
        -vert
    } else {
        0.0
    };
}

fn freecam(g: &mut Game<'_>, s: &State, dir: Option<(f64, f64)>, jump: bool, sneak: bool) {
    let speed = s.n("Freecam", "Speed");
    let Some(p) = g.player.as_mut() else {
        return;
    };
    p.spectator = true;
    p.may_fly = true;
    p.flying = true;
    p.fall_distance = 0.0;
    let (x, z) = dir.map_or((0.0, 0.0), |(x, z)| (x * speed, z * speed));
    let y = if jump {
        speed * 0.6
    } else if sneak {
        -speed * 0.6
    } else {
        0.0
    };
    p.velocity = Vec3::new(x, y, z);
}

fn scaffold(g: &mut Game<'_>, s: &mut State) {
    if !s.on("Scaffold") || !core::playing(g) {
        return;
    }
    let Some(feet) = core::feet(g) else {
        return;
    };
    let below = [feet[0], feet[1] - 1, feet[2]];
    let (_, _, jump, _) = core::input(g);
    if s.b("Scaffold", "Tower") && jump {
        if let Some(p) = g.player.as_mut()
            && p.ground.on_ground
        {
            p.velocity.y = 0.42;
        }
    }
    if !core::replaceable(g, below) {
        return;
    }
    let Some(slot) = core::hotbar_find(g, |i| core::placeable(g, i, "Any solid")) else {
        if s.ticks.is_multiple_of(100) {
            s.say("Scaffold: no blocks in your hotbar");
        }
        return;
    };
    let (yaw, pitch, keep) = (g.yaw, g.pitch, g.selected);
    if !core::place(g, below, slot) {
        // A block diagonally below first, to place against.
        for d in [[1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1]] {
            let side = [below[0] + d[0], below[1], below[2] + d[2]];
            if core::place(g, side, slot) {
                break;
            }
        }
    }
    // The click goes out this tick; the view and the hand go back after it.
    core::restore_after_tick(yaw, pitch, keep);
}

/// `remote_send_state`: Blink and Freecam hold the movement back. NoFall's
/// packet mode says the player is on the ground while falling, as do Flight going
/// down and Jesus on a surface: a server counts a fall only while told it's in the air.
pub fn send_state(g: &mut Game<'_>) -> bool {
    if !core::ACTIVE.with(|c| c.get()) {
        return false;
    }
    if core::BLINK.with(|c| c.get()) {
        return true;
    }
    if core::NOFALL_PACKET.with(|c| c.get())
        && let Some(p) = g.player.as_mut()
        && p.fall_distance > 2.0
        && !p.fall_flying
    {
        p.ground.on_ground = true;
        p.fall_distance = 0.0;
    }
    if let Some(p) = g.player.as_mut() {
        let flying_down = core::FLIGHT_SAFE.with(|c| c.get()) && p.velocity.y < 0.0;
        if flying_down || core::JESUS_SURFACE.with(|c| c.get()) {
            p.ground.on_ground = true;
        }
    }
    false
}
