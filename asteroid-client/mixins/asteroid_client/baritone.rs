// Asteroid Client's pathing: an original Rust take on what Baritone (by leijurv
// and the cabaletta contributors) offers. An
// A* over feet positions (walking, sprinting, jumping up, falling, swimming,
// climbing, digging, bridging and pillaring), run a few milliseconds a tick,
// and the processes on top of it: goto, mine, follow, explore, farm, tunnel.
// It plays through the keys and the mouse, as a player would. `#` commands.
//
// Not made or endorsed by the Baritone project; no Baritone code is used.
use super::Game;
use crate::game::asteroid_core::{self as core, State};
use horizon_client::hud::HudBatch;
use std::{
    cell::RefCell,
    cmp::Ordering,
    collections::{BinaryHeap, HashMap, HashSet, VecDeque},
    time::{Duration, Instant},
};
use winit::keyboard::KeyCode;

type P = [i32; 3];

fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn center(p: P) -> [f64; 3] {
    [f64::from(p[0]) + 0.5, f64::from(p[1]), f64::from(p[2]) + 0.5]
}
fn mid(p: P) -> [f64; 3] {
    p.map(|v| f64::from(v) + 0.5)
}

// ---------------------------------------------------------- what blocks are

const PASS: u16 = 1; // the body fits in it
const FLOOR: u16 = 2; // stands on it
const WATER: u16 = 4;
const LAVA: u16 = 8;
const DANGER: u16 = 16; // hurts or traps whoever touches it
const BREAK: u16 = 32; // can be mined
const FALLS: u16 = 64; // sand, gravel: falls when what's under it goes
const CLIMB: u16 = 128; // ladders and vines
const INFESTED: u16 = 256; // silverfish come out of it when it breaks

fn classify(g: &Game<'_>, state: horizon_world::registry::BlockStateId) -> u16 {
    let facts = g.res.facts();
    if facts.is_air(state) {
        return PASS;
    }
    let f = facts.get(state);
    let name = f.name.strip_prefix("minecraft:").unwrap_or(&f.name);
    let mut out = 0;
    match f.fluid {
        horizon_client::blocks::Fluid::Water => out |= WATER,
        horizon_client::blocks::Fluid::Lava => out |= LAVA,
        horizon_client::blocks::Fluid::None => {}
    }
    let danger = matches!(
        name,
        "fire" | "soul_fire" | "magma_block" | "cactus" | "sweet_berry_bush" | "wither_rose" | "campfire"
            | "soul_campfire" | "cobweb" | "powder_snow" | "pointed_dripstone" | "end_portal" | "nether_portal"
            | "end_gateway" | "turtle_egg" | "stonecutter"
    );
    if danger {
        out |= DANGER;
    }
    let climb = matches!(
        name,
        "ladder" | "vine" | "twisting_vines" | "twisting_vines_plant" | "weeping_vines" | "weeping_vines_plant"
            | "cave_vines" | "cave_vines_plant"
    );
    if climb {
        out |= CLIMB;
    }
    // Fences and walls stand 1.5 high: no standing on them, no jumping them.
    let tall = name.ends_with("_fence") || name.ends_with("_fence_gate") || name.ends_with("_wall");
    // Thin covers (sculk veins, glow lichen, resin) can carry the legacy "solid"
    // flag but hold nobody up: a sculk vein over a drop was walked onto as floor.
    let thin = f.replaceable || name.ends_with("_vein") || name == "glow_lichen" || name == "resin_clump";
    let blocks = !thin && (f.full_collision || f.legacy_solid)
        || tall
        || name.ends_with("_door")
        || name.ends_with("_trapdoor")
        || name.ends_with("_pane")
        || name == "iron_bars"
        || name == "chain";
    if climb {
        out |= PASS;
    } else if !blocks && out & (LAVA | DANGER) == 0 {
        out |= PASS;
    }
    if blocks && !tall && !danger && !climb && !f.outline.is_empty() && out & (WATER | LAVA) == 0 {
        out |= FLOOR;
    }
    if f.hardness >= 0.0 && f.fluid == horizon_client::blocks::Fluid::None && !danger {
        out |= BREAK;
    }
    if f.classes.iter().any(|c| c.contains("FallingBlock")) || name.ends_with("_anvil") || name == "anvil" {
        out |= FALLS;
    }
    if name.starts_with("infested_") {
        out |= INFESTED;
    }
    out
}

#[derive(Default)]
struct Tables {
    cls: Vec<u16>,
    /// Block state id → ticks to break it with the best tool in the hotbar.
    brk: HashMap<u32, f32>,
    /// Steps (from, to) that were tried and went nowhere: the search leaves them out.
    banned: HashSet<(P, P)>,
}
impl Tables {
    fn at(&mut self, g: &Game<'_>, p: P) -> u16 {
        let state = g.world.block(p[0], p[1], p[2]);
        let i = state.0 as usize;
        if i >= self.cls.len() {
            self.cls.resize(i + 1, u16::MAX);
        }
        if self.cls[i] == u16::MAX {
            self.cls[i] = classify(g, state);
        }
        self.cls[i]
    }
}

fn loaded(g: &Game<'_>, p: P) -> bool {
    g.world.chunks.contains_key(&(p[0] >> 4, p[2] >> 4))
}

/// The hotbar slot that breaks `pos` fastest, and in how many ticks.
fn best_tool(g: &mut Game<'_>, pos: P) -> (usize, f32) {
    let keep = g.selected;
    let ground = g.player.as_mut().map(|p| std::mem::replace(&mut p.ground.on_ground, true));
    let mut best = (keep, g.destroy_progress(pos));
    for i in 0..9 {
        g.selected = i;
        let progress = g.destroy_progress(pos);
        if progress > best.1 + 1e-5 {
            best = (i, progress);
        }
    }
    g.selected = keep;
    if let (Some(on), Some(p)) = (ground, g.player.as_mut()) {
        p.ground.on_ground = on;
    }
    (best.0, if best.1 > 0.0 { 1.0 / best.1 } else { f32::INFINITY })
}

// ---------------------------------------------------------------- the goals

#[derive(Clone, Debug)]
enum Goal {
    /// The feet at this position.
    Block(P),
    /// Within `r` blocks of this position.
    Near(P, i32),
    /// This column, at any height.
    Xz(i32, i32),
    /// This height, anywhere.
    Y(i32),
    /// Beside a block of these: close enough to mine it.
    Beside(Vec<P>),
    /// Under the open sky.
    Surface,
}

fn octile(dx: f64, dz: f64) -> f64 {
    let (a, b) = (dx.abs().max(dz.abs()), dx.abs().min(dz.abs()));
    (a - b) * SPRINT + b * SPRINT * std::f64::consts::SQRT_2
}
fn vertical(dy: f64) -> f64 {
    if dy > 0.0 { dy * 8.0 } else { -dy * 6.0 }
}

impl Goal {
    fn reached(&self, g: &Game<'_>, t: &mut Tables, p: P) -> bool {
        match self {
            Goal::Block(b) => p == *b,
            Goal::Near(b, r) => {
                (p[0] - b[0]).abs() <= *r && (p[2] - b[2]).abs() <= *r && (p[1] - b[1]).abs() <= 1
            }
            Goal::Xz(x, z) => p[0] == *x && p[2] == *z,
            Goal::Y(y) => p[1] == *y,
            Goal::Beside(list) => list.iter().any(|b| beside(p, *b)),
            Goal::Surface => {
                let sky = (1..=96).all(|d| {
                    let c = t.at(g, [p[0], p[1] + 1 + d, p[2]]);
                    c & FLOOR == 0 && c & WATER == 0
                });
                sky && [[1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1]].iter().all(|d| {
                    t.at(g, add(p, *d)) & FLOOR == 0 && t.at(g, add(add(p, *d), [0, 1, 0])) & FLOOR == 0
                })
            }
        }
    }
    fn h(&self, p: P) -> f64 {
        let d = |b: P| {
            octile(f64::from(b[0] - p[0]), f64::from(b[2] - p[2])) + vertical(f64::from(b[1] - p[1]))
        };
        match self {
            Goal::Block(b) | Goal::Near(b, _) => d(*b),
            Goal::Xz(x, z) => octile(f64::from(*x - p[0]), f64::from(*z - p[2])),
            Goal::Y(y) => vertical(f64::from(*y - p[1])),
            Goal::Beside(list) => list.iter().take(16).map(|b| d(*b)).fold(f64::MAX, f64::min),
            Goal::Surface => 0.0,
        }
    }
    fn label(&self) -> String {
        match self {
            Goal::Block(b) => format!("{} {} {}", b[0], b[1], b[2]),
            Goal::Near(b, _) => format!("near {} {} {}", b[0], b[1], b[2]),
            Goal::Xz(x, z) => format!("{x} {z}"),
            Goal::Y(y) => format!("y {y}"),
            Goal::Beside(list) => format!("{} blocks", list.len()),
            Goal::Surface => "the surface".to_owned(),
        }
    }
}

/// Feet at `p` can mine `b`: at the side (feet or head level), on it, or under it.
fn beside(p: P, b: P) -> bool {
    let (dx, dz) = ((p[0] - b[0]).abs(), (p[2] - b[2]).abs());
    (dx + dz == 1 && (p[1] == b[1] || p[1] == b[1] - 1)) || (dx + dz == 0 && (p[1] == b[1] + 1 || p[1] == b[1] - 2))
}

// ---------------------------------------------------------------- the search

const WALK: f64 = 4.633;
const SPRINT: f64 = 3.564;
const SWIM: f64 = 9.091;
const JUMP: f64 = 7.0;
const LADDER: f64 = 8.5;
const SNEAK_PLACE: f64 = 16.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mv {
    Start,
    Walk,
    Diag,
    Up,
    Fall,
    Pillar,
    Dig,
    Swim,
    Climb,
    Leap,
}

#[derive(Clone, Copy, Debug)]
struct Step {
    pos: P,
    mv: Mv,
}

#[derive(Clone, Copy)]
struct Allow {
    brk: bool,
    place: bool,
    down: bool,
    parkour: bool,
    max_fall: i32,
    place_cost: f64,
    break_mult: f64,
    /// Survival: never plan a move that puts the head under water.
    safe: bool,
    /// Survival: the extra cost of a step in an open cave.
    cave: f64,
}

