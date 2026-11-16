// Asteroid Client's BaseFinder, BaseESP and BaseFinderHUD: scores the loaded
// chunks for signs of players (placed blocks, storage, spawners, dug shafts,
// pets, deepslate that can't be natural) and remembers why. With the world's
// seed it also knows which of stone and deepslate each block of the y 1..7
// band should be, the way the game generates that band.
use super::Game;
use crate::game::asteroid_core::{self as core, State, fill};
use horizon_client::hud::HudBatch;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
};

/// `minecraft:deepslate` hashed the way `RandomSupport.seedFromHashOf` does (MD5).
const DEEPSLATE_HASH: (u64, u64) = (0x907d_dbc8_bae9_5dcf, 0xcfad_a31a_09f2_6a63);

/// Xoroshiro128++, the world generator's random source.
struct Xoro(u64, u64);
impl Xoro {
    fn from_seed(seed: i64) -> Self {
        fn mix(mut v: u64) -> u64 {
            v = (v ^ (v >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            v = (v ^ (v >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            v ^ (v >> 31)
        }
        let lo = (seed as u64) ^ 0x6A09_E667_F3BC_C909;
        let hi = lo.wrapping_add(0x9E37_79B9_7F4A_7C15);
        Self(mix(lo), mix(hi))
    }
    fn next(&mut self) -> u64 {
        let (l, m) = (self.0, self.1);
        let out = l.wrapping_add(m).rotate_left(17).wrapping_add(l);
        let m = m ^ l;
        self.0 = l.rotate_left(49) ^ m ^ (m << 21);
        self.1 = m.rotate_left(28);
        out
    }
    fn next_float(&mut self) -> f32 {
        (self.next() >> 40) as f32 * (1.0 / 16_777_216.0)
    }
}

/// `Mth.getSeed`.
fn block_seed(x: i32, y: i32, z: i32) -> i64 {
    let a = i64::from(x.wrapping_mul(3_129_871)) ^ i64::from(z).wrapping_mul(116_129_781) ^ i64::from(y);
    let a = a.wrapping_mul(a).wrapping_mul(42_317_861).wrapping_add(a.wrapping_mul(11));
    a >> 16
}

/// The positional factory of the deepslate gradient for a world seed.
fn deepslate_factory(seed: i64) -> (u64, u64) {
    let mut root = Xoro::from_seed(seed);
    let (lo, hi) = (root.next(), root.next());
    let mut named = Xoro(DEEPSLATE_HASH.0 ^ lo, DEEPSLATE_HASH.1 ^ hi);
    (named.next(), named.next())
}

/// Whether the `deepslate` vertical gradient (true at y 0, false at y 8) says deepslate here.
fn deepslate_at(factory: (u64, u64), x: i32, y: i32, z: i32) -> bool {
    if y <= 0 {
        return true;
    }
    if y >= 8 {
        return false;
    }
    let mut r = Xoro((block_seed(x, y, z) as u64) ^ factory.0, factory.1);
    r.next_float() < 1.0 - y as f32 / 8.0
}

/// The world seed as typed: a number, or text hashed as Java does.
fn parse_seed(text: &str) -> Option<i64> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(n) = t.parse::<i64>() {
        return Some(n);
    }
    let mut h = 0i32;
    for u in t.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(i32::from(u));
    }
    Some(i64::from(h))
}

#[derive(Clone, Copy, PartialEq)]
enum Sign {
    None,
    Storage,
    Spawner,
    Placed(f32),
    /// Deepslate that doesn't generate like this.
    Deepslate(f32),
    /// Stone or deepslate, for the seeded check.
    Base(bool),
    /// A block of a generated structure, by index into `STRUCTURES`.
    Marker(u8),
}

/// What generates blocks that would otherwise look placed.
const STRUCTURES: [&str; 8] = [
    "an ancient city", "a ruined portal", "a village", "a trial chamber", "a stronghold", "a bastion", "an ocean monument", "an end city",
];

fn classify(g: &Game<'_>, state: horizon_world::registry::BlockStateId) -> Sign {
    let facts = g.res.facts();
    if facts.is_air(state) {
        return Sign::None;
    }
    let name = &*facts.get(state).name;
    let Some(n) = name.strip_prefix("minecraft:") else {
        return Sign::None;
    };
    match n {
        "stone" => return Sign::Base(false),
        "deepslate" => {
            let text = g.state_string(state);
            return if text.contains("axis=x") || text.contains("axis=z") {
                Sign::Deepslate(3.0)
            } else {
                Sign::Base(true)
            };
        }
        "chest" | "trapped_chest" | "barrel" => return Sign::Storage,
        "spawner" => return Sign::Spawner,
        "sculk_shrieker" | "sculk_catalyst" | "reinforced_deepslate" | "deepslate_tiles" | "cracked_deepslate_tiles"
        | "deepslate_bricks" | "cracked_deepslate_bricks" | "deepslate_brick_stairs" | "deepslate_brick_slab" | "deepslate_brick_wall"
        | "deepslate_tile_stairs" | "deepslate_tile_slab" | "deepslate_tile_wall" | "polished_deepslate" | "polished_deepslate_stairs"
        | "polished_deepslate_slab" | "polished_deepslate_wall" | "chiseled_deepslate" => return Sign::Marker(0),
        "crying_obsidian" => return Sign::Marker(1),
        "bell" => return Sign::Marker(2),
        "vault" | "trial_spawner" | "tuff_bricks" | "chiseled_tuff" | "chiseled_tuff_bricks" => return Sign::Marker(3),
        "end_portal_frame" | "infested_stone_bricks" | "infested_mossy_stone_bricks" => return Sign::Marker(4),
        "gilded_blackstone" | "polished_blackstone_bricks" | "cracked_polished_blackstone_bricks" => return Sign::Marker(5),
        "prismarine" | "prismarine_bricks" | "dark_prismarine" => return Sign::Marker(6),
        "purpur_block" | "purpur_pillar" | "end_stone_bricks" => return Sign::Marker(7),
        "moss_carpet" | "pale_moss_carpet" => return Sign::None,
        _ => {}
    }
    if n.ends_with("shulker_box") {
        return Sign::Placed(6.0);
    }
    let w = match n {
        "beacon" | "netherite_block" | "diamond_block" | "emerald_block" | "respawn_anchor" | "lodestone" | "conduit" => 10.0,
        "ender_chest" | "enchanting_table" | "beehive" | "crafter" | "dragon_head" => 4.0,
        "anvil" | "chipped_anvil" | "damaged_anvil" | "iron_block" | "observer" | "piston" | "sticky_piston" | "comparator" | "repeater" | "redstone_lamp" | "daylight_detector" | "target" => 3.0,
        "hopper" | "dropper" | "dispenser" | "note_block" | "jukebox" | "brewing_stand" | "composter" | "scaffolding" | "tinted_glass" | "sea_lantern" | "shroomlight" => 2.0,
        "crafting_table" | "furnace" | "blast_furnace" | "smoker" | "lectern" | "grindstone" | "stonecutter" | "smithing_table" | "loom" | "cartography_table" | "fletching_table" | "cauldron" | "glowstone" | "gold_block" => 1.0,
        "cobbled_deepslate" | "cobbled_deepslate_slab" | "cobbled_deepslate_stairs" | "cobbled_deepslate_wall" => return Sign::Deepslate(1.0),
        "farmland" | "torch" | "wall_torch" | "glass" | "glass_pane" | "iron_bars" | "campfire" | "ladder" | "obsidian" => 0.3,
        _ if n.ends_with("_bed") => 1.0,
        _ if n.ends_with("_concrete") || n.ends_with("_concrete_powder") => 2.0,
        _ if n.ends_with("_glazed_terracotta") || n.ends_with("_stained_glass") || n.ends_with("_stained_glass_pane") => 1.5,
        _ if n.ends_with("_sign") || n.ends_with("_hanging_sign") || n.ends_with("_banner") => 1.0,
        _ if n.ends_with("_wool") || n.ends_with("_carpet") || n.ends_with("_shelf") => 0.5,
        _ => return Sign::None,
    };
    Sign::Placed(w)
}

/// What a chunk looks like.
#[derive(Clone, Default)]
pub struct Sus {
    pub chunk: (i32, i32),
    pub score: f32,
    pub why: String,
    /// The strongest signs, for BaseESP.
    pub evidence: Vec<[i32; 3]>,
    /// Near the middle of what was found.
    pub at: [i32; 3],
    /// A generated structure's blocks are here.
    pub structure: bool,
}

/// A chunk being looked at, a few columns a tick.
#[derive(Default)]
struct Job {
    chunk: (i32, i32),
    /// The heights worth reading, from the sections' palettes: top first.
    ranges: Vec<(i32, i32)>,
    column: i32,
    storage: u32,
    spawners: u32,
    placed: HashMap<u32, (f32, u32)>,
    deepslate: f32,
    deep_count: u32,
    agree: u32,
    differ: u32,
    mismatch: Vec<[i32; 3]>,
    evidence: Vec<(f32, [i32; 3])>,
    shaft: i32,
    markers: [u32; 8],
}

#[derive(Default)]
struct Rt {
    job: Option<Job>,
    chunks: HashMap<(i32, i32), Sus>,
    kinds: HashMap<u32, Sign>,
    queue: VecDeque<(i32, i32)>,
    stamp: HashMap<(i32, i32), u64>,
    alerted: HashSet<(i32, i32)>,
    dim: String,
    /// Seed text and its factory.
    seed: (String, Option<(u64, u64)>),
    /// How the seeded check has gone: blocks that agreed, blocks that didn't.
    seed_seen: (u64, u64),
    wrong_seed_said: bool,
}
thread_local! {
    static RT: RefCell<Rt> = RefCell::new(Rt::default());
}

fn wanted(s: &State) -> bool {
    s.on("BaseFinder") || s.on("BaseESP") || s.on("BaseFinderHUD")
}

pub fn tick(g: &mut Game<'_>, s: &mut State) {
    if !wanted(s) {
        return;
    }
    RT.with(|rt| {
        let rt = &mut *rt.borrow_mut();
        let dim = g.dim.id().to_owned();
        if dim != rt.dim {
            rt.dim = dim;
            rt.job = None;
            rt.chunks.clear();
            rt.queue.clear();
            rt.stamp.clear();
            rt.alerted.clear();
        }
        let seed_text = s.t("BaseFinder", "Seed").trim().to_owned();
        if seed_text != rt.seed.0 {
            let factory = parse_seed(&seed_text).map(deepslate_factory);
            rt.seed = (seed_text, factory);
            rt.seed_seen = (0, 0);
            rt.wrong_seed_said = false;
            rt.stamp.clear();
        }
        let Some(feet) = core::feet(g) else {
            return;
        };
        let (cx, cz) = (feet[0].div_euclid(16), feet[2].div_euclid(16));
        let r = s.n("BaseFinder", "Radius").max(1.0) as i32;
        if rt.job.is_none() {
            if rt.queue.is_empty() {
                let mut order = Vec::new();
                for dx in -r..=r {
                    for dz in -r..=r {
                        let c = (cx + dx, cz + dz);
                        // A chunk is looked at again after a minute.
                        if rt.stamp.get(&c).is_none_or(|t| s.ticks.saturating_sub(*t) > 1200) {
                            order.push(c);
                        }
                    }
                }
                order.sort_by_key(|(x, z)| (x - cx).abs().max((z - cz).abs()));
                rt.queue.extend(order);
            }
            while let Some((x, z)) = rt.queue.pop_front() {
                if g.world.loaded(x * 16, z * 16) {
                    rt.stamp.insert((x, z), s.ticks);
                    rt.job = Some(start_job(g, rt, (x, z)));
                    break;
                }
            }
        }
        // A sliver of each tick, so the frame rate doesn't notice.
        let deadline = std::time::Instant::now() + std::time::Duration::from_micros(1500);
        if let Some(mut job) = rt.job.take() {
            if scan_columns(g, rt, &mut job, deadline) {
                let (x, z) = job.chunk;
                rt.seed_seen.0 += u64::from(job.agree);
                rt.seed_seen.1 += u64::from(job.differ);
                let sus = finish(g, job);
                if sus.score > 0.0 {
                    rt.chunks.insert((x, z), sus);
                } else {
                    rt.chunks.remove(&(x, z));
                }
            } else {
                rt.job = Some(job);
            }
        }
        if s.ticks.is_multiple_of(100) && s.on("BaseFinder") && s.b("BaseFinder", "Alerts") {
            let minimum = s.n("BaseFinder", "MinScore") as f32;
            let list = ranked(&rt.chunks, minimum);
            for c in list {
                if rt.alerted.insert(c.chunk) {
                    s.say(format!("Possible base at {} {}: score {:.0}, {}", c.chunk.0 * 16 + 8, c.chunk.1 * 16 + 8, c.score, c.why));
                }
            }
        }
        let (agree, differ) = rt.seed_seen;
        if !rt.wrong_seed_said && agree + differ > 2000 && differ * 3 > agree {
            rt.wrong_seed_said = true;
            rt.seed.1 = None;
            rt.stamp.clear();
            s.say("BaseFinder: that seed doesn't match this world (the deepslate band disagrees), so it is ignored");
        }
    });
}

/// Which heights of a chunk can hold anything of interest. A section whose
/// palette holds none of it is skipped unread, which is most of the world.
fn start_job(g: &Game<'_>, rt: &mut Rt, chunk: (i32, i32)) -> Job {
    let overworld = g.dim.id() == "minecraft:overworld";
    let seeded = rt.seed.1.is_some();
    let (y_lo, y_hi) = if overworld { (-64, 320) } else { (0, 256) };
    let mut ranges: Vec<(i32, i32)> = Vec::new();
    let mut y0 = y_lo;
    while y0 < y_hi {
        let y1 = y0 + 15;
        let lo = [chunk.0 * 16, y0, chunk.1 * 16];
        let hi = [chunk.0 * 16 + 15, y1, chunk.1 * 16 + 15];
        let kinds = &mut rt.kinds;
        let wanted = g.world.may_hold(lo, hi, |state| {
            if g.res.facts().is_air(state) {
                // Only a shaft makes air interesting, and only underground.
                return y0 < 56;
            }
            match *kinds.entry(state.0).or_insert_with(|| classify(g, state)) {
                Sign::None => false,
                Sign::Base(deep) => overworld && ((deep && y1 >= 8) || (seeded && y0 < 8 && y1 >= 1)),
                _ => true,
            }
        });
        if wanted {
            // Runs of sections stay one range, so a shaft can cross them.
            match ranges.last_mut() {
                Some(r) if r.1 + 1 == y0 => r.1 = y1,
                _ => ranges.push((y0, y1)),
            }
        }
        y0 += 16;
    }
    ranges.reverse();
    Job { chunk, ranges, ..Job::default() }
}

/// Reads columns until `deadline`; true when the chunk is done.
fn scan_columns(g: &Game<'_>, rt: &mut Rt, job: &mut Job, deadline: std::time::Instant) -> bool {
    let overworld = g.dim.id() == "minecraft:overworld";
    let factory = rt.seed.1;
    let (cx, cz) = job.chunk;
    let ranges = std::mem::take(&mut job.ranges);
    while job.column < 256 {
        if std::time::Instant::now() >= deadline {
            job.ranges = ranges;
            return false;
        }
        let (x, z) = (cx * 16 + (job.column & 15), cz * 16 + (job.column >> 4));
        job.column += 1;
        for (lo, hi) in &ranges {
            let mut run = 0i32;
            for y in (*lo..=*hi).rev() {
                let state = g.world.block(x, y, z);
                if g.res.facts().is_air(state) {
                    // A dug shaft: a tall air column walled in all round, underground.
                    if y < 56 {
                        let walls = [[1, 0], [-1, 0], [0, 1], [0, -1]]
                            .iter()
                            .filter(|d| !g.res.facts().is_air(g.world.block(x + d[0], y, z + d[1])))
                            .count();
                        run = if walls == 4 { run + 1 } else { 0 };
                        if run >= 10 && run > job.shaft {
                            job.shaft = run;
                            if run == 10 {
                                job.evidence.push((6.0, [x, y, z]));
                            }
                        }
                    }
                    continue;
                }
                run = 0;
                let sign = *rt.kinds.entry(state.0).or_insert_with(|| classify(g, state));
                match sign {
                    Sign::None => {}
                    Sign::Marker(i) => job.markers[usize::from(i)] += 1,
                    Sign::Storage => {
                        job.storage += 1;
                        job.evidence.push((3.0, [x, y, z]));
                    }
                    Sign::Spawner => {
                        job.spawners += 1;
                        job.evidence.push((4.0, [x, y, z]));
                    }
                    Sign::Placed(w) => {
                        job.placed.entry(state.0).or_insert((w, 0)).1 += 1;
                        if w >= 1.0 {
                            job.evidence.push((w, [x, y, z]));
                        }
                    }
                    Sign::Deepslate(w) => {
                        job.deepslate += w;
                        job.deep_count += 1;
                        job.evidence.push((w, [x, y, z]));
                    }
                    Sign::Base(is_deep) if overworld => {
                        if is_deep && y >= 8 {
                            job.deepslate += 2.0;
                            job.deep_count += 1;
                            job.evidence.push((2.0, [x, y, z]));
                        } else if (1..8).contains(&y)
                            && let Some(f) = factory
                        {
                            if deepslate_at(f, x, y, z) == is_deep {
                                job.agree += 1;
                            } else {
                                job.differ += 1;
                                job.mismatch.push([x, y, z]);
                            }
                        }
                    }
                    Sign::Base(_) => {}
                }
            }
        }
    }
    true
}

/// What the chunk's signs add up to.
fn finish(g: &Game<'_>, job: Job) -> Sus {
    let Job { chunk: (cx, cz), storage, spawners, placed, deepslate, deep_count, agree, differ, mismatch, mut evidence, shaft, markers, .. } = job;
    // A seed that fits this chunk: what it disagrees with was changed by someone.
    let seeded = agree + differ >= 40 && u64::from(differ) * 10 < u64::from(agree + differ);
    let mut seed_hits = 0;
    if seeded && differ >= 3 {
        seed_hits = differ;
        evidence.extend(mismatch.iter().take(16).map(|p| (2.5, *p)));
    }
    // Pets, named mobs and item frames.
    let in_chunk = |x: f64, z: f64| (x / 16.0).floor() as i32 == cx && (z / 16.0).floor() as i32 == cz;
    let mut pets = 0u32;
    for m in g.mobs.iter().filter(|m| in_chunk(m.position.x, m.position.z)) {
        if m.extra.owner.is_some() || m.custom_name.is_some() {
            pets += 1;
            evidence.push((3.0, [m.position.x.floor() as i32, m.position.y.floor() as i32, m.position.z.floor() as i32]));
        }
    }
    let frames = g.item_frames.iter().filter(|f| {
        let (lo, _) = f.bounds();
        in_chunk(lo[0], lo[2])
    }).count() as u32;
    let stands = g.armor_stands.iter().filter(|a| in_chunk(a.position.x, a.position.z)).count() as u32;

    // Square roots keep a whole floor of one block from outweighing everything.
    let root = |v: f32| v.max(0.0).sqrt();
    let mut score = 0.0_f32;
    // A generated structure's own blocks aren't anyone's doing.
    let structure = (0..STRUCTURES.len()).find(|&i| markers[i] >= if i == 2 { 1 } else { 3 }).map(|i| STRUCTURES[i]);
    // One dungeon's spawner and chests are natural enough to count for little.
    score += match storage {
        0..=2 => storage as f32 * 0.7,
        n => 1.4 + 3.0 * root(n as f32 - 2.0),
    };
    score += spawners as f32 * if storage > 2 { 5.0 } else { 2.0 };
    let mut placed_total = 0u32;
    let mut top: Vec<(f32, u32, u32)> = Vec::new();
    for (id, (w, n)) in &placed {
        score += w * root(*n as f32);
        placed_total += n;
        top.push((*w, *n, *id));
    }
    score += 2.0 * root(deepslate);
    if shaft >= 10 {
        score += 4.0 + (shaft as f32 - 10.0) * 0.25;
    }
    score += pets as f32 * 4.0 + frames as f32 * 1.5 + stands as f32 * 1.5;
    if structure.is_some() {
        score *= 0.1;
    }
    score += 3.0 * root(seed_hits as f32);

    let mut why: Vec<String> = Vec::new();
    if let Some(name) = structure {
        why.push(format!("looks like {name}"));
    }
    if storage > 0 {
        why.push(format!("{storage} storage"));
    }
    if spawners > 0 {
        why.push(format!("{spawners} spawner{}", if spawners == 1 { "" } else { "s" }));
    }
    top.sort_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)));
    if placed_total > 0 {
        let names: Vec<String> = top
            .iter()
            .take(2)
            .map(|(_, _, id)| {
                let state = horizon_world::registry::BlockStateId(*id);
                core::pretty(&g.res.facts().get(state).name).to_lowercase()
            })
            .collect();
        why.push(format!("{placed_total} placed ({})", names.join(", ")));
    }
    if shaft >= 10 {
        why.push(format!("{shaft}m shaft"));
    }
    if pets > 0 {
        why.push(format!("{pets} pet{}", if pets == 1 { "" } else { "s" }));
    }
    if frames + stands > 0 {
        why.push(format!("{} frames/stands", frames + stands));
    }
    if deep_count > 0 {
        why.push(format!("{deep_count} odd deepslate"));
    }
    if seed_hits > 0 {
        why.push(format!("{seed_hits} blocks unlike the seed"));
    }
    evidence.sort_by(|a, b| b.0.total_cmp(&a.0));
    evidence.truncate(24);
    let at = evidence.first().map_or([cx * 16 + 8, 64, cz * 16 + 8], |e| e.1);
    Sus {
        chunk: (cx, cz),
        score,
        why: why.join(", "),
        evidence: evidence.into_iter().map(|e| e.1).collect(),
        at,
        structure: structure.is_some(),
    }
}