struct Entry {
    f: f64,
    g: f64,
    p: P,
}
impl PartialEq for Entry {
    fn eq(&self, o: &Self) -> bool {
        self.f == o.f
    }
}
impl Eq for Entry {}
impl PartialOrd for Entry {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Entry {
    fn cmp(&self, o: &Self) -> Ordering {
        o.f.total_cmp(&self.f)
    }
}

struct Plan {
    goal: Goal,
    allow: Allow,
    open: BinaryHeap<Entry>,
    seen: HashMap<P, (f64, P, Mv)>,
    start: P,
    best: (P, f64),
    spent: Duration,
    expanded: u32,
}

impl Plan {
    fn new(goal: Goal, allow: Allow, start: P) -> Self {
        let mut plan = Self {
            best: (start, goal.h(start)),
            goal,
            allow,
            open: BinaryHeap::new(),
            seen: HashMap::new(),
            start,
            spent: Duration::ZERO,
            expanded: 0,
        };
        plan.seen.insert(start, (0.0, start, Mv::Start));
        let f = plan.goal.h(start);
        plan.open.push(Entry { f, g: 0.0, p: start });
        plan
    }
    fn path_to(&self, end: P) -> Vec<Step> {
        let mut out = Vec::new();
        let mut at = end;
        while let Some(&(_, parent, mv)) = self.seen.get(&at) {
            out.push(Step { pos: at, mv });
            if at == self.start {
                break;
            }
            at = parent;
        }
        out.reverse();
        out
    }
}

enum Searched {
    Running,
    Found(Vec<Step>, f64),
    Partial(Vec<Step>, f64),
    Nothing,
}

/// The ticks to clear `p` for the body: none when it can't be cleared.
fn clear_cost(g: &mut Game<'_>, t: &mut Tables, a: &Allow, p: P) -> Option<f64> {
    let c = t.at(g, p);
    if c & PASS != 0 {
        return Some(0.0);
    }
    if c & BREAK == 0 || !a.brk {
        return None;
    }
    // Survival: silverfish swarm out of infested stone, into a 1x1 shaft or a wall.
    if a.safe && c & INFESTED != 0 {
        return None;
    }
    // Mining next to a fluid floods the path.
    for d in [[0, 1, 0], [1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1]] {
        if t.at(g, add(p, d)) & (WATER | LAVA) != 0 {
            return None;
        }
    }
    let state = g.world.block(p[0], p[1], p[2]);
    let ticks = match t.brk.get(&state.0) {
        Some(v) => *v,
        None => {
            let v = best_tool(g, p).1;
            t.brk.insert(state.0, v);
            v
        }
    };
    if !ticks.is_finite() {
        return None;
    }
    let mut cost = f64::from(ticks) * a.break_mult + 2.0;
    // What rests on it comes down: in Survival never (gravel landing in the
    // cells the player stands in suffocates them), else at a price.
    if t.at(g, add(p, [0, 1, 0])) & FALLS != 0 {
        if a.safe {
            return None;
        }
        cost += 40.0;
    }
    Some(cost)
}

fn fall_cost(h: i32) -> f64 {
    // Roughly the ticks of a fall of `h` blocks.
    5.0 * f64::from(h).sqrt()
}

/// Sand or gravel with nothing under it: it holds until a block beside it
/// changes (digging next to it), then drops whoever stands on it.
fn loose(g: &Game<'_>, t: &mut Tables, p: P) -> bool {
    t.at(g, p) & FALLS != 0 && t.at(g, add(p, [0, -1, 0])) & PASS != 0
}

/// `bridged`: the plan placed the block under `n`, so `n` can be stood on.
fn expand(g: &mut Game<'_>, t: &mut Tables, a: &Allow, n: P, bridged: bool, out: &mut Vec<(P, Mv, f64)>) {
    const SIDES: [[i32; 3]; 4] = [[1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1]];
    const UP: P = [0, 1, 0];
    const DOWN: P = [0, -1, 0];
    let here = t.at(g, n);
    let under = t.at(g, add(n, DOWN));
    let swimming = here & WATER != 0;
    let standing = under & FLOOR != 0 || bridged;
    let footing = standing || swimming || here & CLIMB != 0;
    let step_cost = if swimming { SWIM } else { WALK };
    for d in SIDES {
        let m = add(n, d);
        if !loaded(g, m) {
            continue;
        }
        let mc = t.at(g, m);
        // Walk across: the body clear, something to stand on.
        if mc & LAVA == 0
            && let Some(feet) = clear_cost(g, t, a, m)
            && let Some(head) = clear_cost(g, t, a, add(m, UP))
        {
            let below = t.at(g, add(m, DOWN));
            let base = step_cost + feet + head;
            if mc & WATER != 0 {
                if !(a.safe && t.at(g, add(m, UP)) & WATER != 0) {
                    out.push((m, Mv::Swim, SWIM + feet + head));
                }
            } else if below & FLOOR != 0 && below & DANGER == 0 {
                out.push((m, Mv::Walk, base));
            } else if a.place && standing && below & PASS != 0 && below & (LAVA | WATER) == 0 {
                out.push((m, Mv::Walk, base + a.place_cost + SNEAK_PLACE));
            }
        }
        // Up one block.
        let up = add(m, UP);
        if footing
            && t.at(g, up) & LAVA == 0
            && let Some(above) = clear_cost(g, t, a, add(n, [0, 2, 0]))
            && let Some(feet) = clear_cost(g, t, a, up)
            && let Some(head) = clear_cost(g, t, a, add(up, UP))
        {
            let cost = JUMP + above + feet + head;
            if mc & FLOOR != 0 {
                out.push((up, Mv::Up, cost));
            } else if a.place && mc & PASS != 0 && mc & (LAVA | WATER) == 0 && t.at(g, add(m, DOWN)) & FLOOR != 0 {
                // A block needs something to go on: nothing can be placed in mid-air.
                out.push((up, Mv::Up, cost + a.place_cost));
            }
        }
        // Down: the column in front clear, as far as the fall is allowed.
        if mc & PASS != 0
            && mc & (LAVA | DANGER) == 0
            && t.at(g, add(m, DOWN)) & FLOOR == 0
            && let Some(head) = clear_cost(g, t, a, add(m, UP))
        {
            for h in 1..=64 {
                let q = [m[0], n[1] - h, m[2]];
                if q[1] < -64 {
                    break;
                }
                let c = t.at(g, q);
                if c & PASS == 0 {
                    break;
                }
                if c & WATER != 0 {
                    out.push((q, Mv::Fall, WALK + head + fall_cost(h)));
                    break;
                }
                let below = t.at(g, add(q, DOWN));
                if below & FLOOR != 0 {
                    if h <= a.max_fall && below & DANGER == 0 {
                        out.push((q, Mv::Fall, WALK + head + fall_cost(h)));
                    }
                    break;
                }
                if below & PASS == 0 || h >= a.max_fall && below & WATER == 0 {
                    break;
                }
            }
        }
        // Over a gap.
        if a.parkour
            && under & FLOOR != 0
            && t.at(g, add(m, DOWN)) & FLOOR == 0
            && mc & PASS != 0
            && t.at(g, add(m, UP)) & PASS != 0
            && t.at(g, add(m, [0, 2, 0])) & PASS != 0
            && t.at(g, add(n, [0, 2, 0])) & PASS != 0
        {
            for gap in 2..=4 {
                let q = add(n, d.map(|v| v * gap));
                let qc = t.at(g, q);
                let clear = qc & PASS != 0
                    && qc & (LAVA | DANGER) == 0
                    && t.at(g, add(q, UP)) & PASS != 0
                    && t.at(g, add(q, [0, 2, 0])) & PASS != 0;
                if !clear {
                    break;
                }
                let below = t.at(g, add(q, DOWN));
                if below & FLOOR != 0 && below & DANGER == 0 {
                    out.push((q, Mv::Leap, f64::from(gap) * SPRINT + 6.0));
                    break;
                }
            }
        }
    }
    // Corners: nothing to clear, nothing to place.
    if under & FLOOR != 0 {
        for (dx, dz) in [(1, 1), (1, -1), (-1, 1), (-1, -1)] {
            let m = add(n, [dx, 0, dz]);
            let free = |g: &Game<'_>, t: &mut Tables, p: P| t.at(g, p) & PASS != 0 && t.at(g, add(p, UP)) & PASS != 0;
            if loaded(g, m)
                && free(g, t, m)
                && free(g, t, add(n, [dx, 0, 0]))
                && free(g, t, add(n, [0, 0, dz]))
                && t.at(g, m) & WATER == 0
            {
                let below = t.at(g, add(m, DOWN));
                if below & FLOOR != 0 && below & DANGER == 0 {
                    out.push((m, Mv::Diag, WALK * std::f64::consts::SQRT_2));
                }
            }
        }
    }
    // Straight up: swimming, climbing, or pillaring on placed blocks.
    let up = add(n, UP);
    if let Some(above) = clear_cost(g, t, a, add(n, [0, 2, 0])) {
        let upc = t.at(g, up);
        if swimming && upc & PASS != 0 {
            out.push((up, Mv::Swim, SWIM + above));
        } else if here & CLIMB != 0 && upc & PASS != 0 {
            out.push((up, Mv::Climb, LADDER + above));
        } else if upc & CLIMB != 0 {
            out.push((up, Mv::Climb, LADDER + above));
        } else if a.place && standing && here & PASS != 0 && upc & PASS != 0 {
            out.push((up, Mv::Pillar, JUMP + a.place_cost + above));
        }
    }
    // Straight down: swimming, or digging out the floor.
    let down = add(n, DOWN);
    if under & WATER != 0 {
        if !a.safe {
            out.push((down, Mv::Swim, SWIM));
        }
    } else if a.down && n[1] > -64 && under & BREAK != 0 {
        let landing = t.at(g, add(down, DOWN));
        let shaky = a.safe && landing & FALLS != 0;
        if (landing & FLOOR != 0 && landing & DANGER == 0 && !shaky || landing & WATER != 0)
            && let Some(cost) = clear_cost(g, t, a, down)
        {
            out.push((down, Mv::Dig, cost + fall_cost(1)));
        }
    }
}

// The search gets a slice of each tick; after PLAN_LIMIT in all it walks the
// best part-way path it has. 12 ms a tick turns 1.5 s of searching into about
// six seconds of standing still, not the half minute 5 ms gave.
const PLAN_LIMIT: Duration = Duration::from_millis(1500);
const SLICE: Duration = Duration::from_millis(12);

fn search(g: &mut Game<'_>, t: &mut Tables, plan: &mut Plan) -> Searched {
    let start = Instant::now();
    let mut out = Vec::new();
    let allow = plan.allow;
    loop {
        if plan.expanded.is_multiple_of(32) && start.elapsed() > SLICE {
            plan.spent += start.elapsed();
            if plan.spent > PLAN_LIMIT {
                return partial(plan);
            }
            return Searched::Running;
        }
        let Some(e) = plan.open.pop() else {
            return partial(plan);
        };
        if plan.seen.get(&e.p).is_some_and(|s| s.0 < e.g) {
            continue;
        }
        if plan.goal.reached(g, t, e.p) {
            return Searched::Found(plan.path_to(e.p), e.g);
        }
        plan.expanded += 1;
        out.clear();
        let mv = plan.seen.get(&e.p).map_or(Mv::Start, |s| s.2);
        let bridged = matches!(mv, Mv::Walk | Mv::Up | Mv::Pillar) && t.at(g, add(e.p, [0, -1, 0])) & FLOOR == 0;
        expand(g, t, &allow, e.p, bridged, &mut out);
        for &(m, mv, cost) in &out {
            if t.banned.contains(&(e.p, m)) {
                continue;
            }
            // Survival: never stand where lava touches the body, the feet's floor or
            // the head's ceiling; a stumble, a knockback or a flow is enough.
            if allow.safe && (lava_near(g, t, m) || loose(g, t, add(m, [0, -1, 0]))) {
                continue;
            }
            // Survival: open caves are where mobs spawn and shooters see from far
            // away; a dug 1x2 tunnel is safer, so a cave step costs more.
            let cost = if allow.safe { cost + exposure(g, t, m) * allow.cave } else { cost };
            let next = e.g + cost;
            if plan.seen.get(&m).is_some_and(|s| s.0 <= next) {
                continue;
            }
            plan.seen.insert(m, (next, e.p, mv));
            let h = plan.goal.h(m);
            plan.open.push(Entry { f: next + h, g: next, p: m });
            let score = h + next / 3.0;
            if score < plan.best.1 {
                plan.best = (m, score);
            }
        }
    }
}

fn partial(plan: &Plan) -> Searched {
    let path = plan.path_to(plan.best.0);
    if path.len() >= 2 {
        let cost = plan.seen.get(&plan.best.0).map_or(0.0, |s| s.0);
        Searched::Partial(path, cost)
    } else {
        Searched::Nothing
    }
}

// ----------------------------------------------------------------- the state

enum Proc {
    Goto(Goal),
    Mine { names: Vec<String>, want: Option<u32>, done: u32 },
    Follow { who: String, last: Option<P> },
    Explore { center: (i32, i32), index: i32, goal: Option<(i32, i32)> },
    Farm { range: i32 },
    Tunnel { dir: [i32; 2], left: i32, width: i32, height: i32 },
}

#[derive(Default)]
struct Rt {
    t: Tables,
    process: Option<Proc>,
    paused: bool,
    /// Paused for a player nearby, so it resumes when they leave.
    auto_paused: bool,
    plan: Option<Plan>,
    path: Vec<Step>,
    at: usize,
    /// The path only goes part of the way.
    partial: bool,
    cost: f64,
    /// The last tick the path made progress, and how many replans since.
    moved: u64,
    fails: u32,
    /// What the keys were told last tick.
    keys: Vec<KeyCode>,
    mining: Option<P>,
    wait: u32,
    /// The slot the player held when pathing started.
    slot: Option<usize>,
    /// The saved goal for `#path`.
    saved: Option<Goal>,
    // Mining
    known: HashSet<P>,
    scan: VecDeque<(i32, i32)>,
    scan_at: Option<(i32, i32, i32)>,
    scanned: HashSet<(i32, i32)>,
    scan_names: Vec<String>,
    digging: Option<P>,
    /// The tick the current ore's digging began.
    dig_at: u64,
    /// The feet cell and path step, and the tick either last changed.
    still: (P, usize, u64),
    /// The tick the mine process last got a block (or started).
    mined_at: u64,
    loot: Option<(P, u32)>,
    avoid: HashSet<P>,
    tunnel_at: Option<P>,
    // Farming
    crops: Vec<P>,
    crops_at: u64,
    replant: Option<(P, &'static str)>,
    status: String,
    tick: u64,
    // Survival
    eat: Option<u32>,
    seal: Option<u64>,
    flee: Option<([f64; 3], u64)>,
    hazard: HashSet<P>,
    last_hp: f32,
    hurt_at: u64,
    disturbed: bool,
    worn_out: bool,
    fight: Option<(u32, u64, u64)>,
    ignore: HashMap<u32, u64>,
    /// After 3 minutes walled in with a shooter still about: until this tick a
    /// shooter is fought (at full health) rather than walled off again.
    brave_until: u64,
    /// Tunnelling away from a creeper that camps outside the wall: where to, since when.
    escape: Option<(Goal, u64)>,
    /// A bunker being dug or sat in: the feet cell it was started from. The pit is
    /// the three cells under it; the feet end at the bottom and the top one is capped.
    bunker: Option<P>,
    /// Walking to a spot a bunker can be dug at, and since when.
    to_bunker: Option<(P, u64)>,
    /// The nearest hostiles last seen, for the debug log.
    mobs_note: String,
    /// While escaping: the cell last stood in, and the cells left behind still to fill.
    left_from: Option<P>,
    behind: Vec<P>,
    /// Why ores were left out, counted, for the debug log.
    why: HashMap<&'static str, u32>,
}

thread_local! {
    static RT: RefCell<Rt> = RefCell::new(Rt::default());
}

fn on_ground(g: &Game<'_>) -> bool {
    g.player.as_ref().is_some_and(|p| p.ground.on_ground)
}

fn feet_cell(g: &Game<'_>, t: &mut Tables) -> Option<P> {
    let p = g.player.as_ref()?;
    let mut c = [p.position.x.floor() as i32, (p.position.y + 0.01).floor() as i32, p.position.z.floor() as i32];
    // On a slab or soul sand the feet are inside the block they stand on.
    if t.at(g, c) & PASS == 0 {
        c[1] += 1;
    }
    Some(c)
}

fn allow(s: &State) -> Allow {
    // The fall limit is the player's setting even with NoFall or GodMode on: those
    // make a fall harmless, not a good way to reach the goal (it strands the walk).
    Allow {
        brk: s.b("Baritone", "AllowBreak"),
        place: s.b("Baritone", "AllowPlace"),
        down: s.b("Baritone", "AllowDownward"),
        parkour: s.b("Baritone", "AllowParkour"),
        max_fall: s.n("Baritone", "MaxFall") as i32,
        place_cost: s.n("Baritone", "PlaceCost"),
        break_mult: s.n("Baritone", "BreakCost"),
        safe: s.b("Baritone", "Survival"),
        cave: 10.0,
    }
}

fn throwaway_slot(g: &mut Game<'_>, s: &State) -> Option<usize> {
    let list = s.list("Baritone", "Throwaway");
    if let Some(slot) = core::hotbar_find(g, |i| list.iter().any(|l| l == i)) {
        return Some(slot);
    }
    None
}

/// Moves a throwaway block from the inventory to the hotbar when none is there.
fn stock_hotbar(g: &mut Game<'_>, s: &State) {
    let list = s.list("Baritone", "Throwaway");
    if core::hotbar_find(g, |i| list.iter().any(|l| l == i)).is_none()
        && core::inventory_menu(g)
        && let Some(slot) = core::inventory_find(g, |i| list.iter().any(|l| l == i))
    {
        core::swap_slot(g, slot, 8);
    }
}

fn press(g: &mut Game<'_>, rt: &mut Rt, want: &[KeyCode]) {
    for k in std::mem::take(&mut rt.keys) {
        if !want.contains(&k) && !g.physical_keys.contains(&k) {
            g.keys.remove(&k);
        }
    }
    for k in want {
        g.keys.insert(*k);
    }
    rt.keys = want.to_vec();
}

fn stop_hands(g: &mut Game<'_>, rt: &mut Rt) {
    if rt.mining.take().is_some() {
        core::hold(g, false, false);
    }
}

fn clear_path(rt: &mut Rt) {
    rt.plan = None;
    rt.path.clear();
    rt.at = 0;
    rt.partial = false;
}

fn stop(g: &mut Game<'_>, rt: &mut Rt) {
    rt.process = None;
    rt.paused = false;
    clear_path(rt);
    press(g, rt, &[]);
    stop_hands(g, rt);
    rt.digging = None;
    rt.loot = None;
    rt.replant = None;
    stop_eating(g, rt);
    rt.seal = None;
    rt.bunker = None;
    rt.to_bunker = None;
    rt.escape = None;
    rt.fight = None;
    rt.worn_out = false;
    rt.status.clear();
    if let Some(slot) = rt.slot.take() {
        g.selected = slot;
    }
}

pub fn toggled(g: &mut Game<'_>, _s: &mut State, name: &str, on: bool) {
    if name == "Baritone" && !on {
        RT.with(|rt| stop(g, &mut rt.borrow_mut()));
    }
}

/// Set when the mod has turned `pauseOnLostFocus` off (in memory only, never on
/// disk) because Baritone is running, so it knows to turn it back on.
static PAUSE_OFF_BY_US: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Called at the head of the game's `pause_if_inactive`, every frame. While
/// Baritone runs, a window that loses focus (minimised, alt-tabbed) doesn't open
/// the pause menu; when Baritone stops, the option the player chose comes back.
/// The player's options file is never written.
pub fn pause_guard(g: &mut Game<'_>) {
    use horizon_client::options_file::V;
    use std::sync::atomic::Ordering;
    // The server thread ticks this too (`native_window` is false there), and it
    // has no Baritone of its own: only the window's tick decides.
    if !g.native_window() {
        return;
    }
    let running = RT.with(|rt| rt.borrow().process.is_some());
    let ours = PAUSE_OFF_BY_US.load(Ordering::Relaxed);
    if running && !ours && g.settings.file.bool("pauseOnLostFocus") {
        g.settings.file.set("pauseOnLostFocus", V::Bool(false));
        PAUSE_OFF_BY_US.store(true, Ordering::Relaxed);
    } else if !running && ours {
        g.settings.file.set("pauseOnLostFocus", V::Bool(true));
        PAUSE_OFF_BY_US.store(false, Ordering::Relaxed);
    }
}

/// What the HUD shows while pathing.
pub fn status() -> Option<String> {
    RT.with(|rt| {
        let rt = rt.borrow();
        rt.process.as_ref()?;
        Some(if rt.paused { "Baritone: paused".to_owned() } else { format!("Baritone: {}", rt.status) })
    })
}

fn finish(g: &mut Game<'_>, s: &mut State, rt: &mut Rt, message: String) {
    s.say(message);
    stop(g, rt);
    s.turn_off("Baritone");
}

// ---------------------------------------------------------------- the driving

fn aim_block(g: &mut Game<'_>, pos: P) -> bool {
    let Some(eye) = g.eye_position() else {
        return false;
    };
    let c = mid(pos);
    let mut faces = [[0, 1, 0], [0, -1, 0], [1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1]];
    faces.sort_by(|a, b| {
        let point = |f: &P| [0, 1, 2].map(|i| c[i] + f64::from(f[i]) * 0.45);
        core::dist(eye, point(a)).total_cmp(&core::dist(eye, point(b)))
    });
    let reach = g.reach();
    for f in faces {
        for inset in [0.45, 0.3, 0.0] {
            let point = [0, 1, 2].map(|i| c[i] + f64::from(f[i]) * inset);
            let (yaw, pitch) = core::angles(eye, point);
            let dir = super::view_vector(yaw, pitch);
            if g.clip_ray(eye, dir, reach).is_some_and(|hit| hit.pos == pos) {
                g.yaw += core::wrap(yaw - g.yaw);
                g.pitch = pitch;
                return true;
            }
        }
    }
    false
}

/// Mines `cell`: looks at it and holds the attack. False once it can be walked through.
fn mine(g: &mut Game<'_>, s: &State, rt: &mut Rt, cell: P) -> bool {
    if rt.t.at(g, cell) & PASS != 0 {
        return false;
    }
    if rt.t.at(g, cell) & BREAK == 0 {
        return false;
    }
    if rt.slot.is_none() {
        rt.slot = Some(g.selected);
    }
    if s.b("Baritone", "AutoTool") {
        g.selected = best_tool(g, cell).0;
    }
    if aim_block(g, cell) {
        core::hold(g, false, true);
        rt.mining = Some(cell);
        rt.moved = rt.tick;
    } else if let Some(other) = in_the_way(g, &mut rt.t, cell)
        && aim_block(g, other)
    {
        // Snow, grass or a flower in front of it takes the crosshair: that first.
        core::hold(g, false, true);
        rt.mining = Some(other);
        rt.moved = rt.tick;
    } else if rt.mining.take().is_some() {
        // Out of sight: no progress, so the walk gives up on it and plans again.
        core::hold(g, false, false);
    }
    true
}

/// A thin block (snow layer, grass, a flower) the crosshair meets before `cell`.
fn in_the_way(g: &Game<'_>, t: &mut Tables, cell: P) -> Option<P> {
    let eye = g.eye_position()?;
    let c = mid(cell);
    let d = core::dist(eye, c);
    let dir = [0, 1, 2].map(|a| (c[a] - eye[a]) / d.max(1e-6));
    let hit = g.clip_ray(eye, dir, g.reach())?;
    let k = t.at(g, hit.pos);
    (hit.pos != cell && k & PASS != 0 && k & BREAK != 0 && k & (WATER | LAVA) == 0).then_some(hit.pos)
}

/// Places a block in `target` against a neighbour the crosshair sees from here.
fn place(g: &mut Game<'_>, s: &State, rt: &mut Rt, target: P) -> bool {
    if rt.wait > 0 {
        return false;
    }
    let Some(slot) = throwaway_slot(g, s) else {
        return false;
    };
    // Never into a cell the body is in: that would bury the player.
    if g.player.as_ref().is_some_and(|p| body_in([p.position.x, p.position.y, p.position.z], target)) {
        return false;
    }
    if rt.slot.is_none() {
        rt.slot = Some(g.selected);
    }
    if core::place(g, target, slot) {
        rt.wait = 3;
        return true;
    }
    false
}

/// Whether a player standing at `pos` (0.6 wide, 1.8 tall) is partly inside `cell`.
fn body_in(pos: [f64; 3], cell: P) -> bool {
    let (x, y, z) = (f64::from(cell[0]), f64::from(cell[1]), f64::from(cell[2]));
    pos[0] + 0.3 > x && pos[0] - 0.3 < x + 1.0 && pos[2] + 0.3 > z && pos[2] - 0.3 < z + 1.0 && pos[1] < y + 1.0 && pos[1] + 1.8 > y
}

fn yaw_to(g: &mut Game<'_>, at: [f64; 3], pos: [f64; 3]) {
    let (dx, dz) = (at[0] - pos[0], at[2] - pos[2]);
    if dx.abs() + dz.abs() > 1e-3 {
        let yaw = (-dx).atan2(dz).to_degrees() as f32;
        g.yaw += core::wrap(yaw - g.yaw);
    }
}

/// Drives one tick along the path. True when the path is used up.
fn drive(g: &mut Game<'_>, s: &State, rt: &mut Rt) -> bool {
    if rt.wait > 0 {
        rt.wait -= 1;
    }
    let Some((pos, on_ground)) = g.player.as_ref().map(|p| ([p.position.x, p.position.y, p.position.z], p.ground.on_ground)) else {
        return true;
    };
    let Some(cell) = feet_cell(g, &mut rt.t) else {
        return true;
    };
    // Reached the next step, or one further on?
    let look = (rt.at + 8).min(rt.path.len());
    for i in (rt.at + 1..look).rev() {
        if rt.path[i].pos == cell {
            // A pillar step counts once standing on the placed block: mid-jump the
            // feet pass into the cell before the block under them is there.
            let pillar = rt.path[i].mv == Mv::Pillar;
            let settled = on_ground || rt.t.at(g, cell) & (WATER | CLIMB) != 0;
            if settled || (i + 1 < rt.path.len() && !pillar) {
                rt.at = i;
                rt.moved = rt.tick;
                stop_hands(g, rt);
                break;
            }
        }
    }
    if rt.at + 1 >= rt.path.len() {
        press(g, rt, &[]);
        stop_hands(g, rt);
        return true;
    }
    let (from, next) = (rt.path[rt.at], rt.path[rt.at + 1]);
    let more = rt.path.get(rt.at + 2).copied();
    // Off the path, or going nowhere: search again. Going nowhere is judged twice:
    // no action for 5 s, or (whatever the actions claim, a placement the game
    // refused looks like one) the same cell and step for 20 s. That step is then
    // left out of the next plans.
    let far = (cell[0] - from.pos[0]).abs().max((cell[2] - from.pos[2]).abs()) > 2
        && (cell[0] - next.pos[0]).abs().max((cell[2] - next.pos[2]).abs()) > 2;
    if rt.still.0 != cell || rt.still.1 != rt.at {
        rt.still = (cell, rt.at, rt.tick);
    }
    let stalled = rt.tick.saturating_sub(rt.still.2) > 400;
    if far || stalled || rt.tick.saturating_sub(rt.moved) > 100 {
        if !far {
            rt.t.banned.insert((from.pos, next.pos));
        }
        rt.still.2 = rt.tick;
        rt.fails += 1;
        rt.moved = rt.tick;
        clear_path(rt);
        press(g, rt, &[]);
        stop_hands(g, rt);
        return true;
    }
    let target = center(next.pos);
    let up = [0, 1, 0];
    let mut keys: Vec<KeyCode> = Vec::new();
    // What stands in the way goes first, the top first so nothing falls on us.
    let mut clear: Vec<P> = Vec::new();
    match next.mv {
        Mv::Walk | Mv::Diag | Mv::Swim => {
            clear.push(add(next.pos, up));
            clear.push(next.pos);
        }
        Mv::Up => {
            clear.push(add(from.pos, [0, 2, 0]));
            clear.push(add(next.pos, up));
            clear.push(next.pos);
        }
        Mv::Fall => {
            clear.push([next.pos[0], from.pos[1] + 1, next.pos[2]]);
            clear.push([next.pos[0], from.pos[1], next.pos[2]]);
        }
        Mv::Pillar | Mv::Climb => clear.push(add(from.pos, [0, 2, 0])),
        Mv::Dig => clear.push(next.pos),
        Mv::Leap | Mv::Start => {}
    }
    for c in clear {
        if mine(g, s, rt, c) {
            press(g, rt, &[]);
            return false;
        }
    }
    stop_hands(g, rt);
    let horiz = ((target[0] - pos[0]).powi(2) + (target[2] - pos[2]).powi(2)).sqrt();
    let here = rt.t.at(g, cell);
    let in_water = here & WATER != 0;
    yaw_to(g, target, pos);
    match next.mv {
        Mv::Walk | Mv::Diag | Mv::Start => {
            let below = add(next.pos, [0, -1, 0]);
            if rt.t.at(g, below) & FLOOR == 0 {
                // Bridging: sneak up to the edge and stand there, placing under the next
                // step, so the walk never runs off it; only walk on while too far to reach.
                let gap = ((center(below)[0] - pos[0]).powi(2) + (center(below)[2] - pos[2]).powi(2)).sqrt();
                keys.push(KeyCode::ShiftLeft);
                if gap > 2.2 {
                    keys.push(KeyCode::KeyW);
                }
                if place(g, s, rt, below) {
                    rt.moved = rt.tick;
                }
            } else {
                keys.push(KeyCode::KeyW);
                let straight = more.is_some_and(|m| {
                    (m.pos[0] - next.pos[0], m.pos[2] - next.pos[2]) == (next.pos[0] - from.pos[0], next.pos[2] - from.pos[2])
                        && matches!(m.mv, Mv::Walk | Mv::Diag)
                });
                let hungry = s.b("Baritone", "Survival") && health(g).1 <= 18;
                if s.b("Baritone", "AllowSprint") && straight && !in_water && !hungry {
                    keys.push(KeyCode::ControlLeft);
                }
                if in_water {
                    keys.push(KeyCode::Space);
                }
            }
        }
        Mv::Swim => {
            keys.push(KeyCode::KeyW);
            if next.pos[1] >= from.pos[1] {
                keys.push(KeyCode::Space);
            }
            if horiz < 0.2 {
                keys.retain(|k| *k != KeyCode::KeyW);
            }
        }
        Mv::Up => {
            let block = [next.pos[0], from.pos[1], next.pos[2]];
            if rt.t.at(g, block) & FLOOR == 0 {
                if body_in(pos, block) {
                    // Standing partly where the block goes: the game won't place it.
                    // Back to the middle of this cell first.
                    yaw_to(g, center(from.pos), pos);
                    press(g, rt, &[KeyCode::KeyW, KeyCode::ShiftLeft]);
                    return false;
                }
                if place(g, s, rt, block) {
                    rt.moved = rt.tick;
                }
                press(g, rt, &[KeyCode::ShiftLeft]);
                return false;
            }
            keys.push(KeyCode::KeyW);
            if on_ground && horiz < 1.6 || in_water {
                keys.push(KeyCode::Space);
            }
        }
        Mv::Fall => {
            if horiz > 0.15 {
                keys.push(KeyCode::KeyW);
            }
        }
        Mv::Climb => {
            keys.push(KeyCode::Space);
            if horiz > 0.1 {
                keys.push(KeyCode::KeyW);
            }
        }
        Mv::Pillar => {
            // Centred, jump, and look down; a block under the feet at the top of the jump.
            let middle = center(from.pos);
            let off = ((middle[0] - pos[0]).powi(2) + (middle[2] - pos[2]).powi(2)).sqrt();
            if off > 0.2 && on_ground {
                yaw_to(g, middle, pos);
                keys.push(KeyCode::KeyW);
                keys.push(KeyCode::ShiftLeft);
            } else {
                g.pitch = 90.0;
                keys.push(KeyCode::Space);
                keys.push(KeyCode::ShiftLeft);
                if pos[1] >= f64::from(from.pos[1]) + 1.0 && place(g, s, rt, from.pos) {
                    rt.moved = rt.tick;
                }
            }
        }
        Mv::Dig => {}
        Mv::Leap => {
            keys.push(KeyCode::KeyW);
            keys.push(KeyCode::ControlLeft);
            let start = center(from.pos);
            let run = ((start[0] - pos[0]).powi(2) + (start[2] - pos[2]).powi(2)).sqrt();
            if on_ground && run > 0.35 {
                keys.push(KeyCode::Space);
            }
        }
    }
    press(g, rt, &keys);
    false
}

/// Keeps a path going towards `goal`: searches in slices, then drives. True once
/// the search can't get any closer and there's nothing left to walk.
fn walk(g: &mut Game<'_>, s: &State, rt: &mut Rt, goal: &Goal) -> bool {
    if !rt.path.is_empty() {
        if !drive(g, s, rt) {
            return false;
        }
        rt.path.clear();
        rt.at = 0;
    }
    if rt.fails > 6 {
        return true;
    }
    if rt.plan.is_none() {
        let Some(cell) = feet_cell(g, &mut rt.t) else {
            return true;
        };
        // Never plan from mid-air: a fall isn't over until it lands, and walking
        // on from there can carry the player over a drop.
        if !on_ground(g) && rt.t.at(g, cell) & (WATER | CLIMB) == 0 {
            press(g, rt, &[]);
            return false;
        }
        rt.t.brk.clear();
        stock_hotbar(g, s);
        let mut a = allow(s);
        if throwaway_slot(g, s).is_none() {
            a.place = false;
        }
        // Getting away from mobs: through rock, nearly never through open cave.
        if rt.escape.is_some() {
            a.cave = 200.0;
        }
        rt.plan = Some(Plan::new(goal.clone(), a, cell));
    }
    let Some(mut plan) = rt.plan.take() else {
        return true;
    };
    let result = search(g, &mut rt.t, &mut plan);
    match result {
        Searched::Running => {
            rt.plan = Some(plan);
            press(g, rt, &[]);
            false
        }
        Searched::Found(path, cost) => {
            rt.partial = false;
            start_path(rt, path, cost);
            false
        }
        Searched::Partial(path, cost) => {
            rt.partial = true;
            start_path(rt, path, cost);
            false
        }
        Searched::Nothing => {
            press(g, rt, &[]);
            true
        }
    }
}

fn start_path(rt: &mut Rt, path: Vec<Step>, cost: f64) {
    rt.path = path;
    rt.at = 0;
    rt.cost = cost;
    rt.moved = rt.tick;
    rt.still.2 = rt.tick;
    rt.plan = None;
}

// -------------------------------------------------------------- the processes

/// `x:y` names as a set: `diamond` also means the deepslate ore, `*_log` any log.
fn block_names(word: &str) -> Vec<String> {
    let word = word.trim_start_matches("minecraft:").to_ascii_lowercase();
    let ore = |n: &str| vec![format!("minecraft:{n}_ore"), format!("minecraft:deepslate_{n}_ore")];
    match word.as_str() {
        "diamond" | "diamonds" => ore("diamond"),
        "iron" => ore("iron"),
        "gold" => ore("gold"),
        "coal" => ore("coal"),
        "copper" => ore("copper"),
        "emerald" | "emeralds" => ore("emerald"),
        "lapis" => ore("lapis"),
        "redstone" => ore("redstone"),
        "diamond_ore" => ore("diamond"),
        "iron_ore" => ore("iron"),
        "gold_ore" => ore("gold"),
        "coal_ore" => ore("coal"),
        "copper_ore" => ore("copper"),
        "emerald_ore" => ore("emerald"),
        "lapis_ore" => ore("lapis"),
        "redstone_ore" => ore("redstone"),
        "debris" | "netherite" => vec!["minecraft:ancient_debris".to_owned()],
        "quartz" => vec!["minecraft:nether_quartz_ore".to_owned()],
        "log" | "logs" | "wood" => vec!["*_log".to_owned()],
        w if w.contains('*') => vec![w.to_owned()],
        w => vec![format!("minecraft:{w}")],
    }
}
fn name_matches(names: &[String], name: &str) -> bool {
    names.iter().any(|n| match n.strip_prefix('*') {
        Some(tail) => name.ends_with(tail),
        None => n == name,
    })
}

/// Reads more of the world for `names`, a budget of blocks a tick, nearest chunks first.
fn scan(g: &Game<'_>, rt: &mut Rt, names: &[String], feet: P) {
    if rt.scan_names != names {
        rt.scan_names = names.to_vec();
        rt.known.clear();
        rt.scan.clear();
        rt.scanned.clear();
        rt.scan_at = None;
    }
    let (cx, cz) = (feet[0] >> 4, feet[2] >> 4);
    if rt.scan.is_empty() && rt.scan_at.is_none() {
        let mut order: Vec<(i32, i32)> = Vec::new();
        for dx in -12..=12 {
            for dz in -12..=12 {
                let c = (cx + dx, cz + dz);
                if !rt.scanned.contains(&c) {
                    order.push(c);
                }
            }
        }
        order.sort_by_key(|(x, z)| (x - cx).abs().max((z - cz).abs()));
        rt.scan.extend(order);
    }
    let facts = g.res.facts();
    let mut budget = 40_000;
    while budget > 0 {
        let (x, z, mut y) = match rt.scan_at {
            Some(at) => at,
            None => {
                let Some((x, z)) = rt.scan.pop_front() else {
                    return;
                };
                if !g.world.chunks.contains_key(&(x, z)) {
                    continue;
                }
                (x, z, -64)
            }
        };
        let top = 320;
        while y < top && budget > 0 {
            for bx in 0..16 {
                for bz in 0..16 {
                    let p = [x * 16 + bx, y, z * 16 + bz];
                    let state = g.world.block(p[0], p[1], p[2]);
                    if !facts.is_air(state) {
                        let name = &facts.get(state).name;
                        if name_matches(names, name) {
                            rt.known.insert(p);
                        } else if &**name == "minecraft:sculk_shrieker" || &**name == "minecraft:spawner" {
                            // A warden comes from shriekers; a spawner (cave spiders in
                            // mineshafts, zombies, skeletons) keeps making mobs.
                            rt.hazard.insert(p);
                        }
                    }
                }
            }
            budget -= 256;
            y += 1;
        }
        if y >= top {
            rt.scanned.insert((x, z));
            rt.scan_at = None;
        } else {
            rt.scan_at = Some((x, z, y));
        }
    }
}

fn mine_tick(g: &mut Game<'_>, s: &mut State, rt: &mut Rt, names: Vec<String>, want: Option<u32>, mut done: u32) {
    let Some(feet) = feet_cell(g, &mut rt.t) else {
        return;
    };
    scan(g, rt, &names, feet);
    let matching = |g: &Game<'_>, p: P| {
        let state = g.world.block(p[0], p[1], p[2]);
        name_matches(&names, &g.res.facts().get(state).name)
    };
    // Digging one out.
    if let Some(block) = rt.digging {
        if !matching(g, block) {
            stop_hands(g, rt);
            rt.digging = None;
            rt.known.remove(&block);
            rt.loot = Some((block, 0));
            rt.mined_at = rt.tick;
            done += 1;
        } else if feet == block || (!beside(feet, block) && on_ground(g)) {
            // Moved off it (in the air, mid-jump, it stays the one).
            stop_hands(g, rt);
            rt.digging = None;
        } else if s.b("Baritone", "Survival") && lava_beside(g, &mut rt.t, block) {
            // Lava would pour out of it onto us.
            stop_hands(g, rt);
            rt.digging = None;
            rt.avoid.insert(block);
        } else if rt.tick.saturating_sub(rt.dig_at) > 300 {
            // Fifteen seconds and still there (out of sight, out of reach): leave it.
            *rt.why.entry("dig timeout").or_default() += 1;
            stop_hands(g, rt);
            rt.digging = None;
            rt.avoid.insert(block);
        } else {
            press(g, rt, &[]);
            let state = g.world.block(block[0], block[1], block[2]);
            let name = g.res.facts().get(state).name.clone();
            let needs = g.res.facts().get(state).requires_tool;
            if needs {
                let keep = g.selected;
                let slot = best_tool(g, block).0;
                g.selected = slot;
                let can = g.items().correct_for_drops(g.held(), &name);
                g.selected = keep;
                if !can {
                    let label = core::pretty(&name);
                    finish(g, s, rt, format!("No tool in your hotbar that harvests {label}"));
                    return;
                }
            }
            mine(g, s, rt, block);
            rt.status = format!("mining {}", core::pretty(&name));
            rt.process = Some(Proc::Mine { names, want, done });
            return;
        }
    }
    if want.is_some_and(|w| done >= w) {
        finish(g, s, rt, format!("Mined {done} blocks"));
        return;
    }
    // Picking up what fell out of it.
    if let Some((at, age)) = rt.loot {
        let eye = center(feet);
        let drop = g
            .dropped
            .iter()
            .filter(|d| core::dist([d.position.x, d.position.y, d.position.z], mid(at)) < 5.0)
            .map(|d| [d.position.x.floor() as i32, d.position.y.floor() as i32, d.position.z.floor() as i32])
            .min_by(|a, b| core::dist(eye, center(*a)).total_cmp(&core::dist(eye, center(*b))));
        match drop {
            Some(item) if age < 100 => {
                rt.loot = Some((at, age + 1));
                let goal = Goal::Block(item);
                if !goal.reached(g, &mut rt.t, feet) {
                    rt.status = "picking up the drop".to_owned();
                    if rt.path.last().is_none_or(|l| l.pos != item) && rt.plan.is_none() {
                        clear_path(rt);
                    }
                    if !walk(g, s, rt, &goal) {
                        rt.process = Some(Proc::Mine { names, want, done });
                        return;
                    }
                }
                clear_path(rt);
                rt.loot = None;
            }
            _ if age < 12 => {
                // Give the item a moment to appear.
                rt.loot = Some((at, age + 1));
                press(g, rt, &[]);
                rt.process = Some(Proc::Mine { names, want, done });
                return;
            }
            _ => rt.loot = None,
        }
    }
    // The nearest ones still there.
    // The 64 nearest first (thousands can be known), then the checks on those.
    let c = center(feet);
    let avoid = std::mem::take(&mut rt.avoid);
    let mut list: Vec<P> = rt.known.iter().copied().filter(|p| !avoid.contains(p)).collect();
    rt.avoid = avoid;
    list.sort_by(|a, b| core::dist(c, center(*a)).total_cmp(&core::dist(c, center(*b))));
    list.truncate(64);
    let (still, gone): (Vec<P>, Vec<P>) = list.into_iter().partition(|p| matching(g, *p));
    for p in gone {
        rt.known.remove(&p);
    }
    let mut list = still;
    if s.b("Baritone", "Survival") {
        let far = |a: P, b: P, r: i32| (a[0] - b[0]).abs() > r || (a[1] - b[1]).abs() > r || (a[2] - b[2]).abs() > r;
        let hazard: Vec<P> = rt.hazard.iter().copied().collect();
        let flee = rt.flee.map(|f| f.0.map(|v| v.floor() as i32));
        let mut keep = Vec::new();
        for p in list {
            let bad = if lava_beside(g, &mut rt.t, p) {
                Some("lava")
            } else if rt.t.at(g, add(p, [0, 1, 0])) & FALLS != 0 {
                // Mined, the sand or gravel on it pours into the hole and the
                // cells around it, which the next step may walk into.
                Some("gravel above")
            } else if hazard.iter().any(|h| !far(*h, p, 24)) {
                Some("shrieker or spawner")
            } else if flee.is_some_and(|w| !far(w, p, 48)) {
                Some("warden")
            } else {
                None
            };
            if let Some(why) = bad {
                *rt.why.entry(why).or_default() += 1;
                rt.avoid.insert(p);
            } else {
                keep.push(p);
            }
        }
        list = keep;
    }
    let c = center(feet);
    list.sort_by(|a, b| core::dist(c, center(*a)).total_cmp(&core::dist(c, center(*b))));
    list.truncate(12);
    // Only standing on something: from mid-jump (pillaring up a shaft) the walk
    // finishes its step first, and mining in the air is five times slower.
    let settled = on_ground(g) || rt.t.at(g, feet) & (WATER | CLIMB) != 0;
    // Not from straight under an ore with sand or gravel on it: mined, it comes
    // down on the head.
    let under_falls = |t: &mut Tables, b: P| feet[0] == b[0] && feet[2] == b[2] && feet[1] < b[1] && t.at(g, add(b, [0, 1, 0])) & FALLS != 0;
    let pick = list.iter().copied().find(|b| beside(feet, *b) && !under_falls(&mut rt.t, *b));
    if pick.is_none() {
        // The goal counts these as reached: leave them out, or the walk stops here.
        let bad: Vec<P> = list.iter().copied().filter(|b| beside(feet, *b) && under_falls(&mut rt.t, *b)).collect();
        if !bad.is_empty() {
            for b in &bad {
                rt.avoid.insert(*b);
            }
            *rt.why.entry("gravel above").or_default() += bad.len() as u32;
            list.retain(|b| !bad.contains(b));
        }
    }
    if settled && let Some(block) = pick {
        clear_path(rt);
        rt.digging = Some(block);
        rt.dig_at = rt.tick;
        rt.process = Some(Proc::Mine { names, want, done });
        return;
    }
    if list.is_empty() {
        if rt.scan.is_empty() && rt.scan_at.is_none() {
            // Nothing in the loaded world: walk on and read more of it.
            rt.status = "looking for it".to_owned();
            let ahead = Goal::Xz(feet[0] + 64, feet[2] + 64);
            if rt.path.is_empty() && rt.plan.is_none() {
                rt.scanned.retain(|&(x, z)| (x - (feet[0] >> 4)).abs() < 16 && (z - (feet[2] >> 4)).abs() < 16);
            }
            walk(g, s, rt, &ahead);
        } else {
            rt.status = format!("reading the world ({} chunks left)", rt.scan.len());
            press(g, rt, &[]);
        }
        rt.process = Some(Proc::Mine { names, want, done });
        return;
    }
    // The last resort against any loop: two minutes without a block, and the
    // ones it was after are left for others.
    if rt.tick.saturating_sub(rt.mined_at) > 2400 {
        rt.mined_at = rt.tick;
        rt.fails = 0;
        clear_path(rt);
        for b in &list {
            rt.avoid.insert(*b);
        }
        *rt.why.entry("no progress").or_default() += list.len() as u32;
        s.say(format!("Baritone: no progress for 2 minutes, leaving {} of them", list.len()));
        rt.process = Some(Proc::Mine { names, want, done });
        return;
    }
    // Far above them: down to their level first. A search for the ores themselves
    // from the surface runs out of time before it finds a way down.
    if feet[1] - list[0][1] > 12 {
        let y = list[0][1] + 1;
        rt.status = format!("going down to y {y} ({} known)", rt.known.len());
        if walk(g, s, rt, &Goal::Y(y)) {
            rt.fails = 0;
            clear_path(rt);
        }
        rt.process = Some(Proc::Mine { names, want, done });
        return;
    }
    rt.status = format!("going to {} ({} known)", core::pretty(&core::block_name(g, list[0])), rt.known.len());
    let goal = Goal::Beside(list.clone());
    if walk(g, s, rt, &goal) {
        // Can't reach those: leave them out and try others.
        rt.fails = 0;
        clear_path(rt);
        for b in &list {
            rt.avoid.insert(*b);
        }
        *rt.why.entry("unreachable").or_default() += list.len() as u32;
        s.say(format!("Baritone: can't reach {} of them, trying others", list.len()));
    }
    rt.process = Some(Proc::Mine { names, want, done });
}

fn follow_target(g: &Game<'_>, who: &str) -> Option<([f64; 3], String)> {
    let names = core::player_names(g);
    let eye = g.eye_position()?;
    let mut best: Option<([f64; 3], String, f64)> = None;
    let self_id = g.remote.as_ref().map(|r| r.profile_name.clone());
    for m in g.mobs.iter().filter(|m| m.alive()) {
        let name = names.get(&m.id).cloned();
        let is_player = name.is_some();
        let kind = m.kind.id.trim_start_matches("minecraft:");
        let wanted = match who {
            "players" | "player" => is_player && name != self_id,
            "entities" | "mobs" => true,
            w => name.as_deref().is_some_and(|n| n.eq_ignore_ascii_case(w)) || kind.eq_ignore_ascii_case(w),
        };
        if !wanted {
            continue;
        }
        let at = [m.position.x, m.position.y, m.position.z];
        let d = core::dist(eye, at);
        if best.as_ref().is_none_or(|b| d < b.2) {
            best = Some((at, name.unwrap_or_else(|| core::pretty(&m.kind.id)), d));
        }
    }
    best.map(|b| (b.0, b.1))
}

/// The mature crop at a state's name and properties, and what grows it again.
fn mature(text: &str) -> Option<Option<&'static str>> {
    let name = text.split('[').next().unwrap_or(text).trim_start_matches("minecraft:");
    let age = |want: &str| text.contains(&format!("age={want}"));
    match name {
        "wheat" if age("7") => Some(Some("minecraft:wheat_seeds")),
        "carrots" if age("7") => Some(Some("minecraft:carrot")),
        "potatoes" if age("7") => Some(Some("minecraft:potato")),
        "beetroots" if age("3") => Some(Some("minecraft:beetroot_seeds")),
        "nether_wart" if age("3") => Some(Some("minecraft:nether_wart")),
        "melon" | "pumpkin" => Some(None),
        _ => None,
    }
}

fn farm_tick(g: &mut Game<'_>, s: &mut State, rt: &mut Rt, range: i32) {
    let Some(feet) = feet_cell(g, &mut rt.t) else {
        return;
    };
    // Plant again what was just harvested.
    if let Some((at, seed)) = rt.replant {
        let state = g.world.block(at[0], at[1], at[2]);
        if g.res.facts().is_air(state) && core::dist(center(feet), mid(at)) < g.reach() {
            press(g, rt, &[]);
            if rt.wait > 0 {
                rt.wait -= 1;
                return;
            }
            if let Some(slot) = core::hotbar_find(g, |i| i == seed)
                && core::place(g, at, slot)
            {
                rt.wait = 2;
            }
            rt.replant = None;
            return;
        }
        if !g.res.facts().is_air(state) {
            rt.replant = None;
        }
    }
    if rt.tick >= rt.crops_at {
        rt.crops_at = rt.tick + 40;
        rt.crops.clear();
        for dx in -range..=range {
            for dz in -range..=range {
                for dy in -6..=6 {
                    let p = [feet[0] + dx, feet[1] + dy, feet[2] + dz];
                    let state = g.world.block(p[0], p[1], p[2]);
                    if !g.res.facts().is_air(state)
                        && g.res.facts().get(state).hardness >= 0.0
                        && mature(&g.state_string(state)).is_some()
                    {
                        rt.crops.push(p);
                    }
                }
            }
        }
        let c = center(feet);
        rt.crops.sort_by(|a, b| core::dist(c, center(*a)).total_cmp(&core::dist(c, center(*b))));
    }
    let crops = std::mem::take(&mut rt.crops);
    let found = crops.iter().copied().find(|p| {
        let state = g.world.block(p[0], p[1], p[2]);
        mature(&g.state_string(state)).is_some()
    });
    rt.crops = crops;
    let Some(crop) = found else {
        let c = center(feet);
        let drop = g
            .dropped
            .iter()
            .map(|d| [d.position.x.floor() as i32, (d.position.y + 0.1).floor() as i32, d.position.z.floor() as i32])
            .filter(|d| (d[0] - feet[0]).abs() <= range && (d[2] - feet[2]).abs() <= range && (d[1] - feet[1]).abs() <= 6)
            .min_by(|a, b| core::dist(c, center(*a)).total_cmp(&core::dist(c, center(*b))));
        if let Some(item) = drop
            && item != feet
        {
            rt.status = "collecting the harvest".to_owned();
            let goal = Goal::Block(item);
            if rt.path.last().is_none_or(|l| l.pos != item) && rt.plan.is_none() {
                clear_path(rt);
            }
            if walk(g, s, rt, &goal) {
                rt.fails = 0;
                clear_path(rt);
            }
            return;
        }
        rt.status = "waiting for crops to grow".to_owned();
        press(g, rt, &[]);
        return;
    };
    rt.status = format!("farming ({} ripe)", rt.crops.len());
    if core::dist(center(feet), mid(crop)) < g.reach() - 0.5 {
        clear_path(rt);
        press(g, rt, &[]);
        let state = g.world.block(crop[0], crop[1], crop[2]);
        let text = g.state_string(state);
        if aim_block(g, crop) {
            core::click(g, false);
            if let Some(Some(seed)) = mature(&text) {
                rt.replant = Some((crop, seed));
                rt.wait = 3;
            }
            rt.crops.retain(|p| *p != crop);
        } else {
            rt.crops.retain(|p| *p != crop);
        }
        return;
    }
    let goal = Goal::Near(crop, 2);
    if rt.path.last().is_none_or(|l| !goal.reached(g, &mut rt.t, l.pos)) && rt.plan.is_none() {
        clear_path(rt);
    }
    if walk(g, s, rt, &goal) {
        rt.fails = 0;
        rt.crops.retain(|p| *p != crop);
    }
}