/// The chunks at or over `minimum`, best first. A structure's outskirts have none
/// of its telling blocks, so what sits beside one counts for a quarter.
fn ranked(chunks: &HashMap<(i32, i32), Sus>, minimum: f32) -> Vec<Sus> {
    let mut list: Vec<Sus> = chunks
        .values()
        .filter(|c| c.score >= minimum)
        .filter_map(|c| {
            let mut near = false;
            for dx in -2..=2 {
                for dz in -2..=2 {
                    near |= chunks.get(&(c.chunk.0 + dx, c.chunk.1 + dz)).is_some_and(|o| o.structure);
                }
            }
            let score = if near { c.score * 0.25 } else { c.score };
            (score >= minimum).then(|| Sus { score, ..c.clone() })
        })
        .collect();
    list.sort_by(|a, b| b.score.total_cmp(&a.score));
    list
}

fn top(minimum: f32, count: usize) -> Vec<Sus> {
    RT.with(|rt| {
        let mut list = ranked(&rt.borrow().chunks, minimum);
        list.truncate(count);
        list
    })
}

fn heat(score: f32, minimum: f32) -> u32 {
    let t = ((score - minimum) / (minimum * 2.0).max(1.0)).clamp(0.0, 1.0);
    let r = 0xFF;
    let gr = (0xD0 as f32 * (1.0 - t)) as u32;
    0xFF00_0000 | (r << 16) | (gr << 8) | 0x30
}