fn tunnel_tick(g: &mut Game<'_>, s: &mut State, rt: &mut Rt, dir: [i32; 2], mut left: i32, width: i32, height: i32) -> i32 {
    let Some(feet) = feet_cell(g, &mut rt.t) else {
        return left;
    };
    // Moved on a block: one done.
    if rt.tunnel_at.is_some_and(|at| at != feet) {
        left -= 1;
        if left <= 0 {
            press(g, rt, &[]);
            stop_hands(g, rt);
            return 0;
        }
    }
    rt.tunnel_at = Some(feet);
    let front = [feet[0] + dir[0], feet[1], feet[2] + dir[1]];
    let side = [dir[1], 0, dir[0]];
    rt.status = format!("tunnelling, {left} to go");
    // Lava or water ahead: stop rather than flood the tunnel.
    for w in -(width / 2)..=(width - 1) / 2 {
        for h in 0..=height {
            let p = add(front, [side[0] * w, h, side[2] * w]);
            if rt.t.at(g, add(p, [dir[0], 0, dir[1]])) & (LAVA | WATER) != 0 || rt.t.at(g, p) & (LAVA | WATER) != 0 {
                finish(g, s, rt, "Tunnel: stopped, there is lava or water ahead".to_owned());
                return 0;
            }
        }
    }
    for h in (0..height).rev() {
        for w in -(width / 2)..=(width - 1) / 2 {
            let p = add(front, [side[0] * w, h, side[2] * w]);
            if mine(g, s, rt, p) {
                press(g, rt, &[]);
                return left;
            }
        }
    }
    stop_hands(g, rt);
    let below = add(front, [0, -1, 0]);
    if rt.t.at(g, below) & FLOOR == 0 && s.b("Baritone", "AllowPlace") {
        press(g, rt, &[KeyCode::ShiftLeft, KeyCode::KeyW]);
        place(g, s, rt, below);
        return left;
    }
    let Some(pos) = g.player.as_ref().map(|p| [p.position.x, p.position.y, p.position.z]) else {
        return left;
    };
    yaw_to(g, center(front), pos);
    g.pitch = 0.0;
    press(g, rt, &[KeyCode::KeyW]);
    left
}

/// Once a tick, before the player moves.
pub fn tick(g: &mut Game<'_>, s: &mut State) {
    RT.with(|rt| {
        let rt = &mut *rt.borrow_mut();
        rt.tick += 1;
        if rt.process.is_none() {
            return;
        }
        if s.b("Baritone", "DebugLog") && rt.tick.is_multiple_of(20) {
            debug_log(g, rt);
        }
        // Stops for a player nearby (not a friend) and goes on once they're gone.
        let near = s.b("Baritone", "PauseNearPlayer") && !core::targets(g, s, 16.0, true, false, false).is_empty();
        if near && !rt.paused {
            rt.paused = true;
            rt.auto_paused = true;
            s.say("Baritone paused: a player is near");
        } else if !near && rt.auto_paused {
            rt.paused = false;
            rt.auto_paused = false;
            rt.moved = rt.tick;
            s.say("Baritone resumed");
        }
        if !core::playing(g) || rt.paused || g.player.is_none() || s.gui.open {
            press(g, rt, &[]);
            stop_hands(g, rt);
            return;
        }
        if survive(g, s, rt) {
            // Time spent staying alive doesn't count against the mining's progress.
            rt.mined_at += 1;
            return;
        }
        after_survival(rt);
        if rt.worn_out {
            finish(g, s, rt, "Baritone stopped: the pickaxe is about to break".to_owned());
            return;
        }
        let Some(process) = rt.process.take() else {
            return;
        };
        match process {
            Proc::Goto(goal) => {
                let Some(feet) = feet_cell(g, &mut rt.t) else {
                    return;
                };
                if goal.reached(g, &mut rt.t, feet) {
                    finish(g, s, rt, format!("Reached {}", goal.label()));
                    return;
                }
                rt.status = format!(
                    "going to {} ({} steps left{})",
                    goal.label(),
                    rt.path.len().saturating_sub(rt.at + 1),
                    if rt.partial { ", more to plan" } else { "" }
                );
                if walk(g, s, rt, &goal) {
                    finish(g, s, rt, format!("No path to {}", goal.label()));
                    return;
                }
                rt.process = Some(Proc::Goto(goal));
            }
            Proc::Mine { names, want, done } => mine_tick(g, s, rt, names, want, done),
            Proc::Follow { who, mut last } => {
                let Some((at, name)) = follow_target(g, &who) else {
                    rt.status = format!("no {who} in sight");
                    press(g, rt, &[]);
                    rt.process = Some(Proc::Follow { who, last });
                    return;
                };
                let Some(feet) = feet_cell(g, &mut rt.t) else {
                    return;
                };
                let cell = [at[0].floor() as i32, at[1].floor() as i32, at[2].floor() as i32];
                let range = s.n("Baritone", "FollowRange") as i32;
                let goal = Goal::Near(cell, range);
                rt.status = format!("following {name}");
                if goal.reached(g, &mut rt.t, feet) {
                    clear_path(rt);
                    press(g, rt, &[]);
                    stop_hands(g, rt);
                    rt.fails = 0;
                } else {
                    let moved = last.is_none_or(|l| (l[0] - cell[0]).abs().max((l[2] - cell[2]).abs()) > 2);
                    if moved && rt.plan.is_none() {
                        rt.path.clear();
                        rt.at = 0;
                        last = Some(cell);
                    }
                    if walk(g, s, rt, &goal) {
                        rt.fails = 0;
                        last = None;
                    }
                }
                rt.process = Some(Proc::Follow { who, last });
            }
            Proc::Explore { center, mut index, mut goal } => {
                let Some(feet) = feet_cell(g, &mut rt.t) else {
                    return;
                };
                let target = *goal.get_or_insert_with(|| spiral(center, index));
                let close = (feet[0] - target.0).abs().max((feet[2] - target.1).abs()) <= 8;
                rt.status = format!("exploring, heading to {} {}", target.0, target.1);
                if close {
                    index += 1;
                    goal = None;
                    clear_path(rt);
                } else if walk(g, s, rt, &Goal::Xz(target.0, target.1)) {
                    rt.fails = 0;
                    index += 1;
                    goal = None;
                    clear_path(rt);
                }
                rt.process = Some(Proc::Explore { center, index, goal });
            }
            Proc::Farm { range } => {
                rt.process = Some(Proc::Farm { range });
                farm_tick(g, s, rt, range);
            }
            Proc::Tunnel { dir, left, width, height } => {
                rt.process = Some(Proc::Tunnel { dir, left, width, height });
                let now = tunnel_tick(g, s, rt, dir, left, width, height);
                if rt.process.is_none() {
                    return;
                }
                if now <= 0 {
                    finish(g, s, rt, "Tunnel done".to_owned());
                } else {
                    rt.process = Some(Proc::Tunnel { dir, left: now, width, height });
                }
            }
        }
    });
}

/// One line of what Baritone is doing, appended to baritone-debug.log in the
/// mod's data folder once a second while the DebugLog setting is on.
fn debug_line(g: &Game<'_>, rt: &mut Rt) -> String {
    let (hp, food, air, fire) = health(g);
    let feet = feet_cell(g, &mut rt.t);
    let next = rt.path.get(rt.at + 1).map(|s| (s.mv, s.pos));
    let held = g.held().map(|s| s.item.to_string());
    format!(
        "tick={} hp={hp:.1} food={food} air={air} fire={fire} feet={feet:?} ground={} held={held:?} status=\"{}\" path={}/{} next={next:?} partial={} fails={} planning={} digging={:?} loot={:?} known={} avoid={} hazard={} flee={:?} seal={:?} bunker={:?} escape={} eat={:?} fight={:?} mobs=[{}] why={:?}",
        rt.tick,
        on_ground(g),
        rt.status,
        rt.at,
        rt.path.len(),
        rt.partial,
        rt.fails,
        rt.plan.is_some(),
        rt.digging,
        rt.loot,
        rt.known.len(),
        rt.avoid.len(),
        rt.hazard.len(),
        rt.flee.map(|f| f.0.map(|v| v.floor() as i32)),
        rt.seal,
        rt.bunker,
        rt.escape.is_some(),
        rt.eat,
        rt.fight,
        rt.mobs_note,
        rt.why,
    )
}

fn debug_log(g: &Game<'_>, rt: &mut Rt) {
    use std::io::Write;
    let line = debug_line(g, rt);
    let dir = g.native_data_dir(core::ID);
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("baritone-debug.log")) {
        let _ = writeln!(f, "{line}");
    }
}

/// Spiral stops around a centre, 96 blocks apart.
fn spiral(center: (i32, i32), index: i32) -> (i32, i32) {
    let (mut x, mut z, mut dx, mut dz) = (0_i32, 0_i32, 0_i32, -1_i32);
    for _ in 0..index {
        if x == z || (x < 0 && x == -z) || (x > 0 && x == 1 - z) {
            (dx, dz) = (-dz, dx);
        }
        x += dx;
        z += dz;
    }
    (center.0 + x * 96, center.1 + z * 96)
}

// ------------------------------------------------------------- staying alive
//
// Runs before the process each tick while Survival is on, and takes the tick
// when something matters more than the path: a head in a block, lava, air
// running out, a mob, hunger, a full inventory. Its last resort is to wall
// itself in (no mob reaches, sees or blows up a player in a sealed 1x2 room)
// and heal there.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Danger {
    Melee,
    Ranged,
    Creeper,
    Warden,
    /// Neutral until provoked (endermen, zombified piglins): only close ones count.
    Neutral,
}

fn danger_of(id: &str) -> Danger {
    match id.trim_start_matches("minecraft:") {
        "warden" => Danger::Warden,
        "creeper" => Danger::Creeper,
        "skeleton" | "stray" | "bogged" | "witch" | "blaze" | "pillager" | "ghast" | "evoker" | "breeze" => Danger::Ranged,
        "enderman" | "zombified_piglin" | "piglin" => Danger::Neutral,
        _ => Danger::Melee,
    }
}

struct Seen {
    t: core::Target,
    kind: Danger,
    /// Nothing solid between the eyes and it.
    sees: bool,
    swell: i32,
}

fn sees(g: &Game<'_>, eye: [f64; 3], at: [f64; 3]) -> bool {
    let d = core::dist(eye, at);
    let dir = [0, 1, 2].map(|a| (at[a] - eye[a]) / d.max(1e-6));
    g.clip_ray(eye, dir, d).is_none()
}

/// A cell the body can step into from beside it: room for feet and head, firm
/// ground under it, and nothing that burns, drowns or traps.
fn safe_cell(g: &Game<'_>, t: &mut Tables, c: P) -> bool {
    let (feet, head, below) = (t.at(g, c), t.at(g, add(c, [0, 1, 0])), t.at(g, add(c, [0, -1, 0])));
    feet & PASS != 0
        && head & PASS != 0
        && (feet | head) & (LAVA | DANGER) == 0
        && below & FLOOR != 0
        && below & DANGER == 0
        && head & WATER == 0
        && loaded(g, c)
}

fn lava_beside(g: &Game<'_>, t: &mut Tables, p: P) -> bool {
    [[0, 1, 0], [0, -1, 0], [1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1]].iter().any(|d| t.at(g, add(p, *d)) & LAVA != 0)
}

/// The extra cost of standing at `feet` in the open: at head height, a dug 1x2
/// tunnel has at most two open sides (back and front); three or more is a cave.
fn exposure(g: &Game<'_>, t: &mut Tables, feet: P) -> f64 {
    let head = add(feet, [0, 1, 0]);
    let open = [[1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1], [0, 1, 0]]
        .iter()
        .filter(|d| t.at(g, add(head, **d)) & PASS != 0)
        .count();
    if open >= 3 { 1.0 } else { 0.0 }
}

/// Lava anywhere around a body standing with its feet at `feet`: beside the feet
/// or the head, under the feet, over the head, or in the two cells themselves.
fn lava_near(g: &Game<'_>, t: &mut Tables, feet: P) -> bool {
    const AROUND: [P; 12] = [
        [0, 0, 0], [0, 1, 0], [0, -1, 0], [0, 2, 0],
        [1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1],
        [1, 1, 0], [-1, 1, 0], [0, 1, 1], [0, 1, -1],
    ];
    AROUND.iter().any(|d| t.at(g, add(feet, *d)) & LAVA != 0)
}