/// BaseESP: the chunks' outlines, why they look suspicious, and the blocks that say so.
pub fn esp(g: &Game<'_>, s: &State, batch: &mut HudBatch, cam: &core::Cam) {
    if !s.on("BaseESP") {
        return;
    }
    let eye = g.eye_position().unwrap_or_default();
    let minimum = s.n("BaseESP", "MinScore") as f32;
    for c in top(minimum, 12) {
        let color = heat(c.score, minimum);
        let (x, z) = (f64::from(c.chunk.0 * 16), f64::from(c.chunk.1 * 16));
        let y = f64::from(c.at[1]);
        cam.box_edges(g, batch, [x, y - 6.0, z], [x + 16.0, y + 10.0, z + 16.0], color & 0xB0FF_FFFF);
        if s.b("BaseESP", "Evidence") {
            for p in &c.evidence {
                let min = p.map(f64::from);
                if core::dist(eye, min) < 96.0 {
                    cam.box_edges(g, batch, min, [min[0] + 1.0, min[1] + 1.0, min[2] + 1.0], color);
                }
            }
        }
        if s.b("BaseESP", "Labels")
            && let Some((sx, sy)) = cam.project([x + 8.0, y + 11.0, z + 8.0])
        {
            let away = core::dist(eye, [x + 8.0, y, z + 8.0]);
            if away > 128.0 {
                continue;
            }
            let head = format!("Base {:.0}  {away:.0}m", c.score);
            // Only what is near says why, so far chunks don't pile up on the screen.
            let near = away < 64.0;
            let wide = if near { g.native_text_width(&c.why).min(190.0) } else { 0.0 };
            let w = g.native_text_width(&head).max(wide) * 0.75;
            fill(g, batch, sx - w / 2.0 - 3.0, sy - 3.0, w + 6.0, if near { 19.0 } else { 10.5 }, 0xC0_10_0A_14);
            fill(g, batch, sx - w / 2.0 - 3.0, sy - 3.0, w + 6.0, 1.0, color);
            core::text_k(g, batch, &head, sx - w / 2.0, sy - 1.0, color & 0xFF_FF_FF, 0.75);
            if near {
                core::text_k(g, batch, &core::clip_text(g, &c.why, 190.0), sx - w / 2.0, sy + 7.0, 0xD8_D8_E0, 0.75);
            }
        }
    }
}