fn weapon_slot(g: &Game<'_>) -> Option<usize> {
    let rank = |item: &str| {
        let tier = if item.contains("netherite") {
            5
        } else if item.contains("diamond") {
            4
        } else if item.contains("iron") {
            3
        } else if item.contains("stone") {
            2
        } else {
            1
        };
        if item.ends_with("_sword") {
            Some(tier * 10 + 2)
        } else if item.ends_with("_axe") {
            Some(tier * 10 + 1)
        } else if item.ends_with("_pickaxe") {
            Some(tier * 10)
        } else {
            None
        }
    };
    (0..9)
        .filter_map(|i| g.inventory[i].as_ref().and_then(|s| rank(&s.item)).map(|r| (i, r)))
        .max_by_key(|x| x.1)
        .map(|x| x.0)
}

fn is_food(g: &Game<'_>, item: &str) -> bool {
    const BAD: [&str; 5] = ["minecraft:rotten_flesh", "minecraft:spider_eye", "minecraft:poisonous_potato", "minecraft:pufferfish", "minecraft:chorus_fruit"];
    !BAD.contains(&item) && g.res.items().get(item).is_some_and(|i| i.food.is_some())
}

/// The hotbar slot with food in it, moving some there from the inventory when none is.
fn food_in_hand_reach(g: &mut Game<'_>) -> Option<usize> {
    let has = |g: &Game<'_>, i: usize| g.inventory[i].as_ref().is_some_and(|s| is_food(g, &s.item));
    if let Some(i) = (0..9).find(|&i| has(g, i)) {
        return Some(i);
    }
    let from = (9..36).find(|&i| has(g, i))?;
    if !core::inventory_menu(g) {
        return None;
    }
    // An empty hotbar slot, else one holding neither a tool nor the throwaway (8).
    let tool = |g: &Game<'_>, i: usize| {
        g.inventory[i].as_ref().is_some_and(|s| s.item.ends_with("_pickaxe") || s.item.ends_with("_sword") || s.item.ends_with("_axe"))
    };
    let to = (0..8).find(|&i| g.inventory[i].is_none()).or_else(|| (0..8).rev().find(|&i| !tool(g, i)))?;
    core::swap_slot(g, from, to);
    (0..9).find(|&i| has(g, i))
}

fn health(g: &Game<'_>) -> (f32, i32, i32, i32) {
    g.player.as_ref().map_or((20.0, 20, 300, 0), |p| (p.health.health(), p.health.food.food, p.air, p.fire_ticks))
}

/// Keeps the walk off what the survival layer just did: a fresh plan from here.
fn after_survival(rt: &mut Rt) {
    if rt.disturbed {
        rt.disturbed = false;
        clear_path(rt);
    }
}

fn stop_eating(g: &mut Game<'_>, rt: &mut Rt) {
    if rt.eat.take().is_some() {
        core::hold(g, true, false);
    }
}

/// True while eating; starts when `start` and there is food.
fn eating(g: &mut Game<'_>, rt: &mut Rt, start: bool) -> bool {
    if let Some(ticks) = rt.eat {
        let held_food = g.held().is_some_and(|st| is_food(g, &st.item));
        let full = health(g).1 >= 20;
        if !held_food || full || (ticks > 6 && g.using.is_none()) || ticks > 60 {
            stop_eating(g, rt);
            return false;
        }
        rt.eat = Some(ticks + 1);
        core::hold(g, true, true);
        return true;
    }
    if !start {
        return false;
    }
    let Some(slot) = food_in_hand_reach(g) else {
        return false;
    };
    stop_hands(g, rt);
    g.selected = slot;
    rt.eat = Some(0);
    core::hold(g, true, true);
    true
}

/// The cells that close the player in: the four sides at feet and head height and
/// the one over the head, the side facing `toward` first.
fn seal_cells(feet: P, toward: Option<[f64; 3]>) -> Vec<P> {
    let mut sides: Vec<P> = vec![[1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1]];
    if let Some(at) = toward {
        let c = center(feet);
        sides.sort_by(|a, b| {
            let da = (c[0] + f64::from(a[0]) - at[0]).powi(2) + (c[2] + f64::from(a[2]) - at[2]).powi(2);
            let db = (c[0] + f64::from(b[0]) - at[0]).powi(2) + (c[2] + f64::from(b[2]) - at[2]).powi(2);
            da.total_cmp(&db)
        });
    }
    let mut out = Vec::new();
    for d in &sides {
        out.push(add(add(feet, *d), [0, 1, 0]));
        out.push(add(feet, *d));
    }
    out.push(add(feet, [0, 2, 0]));
    out
}

fn sealed(g: &Game<'_>, t: &mut Tables, feet: P) -> bool {
    seal_cells(feet, None).iter().all(|c| t.at(g, *c) & PASS == 0)
}

/// Places the next missing wall block. False when it has nothing to place with.
fn seal_step(g: &mut Game<'_>, s: &State, rt: &mut Rt, feet: P, toward: Option<[f64; 3]>) -> bool {
    stock_hotbar(g, s);
    let Some(slot) = throwaway_slot(g, s) else {
        return false;
    };
    if rt.wait > 0 {
        rt.wait -= 1;
        return true;
    }
    let pos = g.player.as_ref().map_or(center(feet), |p| [p.position.x, p.position.y, p.position.z]);
    let mut open: Vec<P> = seal_cells(feet, toward).into_iter().filter(|c| rt.t.at(g, *c) & PASS != 0 && core::replaceable(g, *c)).collect();
    // A cell with nothing solid beside it (the one over the head, in a cave) can't
    // take a block: one beside it first, over a wall block, gives it something.
    let solid_near = |g: &Game<'_>, t: &mut Tables, c: P| {
        [[0, -1, 0], [0, 1, 0], [1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1]].iter().any(|d| t.at(g, add(c, *d)) & PASS == 0)
    };
    let mut support = Vec::new();
    for c in &open {
        if !solid_near(g, &mut rt.t, *c) {
            for d in [[1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1]] {
                let s2 = add(*c, d);
                if rt.t.at(g, s2) & PASS != 0 && core::replaceable(g, s2) && solid_near(g, &mut rt.t, s2) && !open.contains(&s2) {
                    support.push(s2);
                    break;
                }
            }
        }
    }
    open.extend(support);
    // Off the middle, the body pokes into a wall cell and that block can't go
    // in: to the middle first, sneaking so as not to step out.
    if open.iter().any(|c| body_in(pos, *c)) {
        yaw_to(g, center(feet), pos);
        press(g, rt, &[KeyCode::KeyW, KeyCode::ShiftLeft]);
        return true;
    }
    press(g, rt, &[]);
    for c in open {
        if core::place(g, c, slot) {
            rt.wait = 1;
            return true;
        }
    }
    true
}

/// Whether a bunker can be dug under `top` (the feet cell now): the three cells
/// below breakable and safe, every side of them solid natural rock (nothing walks
/// or flows in), a firm floor under the bottom, and no lava near the bottom.
fn bunker_ok(g: &Game<'_>, t: &mut Tables, top: P) -> bool {
    const SIDES: [P; 4] = [[1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1]];
    for d in 1..=3 {
        let c = add(top, [0, -d, 0]);
        let k = t.at(g, c);
        if k & BREAK == 0 || k & (INFESTED | FALLS | WATER | LAVA | DANGER) != 0 || !loaded(g, c) {
            return false;
        }
        for sd in SIDES {
            let w = t.at(g, add(c, sd));
            if w & PASS != 0 || w & (WATER | LAVA | INFESTED) != 0 {
                return false;
            }
        }
    }
    let floor = t.at(g, add(top, [0, -4, 0]));
    floor & FLOOR != 0 && floor & (DANGER | FALLS) == 0 && !lava_near(g, t, add(top, [0, -3, 0]))
}

/// The nearest spot within 4 blocks to dig a bunker at, that can be stood on.
fn bunker_near(g: &Game<'_>, t: &mut Tables, feet: P) -> Option<P> {
    let mut best: Option<(P, i32)> = None;
    for dx in -4..=4_i32 {
        for dz in -4..=4_i32 {
            for dy in -1..=1_i32 {
                let c = add(feet, [dx, dy, dz]);
                let d = dx.abs() + dz.abs() + dy.abs();
                if d == 0 || d > 5 || best.is_some_and(|b| b.1 <= d) {
                    continue;
                }
                if safe_cell(g, t, c) && !lava_near(g, t, c) && bunker_ok(g, t, c) {
                    best = Some((c, d));
                }
            }
        }
    }
    best.map(|b| b.0)
}

/// The bunker is shut: the feet at the bottom and the top cell of the pit solid.
fn bunker_closed(g: &Game<'_>, t: &mut Tables, top: P, feet: P) -> bool {
    feet == add(top, [0, -3, 0]) && t.at(g, add(top, [0, -1, 0])) & PASS == 0
}

/// One tick of digging the bunker under `top` or capping it. False when it
/// can't go on.
fn bunker_step(g: &mut Game<'_>, s: &State, rt: &mut Rt, top: P) -> bool {
    let Some(pos) = g.player.as_ref().map(|p| [p.position.x, p.position.y, p.position.z]) else {
        return false;
    };
    let cell = [pos[0].floor() as i32, (pos[1] + 0.01).floor() as i32, pos[2].floor() as i32];
    if cell[0] != top[0] || cell[2] != top[2] || cell[1] > top[1] || cell[1] < top[1] - 3 {
        return false;
    }
    let bottom = top[1] - 3;
    if cell[1] > bottom {
        if !on_ground(g) {
            press(g, rt, &[]);
            return true;
        }
        // In the middle of the cell, or the body stays on the rim when the floor goes.
        let mid_cell = center(cell);
        if (pos[0] - mid_cell[0]).abs() > 0.2 || (pos[2] - mid_cell[2]).abs() > 0.2 {
            stop_hands(g, rt);
            yaw_to(g, mid_cell, pos);
            press(g, rt, &[KeyCode::KeyW, KeyCode::ShiftLeft]);
            return true;
        }
        press(g, rt, &[]);
        let below = add(cell, [0, -1, 0]);
        if rt.t.at(g, below) & PASS != 0 {
            return true;
        }
        g.selected = best_tool(g, below).0;
        if aim_block(g, below) {
            core::hold(g, false, true);
            rt.mining = Some(below);
        }
        return true;
    }
    // At the bottom: the lid.
    stop_hands(g, rt);
    press(g, rt, &[]);
    stock_hotbar(g, s);
    let cap = add(top, [0, -1, 0]);
    if rt.t.at(g, cap) & PASS != 0 {
        if rt.wait > 0 {
            rt.wait -= 1;
        } else if !place(g, s, rt, cap) && throwaway_slot(g, s).is_none() {
            return false;
        }
    }
    true
}

/// Junk out when the inventory is full, so drops (diamonds above all) still fit.
fn drop_junk(g: &mut Game<'_>, s: &State, rt: &mut Rt) -> bool {
    if rt.tick % 20 != 0 || !core::inventory_menu(g) {
        return false;
    }
    if (0..36).any(|i| g.inventory[i].is_none()) {
        return false;
    }
    let list = s.list("Baritone", "Throwaway");
    let keep_names = ["minecraft:diamond", "minecraft:torch", "minecraft:water_bucket", "minecraft:totem_of_undying"];
    // The two biggest throwaway stacks stay, for walls and bridges.
    let mut throwaway: Vec<(usize, u32)> = (0..36)
        .filter_map(|i| g.inventory[i].as_ref().filter(|st| list.iter().any(|l| *l == *st.item)).map(|st| (i, st.count)))
        .collect();
    throwaway.sort_by(|a, b| b.1.cmp(&a.1));
    let kept: Vec<usize> = throwaway.iter().take(2).map(|x| x.0).collect();
    let junk = (0..36).rev().find(|&i| {
        let Some(st) = g.inventory[i].as_ref() else {
            return false;
        };
        let item: &str = &st.item;
        !(keep_names.contains(&item)
            || item.ends_with("_pickaxe")
            || item.ends_with("_sword")
            || item.ends_with("_axe")
            || item.ends_with("_shovel")
            || is_food(g, item)
            || kept.contains(&i))
    });
    let Some(slot) = junk else {
        return false;
    };
    // Thrown backwards along the path, so the walk doesn't pick it up again.
    let (yaw, pitch) = (g.yaw, g.pitch);
    g.yaw += 180.0;
    g.pitch = -10.0;
    core::throw_stack(g, slot);
    g.yaw = yaw;
    g.pitch = pitch;
    let _ = rt;
    true
}

fn retreat(g: &mut Game<'_>, rt: &mut Rt, feet: P, from: [f64; 3]) -> bool {
    let Some(pos) = g.player.as_ref().map(|p| [p.position.x, p.position.y, p.position.z]) else {
        return false;
    };
    let mut best: Option<(P, f64)> = None;
    for d in [[1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1]] {
        let c = add(feet, d);
        if !safe_cell(g, &mut rt.t, c) {
            continue;
        }
        let away = core::dist(center(c), from);
        if away > core::dist(center(feet), from) + 0.2 && best.is_none_or(|b| away > b.1) {
            best = Some((c, away));
        }
    }
    let Some((c, _)) = best else {
        return false;
    };
    yaw_to(g, center(c), pos);
    press(g, rt, &[KeyCode::KeyW]);
    rt.disturbed = true;
    true
}

/// One tick of the survival layer. True when it used the tick.
fn survive(g: &mut Game<'_>, s: &State, rt: &mut Rt) -> bool {
    if !s.b("Baritone", "Survival") || g.player.is_none() {
        return false;
    }
    let (hp, food, air, fire) = health(g);
    let Some(feet) = feet_cell(g, &mut rt.t) else {
        return false;
    };
    let Some(eye) = g.eye_position() else {
        return false;
    };
    if hp < rt.last_hp {
        rt.hurt_at = rt.tick;
    }
    rt.last_hp = hp;

    // What is around: every hostile in 32 blocks.
    rt.ignore.retain(|_, until| *until > rt.tick);
    let ignore = rt.ignore.clone();
    let seen: Vec<Seen> = core::targets(g, s, 32.0, false, true, false)
        .into_iter()
        .filter(|t| !ignore.contains_key(&t.id) || t.distance < 2.5)
        .map(|t| {
            let kind = danger_of(&g.mobs[t.index].kind.id);
            let swell = g.mobs[t.index].swell;
            let vis = sees(g, eye, t.center);
            Seen { t, kind, sees: vis, swell }
        })
        .collect();
    rt.mobs_note = seen
        .iter()
        .take(4)
        .map(|x| format!("{}@{:.1}{}", x.t.name, x.t.distance, if x.sees { "" } else { "(hidden)" }))
        .collect::<Vec<_>>()
        .join(",");
    let near = |kinds: &[Danger], range: f64, need_sight: bool| {
        seen.iter().filter(|x| kinds.contains(&x.kind) && x.t.distance <= range && (x.sees || !need_sight || x.t.distance < 2.5)).count()
    };

    // A warden: get far away and stay away from where it was.
    if let Some(w) = seen.iter().find(|x| x.kind == Danger::Warden && x.t.distance < 40.0) {
        rt.flee = Some((w.t.center, rt.tick + 2400));
        if w.t.distance < 20.0 && retreat(g, rt, feet, w.t.center) {
            stop_eating(g, rt);
            stop_hands(g, rt);
            rt.status = "getting away from a warden".to_owned();
            return true;
        }
    }
    if rt.flee.is_some_and(|f| rt.tick > f.1) {
        rt.flee = None;
    }

    let head = add(feet, [0, 1, 0]);
    let head_c = rt.t.at(g, head);
    // Head in a block (sand or gravel fell in): dig it out now. The cells are
    // the ones the eye and the feet are really in: `feet` moves up a cell when
    // its own cell is solid, which is just what fallen gravel does.
    let pos = g.player.as_ref().map_or(eye, |p| [p.position.x, p.position.y, p.position.z]);
    let eye_cell = eye.map(|v| v.floor() as i32);
    let low_cell = [pos[0].floor() as i32, (pos[1] + 0.2).floor() as i32, pos[2].floor() as i32];
    let stuck_in = |g: &Game<'_>, t: &mut Tables, c: P, falls_only: bool| {
        let k = t.at(g, c);
        k & PASS == 0 && k & BREAK != 0 && k & (WATER | LAVA) == 0 && (!falls_only || k & FALLS != 0)
    };
    let buried = if stuck_in(g, &mut rt.t, eye_cell, false) {
        Some(eye_cell)
    } else if stuck_in(g, &mut rt.t, low_cell, true) {
        Some(low_cell)
    } else {
        None
    };
    if let Some(cell) = buried {
        stop_eating(g, rt);
        press(g, rt, &[]);
        g.selected = best_tool(g, cell).0;
        if aim_block(g, cell) {
            core::hold(g, false, true);
            rt.mining = Some(cell);
        }
        rt.status = "digging out".to_owned();
        rt.disturbed = true;
        return true;
    }

    // In lava or burning beside it: out the way the path came, or any safe side.
    let in_lava = (rt.t.at(g, feet) | head_c) & LAVA != 0;
    if in_lava || (fire > 0 && lava_beside(g, &mut rt.t, feet)) {
        stop_eating(g, rt);
        stop_hands(g, rt);
        let back = if rt.at > 0 { rt.path.get(rt.at - 1).map(|s| s.pos) } else { None };
        let pos = g.player.as_ref().map(|p| [p.position.x, p.position.y, p.position.z]).unwrap_or(eye);
        let to = back.filter(|c| safe_cell(g, &mut rt.t, *c)).or_else(|| {
            [[1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1]].iter().map(|d| add(feet, *d)).find(|c| safe_cell(g, &mut rt.t, *c))
        });
        if let Some(c) = to {
            yaw_to(g, center(c), pos);
        }
        press(g, rt, &[KeyCode::KeyW, KeyCode::Space]);
        rt.status = "out of the lava".to_owned();
        rt.disturbed = true;
        return true;
    }

    // Air running out: up.
    if head_c & WATER != 0 && air < 150 {
        stop_eating(g, rt);
        stop_hands(g, rt);
        press(g, rt, &[KeyCode::Space]);
        rt.status = "coming up for air".to_owned();
        rt.disturbed = true;
        return true;
    }

    // A creeper swells within 3 blocks and blows up 1.5 s later, which kills a
    // player with no armour: act well before that, also round a corner.
    // While tunnelling away from one camped behind the wall, only a creeper that
    // sees us or swells counts: the one left behind is close by but blind.
    let escaping = rt.escape.is_some();
    let creeper_close = seen.iter().any(|x| {
        x.kind == Danger::Creeper
            && ((x.t.distance < 7.0 && x.sees) || (!escaping && x.t.distance < 4.0) || (x.swell > 0 && x.t.distance < 8.0))
    });
    let melee = near(&[Danger::Melee], 6.0, true);
    let ranged = near(&[Danger::Ranged], 16.0, true);
    let neutral = near(&[Danger::Neutral], 3.0, false);
    let threat = creeper_close || melee > 0 || ranged > 0 || neutral > 0;
    let nearest = seen
        .iter()
        .filter(|x| x.kind != Danger::Warden && (x.sees || x.t.distance < 2.5) && (x.kind != Danger::Neutral || x.t.distance < 3.0))
        .min_by(|a, b| a.t.distance.total_cmp(&b.t.distance));

    // On the way to a spot to dig a bunker at: a few seconds at most.
    if let Some((spot, since)) = rt.to_bunker {
        if feet == spot && on_ground(g) && bunker_ok(g, &mut rt.t, spot) {
            rt.to_bunker = None;
            clear_path(rt);
            rt.seal = Some(rt.tick);
            rt.bunker = Some(spot);
            rt.status = "digging a bunker".to_owned();
            bunker_step(g, s, rt, spot);
            return true;
        }
        let creeper_by = seen.iter().any(|x| x.kind == Danger::Creeper && x.t.distance < 8.0);
        if creeper_by || rt.tick.saturating_sub(since) > 120 || walk(g, s, rt, &Goal::Block(spot)) {
            // Not there in time: walls where it stands (next tick decides).
            rt.to_bunker = None;
            clear_path(rt);
        } else {
            rt.status = "going to dig in".to_owned();
            rt.disturbed = true;
            return true;
        }
    }

    // Walled in: stay until healed and the worst has gone.
    if let Some(since) = rt.seal {
        let done = hp >= 18.0
            && !seen.iter().any(|x| x.kind == Danger::Creeper && x.t.distance < 9.0)
            && near(&[Danger::Ranged], 14.0, false) == 0
            && near(&[Danger::Melee, Danger::Neutral], 5.0, false) <= 1
            && rt.tick.saturating_sub(since) > 40;
        // After 3 minutes at full health: mobs that camp outside the wall (a
        // creeper, zombies) never leave by themselves, so tunnel away from them
        // through the far side, filling the tunnel behind (see the escape below).
        // With only a shooter about, come out and fight it for a while instead.
        let long = rt.tick.saturating_sub(since) > 20 * 180 && hp >= 18.0 && !done;
        let campers: Vec<[f64; 3]> = seen
            .iter()
            .filter(|x| (x.kind == Danger::Creeper && x.t.distance < 7.0) || (matches!(x.kind, Danger::Melee | Danger::Neutral) && x.t.distance < 5.0))
            .map(|x| x.t.center)
            .collect();
        if long && !campers.is_empty() {
            let me = center(feet);
            let n = campers.len() as f64;
            let (cx, cz) = (campers.iter().map(|c| c[0]).sum::<f64>() / n, campers.iter().map(|c| c[2]).sum::<f64>() / n);
            let (dx, dz) = (me[0] - cx, me[2] - cz);
            let len = (dx * dx + dz * dz).sqrt().max(0.1);
            let to = Goal::Xz((me[0] + dx / len * 24.0).floor() as i32, (me[2] + dz / len * 24.0).floor() as i32);
            rt.seal = None;
            rt.bunker = None;
            rt.escape = Some((to, rt.tick));
            rt.behind.clear();
            rt.left_from = Some(feet);
            rt.fails = 0;
            clear_path(rt);
            stop_eating(g, rt);
            rt.status = "tunnelling away from mobs camped outside".to_owned();
            return true;
        }
        let too_long = long;
        if done || too_long {
            if too_long && !done {
                rt.brave_until = rt.tick + 600;
            }
            rt.seal = None;
            rt.bunker = None;
            rt.disturbed = true;
            stop_eating(g, rt);
        } else {
            press(g, rt, &[]);
            stop_hands(g, rt);
            // A mob in a gap of the wall gets hit first.
            // Only one it can see: one behind the wall is out of reach, and the
            // swing would only hit the wall.
            let close = seen
                .iter()
                .filter(|x| x.t.distance < 2.9 && x.kind != Danger::Creeper && x.kind != Danger::Warden)
                .min_by(|a, b| a.t.distance.total_cmp(&b.t.distance));
            if let Some(m) = close
                && rt.eat.is_none()
            {
                // Face it; only if the crosshair lands on it (not on the wall) is it
                // a fight. Line of sight to its middle misses mobs at corners.
                let (yaw, pitch) = (g.yaw, g.pitch);
                core::look_at(g, m.t.center);
                if g.mob_hit().is_some_and(|(i, _)| i == m.t.index) {
                    if let Some(w) = weapon_slot(g) {
                        g.selected = w;
                    }
                    if core::charged(g) {
                        core::click(g, false);
                    }
                    rt.status = "walled in, fighting".to_owned();
                    return true;
                }
                g.yaw = yaw;
                g.pitch = pitch;
            }
            // Hurt and nothing in reach: food first, so it heals (no healing below
            // 18 food), then the rest of the wall.
            let hungry_hurt = hp < 18.0 && food < 18;
            if (hungry_hurt || rt.eat.is_some()) && eating(g, rt, hp < 20.0 && food < 20) {
                rt.status = "walled in, eating".to_owned();
            } else if let Some(top) = rt.bunker
                && !bunker_closed(g, &mut rt.t, top, feet)
            {
                let creeper_by = seen.iter().any(|x| x.kind == Danger::Creeper && x.t.distance < 6.0);
                if creeper_by || !bunker_step(g, s, rt, top) {
                    // It can't be finished (something gave way): walls instead.
                    rt.bunker = None;
                }
                rt.status = format!("in a bunker, closing it ({hp:.0}/20)");
            } else if rt.bunker.is_none() && !sealed(g, &mut rt.t, feet) {
                seal_step(g, s, rt, feet, nearest.map(|m| m.t.center));
                rt.status = format!("walled in, closing the wall ({hp:.0}/20)");
            } else if !eating(g, rt, hp < 20.0 && food < 20) {
                rt.status = format!("walled in, healing ({hp:.0}/20)");
            } else {
                rt.status = "walled in, eating".to_owned();
            }
            return true;
        }
    }

    // When to wall in: a creeper about to blow, low health with something near,
    // more than two mobs at once, or a shooter while hurt.
    let have_blocks = throwaway_slot(g, s).is_some() || {
        let list = s.list("Baritone", "Throwaway");
        core::inventory_find(g, |i| list.iter().any(|l| l == i)).is_some()
    };
    // A shooter that sees us is walled off at once: walking up to it costs an
    // arrow a second or two, more than a bare player can spare.
    let wall = creeper_close
        || (hp <= 12.0 && (melee > 0 || ranged > 0 || neutral > 0 || near(&[Danger::Melee], 10.0, false) > 0))
        || melee >= 2
        || (ranged > 0 && rt.tick > rt.brave_until);
    // Which shelter. A creeper near: walls, now, the side facing it first; a
    // block between breaks its sight and stops its fuse, and a pit takes too long
    // (one walked to the rim of an open bunker and dropped in). Few gaps (a 1x2
    // tunnel): walls too. Many gaps (a cave): a bunker, here or a few steps away.
    let creeper_near = seen.iter().any(|x| x.kind == Danger::Creeper && x.t.distance < 8.0);
    let gaps = seal_cells(feet, None).iter().filter(|c| rt.t.at(g, **c) & PASS != 0).count();
    let walls_first = have_blocks && (creeper_near || gaps <= 4);
    let bunker_here = wall && !walls_first && on_ground(g) && bunker_ok(g, &mut rt.t, feet);
    if wall && !walls_first && !bunker_here && rt.to_bunker.is_none() && on_ground(g)
        && let Some(spot) = bunker_near(g, &mut rt.t, feet)
    {
        stop_eating(g, rt);
        stop_hands(g, rt);
        clear_path(rt);
        rt.fails = 0;
        rt.to_bunker = Some((spot, rt.tick));
        rt.status = "going to dig in".to_owned();
        return true;
    }
    if wall && (have_blocks || bunker_here) {
        stop_eating(g, rt);
        stop_hands(g, rt);
        press(g, rt, &[]);
        clear_path(rt);
        rt.seal = Some(rt.tick);
        if bunker_here {
            // Down into the rock and a lid on top: sturdier than walls in the open.
            rt.bunker = Some(feet);
            rt.status = "digging a bunker".to_owned();
            bunker_step(g, s, rt, feet);
        } else {
            rt.bunker = None;
            rt.status = "walling in".to_owned();
            seal_step(g, s, rt, feet, nearest.map(|m| m.t.center));
        }
        return true;
    }
    if creeper_close
        && let Some(c) = seen.iter().filter(|x| x.kind == Danger::Creeper).min_by(|a, b| a.t.distance.total_cmp(&b.t.distance))
        && retreat(g, rt, feet, c.t.center)
    {
        rt.status = "backing away from a creeper".to_owned();
        return true;
    }

    // Getting away from a creeper that camped by the wall: walk (digging) to the
    // spot picked, a minute at most, then back to the job with a fresh plan.
    if let Some((goal, since)) = rt.escape.clone() {
        // Each cell left gets filled (feet and head), so nothing follows down the
        // tunnel: the player moves in a closed pocket.
        if rt.left_from.is_some_and(|c| c != feet) {
            let c = rt.left_from.unwrap_or(feet);
            if !rt.path.iter().skip(rt.at).any(|st| st.pos == c) {
                rt.behind.push(add(c, [0, 1, 0]));
                rt.behind.push(c);
            }
            rt.left_from = Some(feet);
        }
        let pos = g.player.as_ref().map_or(center(feet), |p| [p.position.x, p.position.y, p.position.z]);
        rt.behind.retain(|c| rt.t.at(g, *c) & PASS != 0 && core::replaceable(g, *c));
        if let Some(&c) = rt.behind.iter().find(|c| !body_in(pos, **c)) {
            press(g, rt, &[]);
            stop_hands(g, rt);
            if rt.wait > 0 {
                rt.wait -= 1;
            } else {
                // One try per cell: a refused block (a mob standing there) isn't retried.
                place(g, s, rt, c);
                rt.behind.retain(|x| *x != c);
            }
            rt.status = "closing the tunnel behind".to_owned();
            rt.disturbed = true;
            return true;
        }
        let reached = goal.reached(g, &mut rt.t, feet);
        if reached || rt.tick.saturating_sub(since) > 2400 || walk(g, s, rt, &goal) {
            rt.escape = None;
            rt.behind.clear();
            rt.left_from = None;
            rt.fails = 0;
            clear_path(rt);
            rt.disturbed = true;
        } else {
            rt.status = "tunnelling away from mobs camped outside".to_owned();
            rt.disturbed = true;
            return true;
        }
    }

    // Fighting what comes close: face it, hit when the swing is charged, step back
    // while it charges.
    if let Some(m) = nearest.filter(|m| m.kind != Danger::Creeper && (m.t.distance < 6.0 || m.kind == Danger::Ranged && m.t.distance < 16.0)) {
        // One that never comes into reach (across a drop, behind bars) is left alone for a while.
        match rt.fight {
            Some((id, since, close)) if id == m.t.id => {
                let close = if m.t.distance < 2.9 { rt.tick } else { close };
                if rt.tick.saturating_sub(close) > 200 {
                    rt.ignore.insert(id, rt.tick + 600);
                    rt.fight = None;
                    return false;
                }
                rt.fight = Some((id, since, close));
            }
            _ => rt.fight = Some((m.t.id, rt.tick, rt.tick)),
        }
        stop_eating(g, rt);
        stop_hands(g, rt);
        if let Some(w) = weapon_slot(g) {
            g.selected = w;
        }
        core::look_at(g, m.t.center);
        let pos = g.player.as_ref().map(|p| [p.position.x, p.position.y, p.position.z]).unwrap_or(eye);
        let reach = m.t.distance < 2.9;
        if reach && core::charged(g) && g.mob_hit().is_some_and(|(i, _)| i == m.t.index) {
            core::click(g, false);
            press(g, rt, &[]);
        } else if reach && !core::charged(g) && m.t.distance < 2.2 && m.kind != Danger::Ranged {
            if !retreat(g, rt, feet, m.t.center) {
                press(g, rt, &[]);
            }
        } else if m.kind == Danger::Ranged && !reach {
            // Close the distance on a shooter, one safe step at a time.
            let dx = (m.t.center[0] - pos[0]).signum() as i32;
            let dz = (m.t.center[2] - pos[2]).signum() as i32;
            let step = if (m.t.center[0] - pos[0]).abs() > (m.t.center[2] - pos[2]).abs() { [dx, 0, 0] } else { [0, 0, dz] };
            if safe_cell(g, &mut rt.t, add(feet, step)) {
                press(g, rt, &[KeyCode::KeyW]);
            } else {
                press(g, rt, &[]);
            }
        } else {
            press(g, rt, &[]);
        }
        rt.status = format!("fighting {}", m.t.name);
        rt.disturbed = true;
        rt.moved = rt.tick;
        return true;
    }

    // Eating: when hungry, or hurt and too hungry to heal by itself.
    let calm = !threat && near(&[Danger::Melee, Danger::Ranged, Danger::Creeper], 10.0, false) == 0;
    let want_food = food <= 14 || (hp < 18.0 && food < 18);
    if rt.eat.is_some() || (calm && want_food) {
        press(g, rt, &[]);
        if eating(g, rt, calm && want_food) {
            rt.status = "eating".to_owned();
            rt.moved = rt.tick;
            return true;
        }
    }

    // Hurt, nothing in sight, nothing to eat with: wait for it to heal before more digging.
    if calm && hp <= 6.0 && food >= 18 {
        press(g, rt, &[]);
        stop_hands(g, rt);
        rt.status = format!("resting ({hp:.0}/20)");
        rt.moved = rt.tick;
        return true;
    }

    if calm && drop_junk(g, s, rt) {
        return true;
    }

    // The pickaxe: stop before it breaks, so it can still be repaired.
    if let Some(st) = g.inventory[..9].iter().flatten().find(|st| st.item.ends_with("_pickaxe")) {
        let max = g.res.items().get(&st.item).map_or(0, |i| i.max_damage as i32);
        if max > 0 && max - st.damage as i32 <= 8&& core::count(g, |i| i.ends_with("_pickaxe")) <= 1 {
            rt.worn_out = true;
        }
    }
    false
}