/// An arrow at `(x, y)`, turned `angle` radians clockwise from straight up.
fn arrow(g: &Game<'_>, batch: &mut HudBatch, x: f32, y: f32, angle: f32, size: f32, argb: u32) {
    let (sn, cs) = angle.sin_cos();
    let at = |ax: f32, ay: f32| (x + ax * cs - ay * sn, y + ax * sn + ay * cs);
    let tip = at(0.0, -size);
    let left = at(-size * 0.7, size * 0.8);
    let right = at(size * 0.7, size * 0.8);
    let back = at(0.0, size * 0.3);
    core::quad(g, batch, [tip, right, back, left], argb);
}

/// BaseFinderHUD: the most suspicious chunks, each with an arrow pointing to it.
pub fn hud(g: &Game<'_>, s: &State, batch: &mut HudBatch) {
    if !s.on("BaseFinderHUD") || s.gui.open {
        return;
    }
    let eye = g.eye_position().unwrap_or_default();
    let minimum = s.n("BaseFinder", "MinScore") as f32;
    let rows = top(minimum, s.n("BaseFinderHUD", "Rows") as usize);
    let k = 0.75;
    let (w, row) = (150.0, 17.0);
    // Under the left column, clear of the module list on the right.
    let x = 3.0;
    let y0 = core::LEFT_Y.with(|c| c.get()).max(14.0);
    let h = 13.0 + rows.len().max(1) as f32 * row;
    let accent = core::accent(s);
    fill(g, batch, x, y0, w, h, 0xC0_10_0A_14);
    fill(g, batch, x, y0, w, 1.0, 0xFF00_0000 | accent);
    core::text_k(g, batch, "Base Finder", x + 4.0, y0 + 3.0, 0xFF_FF_FF, k);
    if rows.is_empty() {
        core::text_k(g, batch, "Nothing suspicious yet", x + 4.0, y0 + 14.0, 0x9A_9A_A8, k);
        return;
    }
    for (i, c) in rows.iter().enumerate() {
        let ry = y0 + 13.0 + i as f32 * row;
        let (tx, tz) = (f64::from(c.chunk.0 * 16 + 8), f64::from(c.chunk.1 * 16 + 8));
        let (dx, dz) = (tx - eye[0], tz - eye[2]);
        // The direction on the ground, against where the view points.
        let want = (-dx).atan2(dz).to_degrees() as f32;
        let turn = core::wrap(want - g.yaw).to_radians();
        arrow(g, batch, x + 9.0, ry + 7.0, turn, 5.0, heat(c.score, minimum));
        let head = format!("{:.0}  {:.0}m  {} {}", c.score, (dx * dx + dz * dz).sqrt(), tx as i32, tz as i32);
        core::text_k(g, batch, &head, x + 18.0, ry + 1.0, 0xFF_FF_FF, k);
        core::text_k(g, batch, &core::clip_text(g, &c.why, (w - 22.0) / k), x + 18.0, ry + 8.5, 0xA8_A8_B4, k);
    }
}

/// `.basefinder`'s list.
pub fn report(s: &mut State, eye: [f64; 3]) {
    let minimum = s.n("BaseFinder", "MinScore") as f32;
    let list = top(minimum, 5);
    let _ = eye;
    if list.is_empty() {
        s.say("BaseFinder: nothing suspicious in the chunks looked at so far");
    }
    for c in list {
        s.say(format!("{:.0} at {} {}: {}", c.score, c.chunk.0 * 16 + 8, c.chunk.1 * 16 + 8, c.why));
    }
    let (agree, differ) = RT.with(|rt| rt.borrow().seed_seen);
    if agree + differ > 0 {
        s.say(format!("Seed check: {agree} blocks agree, {differ} differ"));
    }
}