// ----------------------------------------------------------------- the path

pub fn hud(g: &Game<'_>, s: &State, batch: &mut HudBatch) {
    if !s.b("Baritone", "RenderPath") {
        return;
    }
    let Some(cam) = core::camera(g, batch) else {
        return;
    };
    RT.with(|rt| {
        let rt = rt.borrow();
        if rt.process.is_none() {
            return;
        }
        let lift = |p: P| [f64::from(p[0]) + 0.5, f64::from(p[1]) + 0.1, f64::from(p[2]) + 0.5];
        let path = &rt.path[rt.at.min(rt.path.len())..];
        for w in path.windows(2) {
            if let Some((a, b)) = cam.segment(lift(w[0].pos), lift(w[1].pos)) {
                core::line(g, batch, a, b, 2.0, 0xFF_00_FF_88);
            }
        }
        if let Some(end) = path.last() {
            let min = end.pos.map(f64::from);
            cam.box_edges(g, batch, min, [min[0] + 1.0, min[1] + 2.0, min[2] + 1.0], 0xFF_00_FF_88);
        }
        if let Some(m) = rt.mining.or(rt.digging) {
            let min = m.map(f64::from);
            cam.box_edges(g, batch, min, [min[0] + 1.0, min[1] + 1.0, min[2] + 1.0], 0xFF_FF_33_33);
        }
    });
}

// -------------------------------------------------------------- the commands

const HELP: &[&str] = &[
    "#goto <x y z | x z | y | waypoint | block>   walk there, digging and bridging if allowed",
    "#mine <block...> [count]  mine it: #mine diamond 8, #mine iron gold, #mine *_log 16",
    "#follow <player|mob|players|entities>",
    "#explore [x z]  #farm [range]  #tunnel [length] [width] [height]",
    "#thisway <blocks>  #surface  #come  #axis  #goal <...>  #path",
    "#pause  #resume  #stop  #status  #find <block>",
    "#set <setting> <value>  #settings   (.set Baritone <setting> <value> too)",
];

fn start(g: &mut Game<'_>, s: &mut State, rt: &mut Rt, process: Proc, message: String) {
    stop(g, rt);
    rt.slot = Some(g.selected);
    rt.tunnel_at = None;
    rt.fails = 0;
    rt.avoid.clear();
    rt.t.banned.clear();
    rt.mined_at = rt.tick;
    rt.process = Some(process);
    rt.moved = rt.tick;
    rt.status = "planning".to_owned();
    if let Some(p) = g.player.as_mut()
        && !s.on("Flight")
    {
        p.flying = false;
    }
    if s.on("Flight") {
        s.say("Baritone: Flight is on and will fight the path; turn it off");
    }
    if let Some(i) = s.index("Baritone") {
        s.set_on(i, true);
    }
    s.say(message);
}

fn parse_goal(g: &Game<'_>, s: &State, feet: P, words: &[&str]) -> Result<Goal, String> {
    let num = |w: &str, base: i32| -> Option<i32> {
        match w.strip_prefix('~') {
            Some("") => Some(base),
            Some(rel) => rel.parse::<i32>().ok().map(|v| base + v),
            None => w.parse::<f64>().ok().map(|v| v.floor() as i32),
        }
    };
    match words {
        [] => Err("Name a place: #goto <x y z | x z | y | waypoint | block>".to_owned()),
        [x, y, z] => match (num(x, feet[0]), num(y, feet[1]), num(z, feet[2])) {
            (Some(x), Some(y), Some(z)) => Ok(Goal::Block([x, y, z])),
            _ => Err("Those aren't coordinates".to_owned()),
        },
        [x, z] => match (num(x, feet[0]), num(z, feet[2])) {
            (Some(x), Some(z)) => Ok(Goal::Xz(x, z)),
            _ => Err("Those aren't coordinates".to_owned()),
        },
        [one] => {
            if let Some(y) = num(one, feet[1]) {
                return Ok(Goal::Y(y));
            }
            let dim = g.dim.id();
            if let Some(w) = s.waypoints.iter().find(|w| w.name.eq_ignore_ascii_case(one) && w.dim == dim) {
                return Ok(Goal::Block(w.pos.map(|v| v.floor() as i32)));
            }
            let names = block_names(one);
            let mut best: Option<(P, f64)> = None;
            let c = center(feet);
            let facts = g.res.facts();
            for dx in -48..=48 {
                for dz in -48..=48 {
                    for dy in -24..=24 {
                        let p = [feet[0] + dx, feet[1] + dy, feet[2] + dz];
                        let state = g.world.block(p[0], p[1], p[2]);
                        if !facts.is_air(state) && name_matches(&names, &facts.get(state).name) {
                            let d = core::dist(c, center(p));
                            if best.is_none_or(|b| d < b.1) {
                                best = Some((p, d));
                            }
                        }
                    }
                }
            }
            match best {
                Some((p, _)) => Ok(Goal::Beside(vec![p])),
                None => Err(format!("No waypoint or block called {one} nearby")),
            }
        }
        _ => Err("#goto takes x y z, x z, y, a waypoint or a block".to_owned()),
    }
}

/// A `#` line.
pub fn command(g: &mut Game<'_>, s: &mut State, line: &str) {
    RT.with(|rt| {
        let rt = &mut *rt.borrow_mut();
        run_command(g, s, rt, line);
    });
}

fn run_command(g: &mut Game<'_>, s: &mut State, rt: &mut Rt, line: &str) {
    let words: Vec<&str> = line.split_whitespace().collect();
    let Some(&cmd) = words.first() else {
        return;
    };
    let args = &words[1..];
    let feet = feet_cell(g, &mut rt.t);
    match cmd.to_ascii_lowercase().as_str() {
        "help" | "?" => {
            s.say("Baritone commands (also .b <command>):");
            for h in HELP {
                s.say(*h);
            }
        }
        "stop" | "cancel" | "forcecancel" | "s" => {
            stop(g, rt);
            s.turn_off("Baritone");
            s.say("Baritone stopped");
        }
        "pause" | "p" => {
            rt.paused = true;
            rt.auto_paused = false;
            s.say("Baritone paused: #resume");
        }
        "resume" | "r" | "unpause" => {
            rt.paused = false;
            rt.moved = rt.tick;
            s.say("Baritone resumed");
        }
        "status" | "eta" => {
            let line = match rt.process.is_some() {
                true => format!(
                    "{}; {} steps left, about {:.0}s",
                    rt.status,
                    rt.path.len().saturating_sub(rt.at + 1),
                    rt.cost * (rt.path.len().saturating_sub(rt.at + 1) as f64 / rt.path.len().max(1) as f64) / 20.0
                ),
                false => "Nothing running".to_owned(),
            };
            s.say(line);
        }
        "goto" | "go" | "goal" | "path" | "come" | "surface" | "top" | "axis" | "thisway" | "forward" => {
            let Some(feet) = feet else {
                return;
            };
            let word = cmd.to_ascii_lowercase();
            let goal = match word.as_str() {
                "come" => {
                    // The camera's place, which is the point of it with Freecam on.
                    let at = g.locator_camera.map_or([f64::from(feet[0]), f64::from(feet[1]), f64::from(feet[2])], |c| c.0);
                    Ok(Goal::Block(at.map(|v| v.floor() as i32)))
                }
                "surface" | "top" => Ok(Goal::Surface),
                "axis" => Ok(if feet[0].abs() < feet[2].abs() { Goal::Xz(0, feet[2]) } else { Goal::Xz(feet[0], 0) }),
                "thisway" | "forward" => match args.first().and_then(|v| v.parse::<f64>().ok()) {
                    Some(d) => {
                        let yaw = f64::from(g.yaw).to_radians();
                        Ok(Goal::Xz(
                            (f64::from(feet[0]) - yaw.sin() * d).floor() as i32,
                            (f64::from(feet[2]) + yaw.cos() * d).floor() as i32,
                        ))
                    }
                    None => Err("Usage: #thisway <blocks>".to_owned()),
                },
                "path" => rt.saved.clone().ok_or_else(|| "No goal yet: #goal <x y z>".to_owned()),
                _ => parse_goal(g, s, feet, args),
            };
            match goal {
                Err(e) => s.say(e),
                Ok(goal) if word == "goal" => {
                    s.say(format!("Goal set: {}, #path goes there", goal.label()));
                    rt.saved = Some(goal);
                }
                Ok(goal) => {
                    let label = goal.label();
                    start(g, s, rt, Proc::Goto(goal), format!("Baritone: going to {label}"));
                }
            }
        }
        "mine" => {
            if args.is_empty() {
                s.say("Usage: #mine <block...> [count]");
                return;
            }
            let (list, want) = match args.last().and_then(|v| v.parse::<u32>().ok()) {
                Some(n) => (&args[..args.len() - 1], Some(n)),
                None => (args, None),
            };
            let names: Vec<String> = list.iter().flat_map(|w| block_names(w)).collect();
            let known = names.iter().all(|n| {
                n.starts_with('*') || g.res.facts().states.iter().any(|f| &*f.name == n.as_str())
            });
            if names.is_empty() || !known {
                s.say(format!("No such block: {}", list.join(" ")));
                return;
            }
            rt.scan_names.clear();
            let text = match want {
                Some(n) => format!("Baritone: mining {n} of {}", list.join(", ")),
                None => format!("Baritone: mining {}", list.join(", ")),
            };
            start(g, s, rt, Proc::Mine { names, want, done: 0 }, text);
        }
        "follow" => {
            let who = args.join(" ");
            if who.is_empty() {
                s.say("Usage: #follow <player|mob|players|entities>");
                return;
            }
            start(g, s, rt, Proc::Follow { who: who.clone(), last: None }, format!("Baritone: following {who}"));
        }
        "explore" => {
            let Some(feet) = feet else {
                return;
            };
            let c = match (args.first().and_then(|v| v.parse().ok()), args.get(1).and_then(|v| v.parse().ok())) {
                (Some(x), Some(z)) => (x, z),
                _ => (feet[0], feet[2]),
            };
            start(g, s, rt, Proc::Explore { center: c, index: 1, goal: None }, format!("Baritone: exploring around {} {}", c.0, c.1));
        }
        "farm" => {
            let range = args.first().and_then(|v| v.parse().ok()).unwrap_or(24).clamp(4, 48);
            rt.crops_at = 0;
            start(g, s, rt, Proc::Farm { range }, format!("Baritone: farming within {range} blocks"));
        }
        "tunnel" => {
            let yaw = core::wrap(g.yaw);
            let dir = if yaw.abs() <= 45.0 {
                [0, 1]
            } else if yaw.abs() >= 135.0 {
                [0, -1]
            } else if yaw > 0.0 {
                [-1, 0]
            } else {
                [1, 0]
            };
            let n = |i: usize, d: i32| args.get(i).and_then(|v| v.parse().ok()).unwrap_or(d);
            let (left, width, height) = (n(0, 64).max(1), n(1, 1).clamp(1, 9), n(2, 2).clamp(2, 9));
            start(g, s, rt, Proc::Tunnel { dir, left, width, height }, format!("Baritone: tunnelling {left} blocks, {width}x{height}"));
        }
        "find" => {
            let Some(feet) = feet else {
                return;
            };
            let Some(word) = args.first() else {
                s.say("Usage: #find <block>");
                return;
            };
            let names = block_names(word);
            let facts = g.res.facts();
            let mut hits: Vec<P> = Vec::new();
            for dx in -64..=64 {
                for dz in -64..=64 {
                    for dy in -32..=32 {
                        let p = [feet[0] + dx, feet[1] + dy, feet[2] + dz];
                        let state = g.world.block(p[0], p[1], p[2]);
                        if !facts.is_air(state) && name_matches(&names, &facts.get(state).name) {
                            hits.push(p);
                        }
                    }
                }
            }
            let c = center(feet);
            hits.sort_by(|a, b| core::dist(c, center(*a)).total_cmp(&core::dist(c, center(*b))));
            s.say(format!("{} found nearby", hits.len()));
            for p in hits.iter().take(5) {
                s.say(format!("  {} {} {}", p[0], p[1], p[2]));
            }
        }
        "set" | "setting" => {
            let (Some(name), true) = (args.first(), args.len() >= 2) else {
                s.say("Usage: #set <setting> <value>; #settings lists them");
                return;
            };
            let value = args[1..].join(" ");
            let Some(i) = s.index("Baritone") else {
                return;
            };
            let line = match s.modules[i].sets.iter_mut().find(|x| x.name.eq_ignore_ascii_case(name)) {
                Some(st) => match st.val.parse(&value) {
                    Ok(()) => format!("Baritone {} = {}", st.name, st.val.show()),
                    Err(e) => format!("Baritone {}: {e}", st.name),
                },
                None => format!("No setting {name}"),
            };
            s.dirty = true;
            s.say(line);
        }
        "debug" => {
            let line = debug_line(g, rt);
            s.say(line);
        }
        "settings" => {
            if let Some(i) = s.index("Baritone") {
                let list: Vec<String> = s.modules[i].sets.iter().map(|st| format!("{} = {}", st.name, st.val.show())).collect();
                for l in list {
                    s.say(l);
                }
            }
        }
        "build" | "schematic" | "sel" | "selection" => {
            s.say("Building schematics isn't in Asteroid Client yet");
        }
        other => s.say(format!("Unknown Baritone command {other}: #help lists them")),
    }
}
