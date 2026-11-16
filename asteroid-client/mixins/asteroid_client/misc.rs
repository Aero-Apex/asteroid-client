// Asteroid Client's misc modules (AntiAFK, Spam, Announcer, Notifier,
// MessageAura, AutoReconnect) and its `.` chat commands.
use super::Game;
use crate::game::asteroid_core::{self as core, CATS, State, Val, Waypoint};
use std::{
    cell::RefCell,
    collections::HashSet,
    time::{Duration, Instant},
};

#[derive(Default)]
struct Rt {
    afk: u64,
    spam: u64,
    announce: u64,
    walked: f64,
    last: Option<[f64; 3]>,
    seen: HashSet<String>,
    greeted: HashSet<String>,
    disconnected: Option<Instant>,
}
thread_local! {
    static RT: RefCell<Rt> = RefCell::new(Rt::default());
}

pub fn toggled(g: &mut Game<'_>, s: &mut State, name: &str, on: bool) {
    // Panic: everything off at once, and Panic itself back off. Done before the
    // module's state is borrowed, since turning a module off comes back here.
    if name == "VClip" && on {
        // One move through the blocks, then off again.
        let d = s.n("VClip", "Distance");
        if let Some(p) = g.player.as_mut() {
            p.position.y += d;
            p.old_position = p.position;
        }
        s.turn_off("VClip");
        return;
    }
    if name == "Panic" && on {
        for i in 0..s.modules.len() {
            if s.modules[i].name != "Panic" && s.modules[i].on {
                s.set_on(i, false);
            }
        }
        s.turn_off("Panic");
        return;
    }
    RT.with(|rt| {
        let rt = &mut *rt.borrow_mut();
        match (name, on) {
            ("Notifier", true) => {
                rt.seen = core::player_names(g).into_values().collect();
            }
            ("MessageAura", true) => rt.greeted.clear(),
            ("Announcer", true) => {
                rt.walked = 0.0;
                rt.announce = s.ticks;
            }
            _ => {}
        }
    });
}

fn chat(g: &mut Game<'_>, message: &str) {
    if g.remote.is_some() {
        g.send_chat(message);
    }
}

pub fn tick(g: &mut Game<'_>, s: &mut State) {
    if s.on("FastPlace") {
        // Blocks can be placed as fast as the clicks come.
        g.use_delay = 0;
        g.clicks.right_click_delay = 0;
    }
    RT.with(|rt| {
        let rt = &mut *rt.borrow_mut();
        let now = s.ticks;
        if g.player.is_none() {
            return;
        }
        // AntiAFK
        if s.on("AntiAFK") && now >= rt.afk + (s.n("AntiAFK", "Interval") * 20.0) as u64 {
            rt.afk = now;
            if s.b("AntiAFK", "Spin") {
                g.yaw += 90.0;
            }
            if s.b("AntiAFK", "Swing") {
                g.swing_arm();
            }
            if s.b("AntiAFK", "Jump")
                && let Some(p) = g.player.as_mut()
                && p.ground.on_ground
            {
                p.velocity.y = 0.42;
            }
        }
        // Spam
        if s.on("Spam") && now >= rt.spam + (s.n("Spam", "Delay") * 20.0) as u64 {
            rt.spam = now;
            let message = s.t("Spam", "Message").to_owned();
            if !message.is_empty() {
                chat(g, &message);
            }
        }
        // Announcer
        if let Some(p) = g.player.as_ref() {
            let pos = [p.position.x, p.position.y, p.position.z];
            if let Some(last) = rt.last {
                let d = core::dist(pos, last);
                if d < 10.0 {
                    rt.walked += d;
                }
            }
            rt.last = Some(pos);
        }
        if s.on("Announcer") && now >= rt.announce + (s.n("Announcer", "Delay") * 20.0) as u64 {
            rt.announce = now;
            if rt.walked >= 1.0 {
                let message = format!("I just moved {:.0} blocks thanks to {}!", rt.walked, core::NAME);
                chat(g, &message);
            }
            rt.walked = 0.0;
        }
        // Notifier and MessageAura
        let names: HashSet<String> = core::player_names(g).into_values().collect();
        if s.on("Notifier") {
            for n in names.difference(&rt.seen) {
                if s.b("Notifier", "Enter") {
                    s.say(format!("{n} entered your visual range"));
                }
            }
            for n in rt.seen.difference(&names) {
                if s.b("Notifier", "Leave") {
                    s.say(format!("{n} left your visual range"));
                }
            }
        }
        rt.seen = names;
        if s.on("MessageAura") {
            let close: Vec<String> = core::targets(g, s, 8.0, true, false, false)
                .into_iter()
                .map(|t| t.name)
                .filter(|n| !rt.greeted.contains(n))
                .collect();
            if let Some(name) = close.into_iter().next() {
                let message = format!("/msg {name} {}", s.t("MessageAura", "Message"));
                rt.greeted.insert(name);
                chat(g, &message);
            }
        }
    });
}

/// Every frame, in a world or not: AutoReconnect.
pub fn frame(g: &mut Game<'_>, s: &mut State) {
    RT.with(|rt| {
        let rt = &mut *rt.borrow_mut();
        let kicked = g.remote.is_none()
            && g.title.as_ref().is_some_and(|t| {
                matches!(
                    t.mode,
                    horizon_client::title::Mode::Multiplayer(horizon_client::multiplayer::Screen::Disconnected { .. })
                )
            });
        if !kicked || !s.on("AutoReconnect") {
            rt.disconnected = None;
            return;
        }
        let since = *rt.disconnected.get_or_insert_with(Instant::now);
        if since.elapsed() < Duration::from_secs_f64(s.n("AutoReconnect", "Delay")) {
            return;
        }
        rt.disconnected = None;
        if let Some((address, name)) = s.last_server.clone() {
            s.say(format!("Reconnecting to {name}"));
            let _ = g.connect_server(&address, &name);
        }
    });
}

// ---------------------------------------------------------------- commands

const HELP: &[&str] = &[
    ".toggle <module>  (.t)       turn a module on or off",
    ".bind <module> <key|none>    bind a key (.binds lists them)",
    ".set <module> <setting> <v>  change a setting",
    ".settings <module>           show a module's settings",
    ".modules [category]          list the modules",
    ".reset <module>              put a module's settings back",
    ".hide <module>               hide it from the module list",
    ".friends add|remove|list <name>",
    ".waypoint add <name> [x y z] | remove <name> | list",
    ".vclip <blocks>  .hclip <blocks>",
    ".say <text>  .drop  .fov <n>  .coords  .panic  .gui",
    ".name <name|off>  show another name, on your screen only",
    ".credits  who this is modelled on",
    "#help  the Baritone commands: #goto, #mine, #follow, #explore, #farm, #tunnel...",
];

/// `key` as a key name: `r` → `key.keyboard.r`, `rshift` → right shift.
fn key_name(key: &str) -> String {
    let k = key.to_ascii_lowercase();
    if k.starts_with("key.") {
        return k;
    }
    let named = match k.as_str() {
        "rshift" | "right_shift" => "right.shift",
        "lshift" | "left_shift" => "left.shift",
        "rctrl" | "right_ctrl" | "right_control" => "right.control",
        "lctrl" | "left_ctrl" | "left_control" => "left.control",
        "ralt" | "right_alt" => "right.alt",
        "lalt" | "left_alt" => "left.alt",
        "mouse4" => return "key.mouse.4".to_owned(),
        "mouse5" => return "key.mouse.5".to_owned(),
        "middle" | "mouse3" => return "key.mouse.middle".to_owned(),
        other => return format!("key.keyboard.{}", other.replace('_', ".")),
    };
    format!("key.keyboard.{named}")
}

fn module_or_say(s: &mut State, name: Option<&str>) -> Option<usize> {
    let Some(name) = name else {
        s.say("Name a module: .modules lists them");
        return None;
    };
    let i = s.index(name);
    if i.is_none() {
        s.say(format!("No module called {name}"));
    }
    i
}

pub fn command(g: &mut Game<'_>, s: &mut State, line: &str) {
    let words: Vec<&str> = line.split_whitespace().collect();
    let Some(&cmd) = words.first() else {
        return;
    };
    let arg = |i: usize| words.get(i).copied();
    match cmd.to_ascii_lowercase().as_str() {
        "help" | "commands" => {
            s.say(format!("{} {} commands:", core::NAME, core::VERSION));
            for h in HELP {
                s.say(*h);
            }
        }
        "b" => crate::game::asteroid_baritone::command(g, s, &words[1..].join(" ")),
        "credits" | "about" => {
            s.say(format!("{} {}, written from scratch in Rust for Horizon.", core::NAME, core::VERSION));
            s.say("Modules modelled on Meteor Client, by MineGame159 and the Meteor Development contributors.");
            s.say("Pathing modelled on Baritone, by leijurv and the cabaletta contributors.");
            s.say("Uses none of their code; not made or endorsed by them or by Horizon.");
        }
        "gui" | "menu" => s.gui.open = !s.gui.open,
        "streamproof" => match arg(1) {
            Some("dump") => {
                let path = g.native_data_dir(core::ID).join("overlay.bmp");
                crate::game::asteroid_stream::dump(path.clone());
                s.say(format!("The overlay's next picture goes to {}", path.display()));
            }
            _ => s.say(format!("StreamProof: {}", crate::game::asteroid_stream::status())),
        },
        "basefinder" => {
            let eye = g.eye_position().unwrap_or_default();
            crate::game::asteroid_base::report(s, eye);
        }
        "name" => {
            let Some(i) = s.index("NameChanger") else {
                return;
            };
            match arg(1) {
                None => s.say("Usage: .name <name|off>: only you see it"),
                Some("off") => s.set_on(i, false),
                Some(_) => {
                    let name = words[1..].join(" ");
                    if let Some(set) = s.modules[i].sets.iter_mut().find(|x| x.name == "Name") {
                        let _ = set.val.parse(&name);
                    }
                    s.set_on(i, true);
                    s.dirty = true;
                    s.say(format!("You now show as {name}, on your screen only"));
                }
            }
        }
        "toggle" | "t" => {
            if let Some(i) = module_or_say(s, arg(1)) {
                s.toggle(i);
            }
        }
        "bind" => {
            let Some(i) = module_or_say(s, arg(1)) else {
                return;
            };
            let name = s.modules[i].name;
            match arg(2) {
                None => s.say(format!("Usage: .bind {name} <key|none>")),
                Some("none" | "clear") => {
                    s.modules[i].bind.clear();
                    s.say(format!("{name} unbound"));
                }
                Some(k) => {
                    let key = key_name(k);
                    s.say(format!("{name} bound to {}", core::key_label(&key)));
                    s.modules[i].bind = key;
                }
            }
            s.dirty = true;
        }
        "binds" => {
            let binds: Vec<String> = s
                .modules
                .iter()
                .filter(|m| !m.bind.is_empty())
                .map(|m| format!("{}: {}", m.name, core::key_label(&m.bind)))
                .collect();
            if binds.is_empty() {
                s.say("No binds yet: .bind <module> <key>");
            }
            for b in binds {
                s.say(b);
            }
        }
        "profile" => {
            let (Some(op), Some(name)) = (arg(1), arg(2)) else {
                s.say("Usage: .profile save|load <name>. A server's address is its profile too: it loads on joining.");
                return;
            };
            let key = core::profile_key(name);
            match op {
                "save" => match core::save_profile(g, s, &key) {
                    Ok(()) => s.say(format!("Saved the profile {key}")),
                    Err(e) => s.say(format!("Could not save the profile {key}: {e}")),
                },
                "load" => {
                    if core::load_profile(g, s, &key) {
                        s.say(format!("Loaded the profile {key}"));
                    } else {
                        s.say(format!("No profile called {key}"));
                    }
                }
                _ => s.say("Usage: .profile save|load <name>"),
            }
        }
        "preset" => {
            // Ready-made block lists for X-ray and BlockESP: `.preset xray diamonds`.
            let Some(i) = module_or_say(s, arg(1)) else {
                return;
            };
            let mname = s.modules[i].name;
            let presets = [
                ("Xray", "diamonds", "Ores", "minecraft:diamond_ore,minecraft:deepslate_diamond_ore"),
                ("Xray", "debris", "Ores", "minecraft:ancient_debris"),
                ("BlockESP", "storage", "Blocks", "minecraft:chest,minecraft:trapped_chest,minecraft:barrel,minecraft:ender_chest,minecraft:shulker_box"),
                ("BlockESP", "spawners", "Blocks", "minecraft:spawner"),
            ];
            let mine: Vec<_> = presets.iter().filter(|p| p.0.eq_ignore_ascii_case(mname)).collect();
            let Some(name) = arg(2) else {
                let names: Vec<&str> = mine.iter().map(|p| p.1).collect();
                s.say(format!("Presets for {mname}: {}", names.join(", ")));
                return;
            };
            let Some(&&(_, _, set, value)) = mine.iter().find(|p| p.1.eq_ignore_ascii_case(name)) else {
                s.say(format!("No preset {name} for {mname}"));
                return;
            };
            let result = match s.modules[i].sets.iter_mut().find(|x| x.name.eq_ignore_ascii_case(set)) {
                Some(st) => match st.val.parse(value) {
                    Ok(()) => Ok(format!("{mname} {} = preset {name}", st.name)),
                    Err(e) => Err(format!("{mname} {}: {e}", st.name)),
                },
                None => Err(format!("{mname} has no setting {set}")),
            };
            match result {
                Ok(line) => {
                    s.say(line);
                    s.dirty = true;
                }
                Err(line) => s.say(line),
            }
        }
        "set" => {
            let Some(i) = module_or_say(s, arg(1)) else {
                return;
            };
            let (Some(set), true) = (arg(2), words.len() >= 4) else {
                s.say(format!("Usage: .set {} <setting> <value>", s.modules[i].name));
                return;
            };
            let value = words[3..].join(" ");
            let mname = s.modules[i].name;
            let result = match s.modules[i].sets.iter_mut().find(|x| x.name.eq_ignore_ascii_case(set)) {
                Some(st) => match st.val.parse(&value) {
                    Ok(()) => Ok(format!("{mname} {} = {}", st.name, st.val.show())),
                    Err(e) => Err(format!("{mname} {}: {e}", st.name)),
                },
                None => Err(format!("{mname} has no setting {set}")),
            };
            match result {
                Ok(line) => {
                    s.say(line);
                    s.dirty = true;
                }
                Err(line) => s.say(line),
            }
        }
        "settings" | "get" => {
            let Some(i) = module_or_say(s, arg(1)) else {
                return;
            };
            let m = s.modules[i].clone();
            s.say(format!("{} ({}): {}", m.name, if m.on { "ON" } else { "OFF" }, m.desc));
            for st in &m.sets {
                let kind = match &st.val {
                    Val::Bool(_) => "true/false".to_owned(),
                    Val::Num(_, lo, hi, _) => format!("{lo}..{hi}"),
                    Val::Choice(_, o) => o.join("/"),
                    Val::Text(_) => "text".to_owned(),
                };
                s.say(format!("  {} = {}  ({kind})", st.name, st.val.show()));
            }
        }
        "modules" | "list" => {
            for cat in CATS {
                if let Some(want) = arg(1)
                    && !cat.name().eq_ignore_ascii_case(want)
                {
                    continue;
                }
                let names: Vec<String> = s
                    .modules
                    .iter()
                    .filter(|m| m.cat == cat)
                    .map(|m| if m.on { format!("{}*", m.name) } else { m.name.to_owned() })
                    .collect();
                s.say(format!("{}: {}", cat.name(), names.join(", ")));
            }
            s.say("(* = on)");
        }
        "reset" => {
            let Some(i) = module_or_say(s, arg(1)) else {
                return;
            };
            let name = s.modules[i].name;
            if let Some(fresh) = core::defs().into_iter().find(|m| m.name == name) {
                s.modules[i].sets = fresh.sets;
                s.dirty = true;
                s.say(format!("{name} settings reset"));
            }
        }
        "hide" => {
            if let Some(i) = module_or_say(s, arg(1)) {
                s.modules[i].visible = !s.modules[i].visible;
                let shown = if s.modules[i].visible { "shown in" } else { "hidden from" };
                s.say(format!("{} {shown} the module list", s.modules[i].name));
                s.dirty = true;
            }
        }
        "panic" => {
            for i in 0..s.modules.len() {
                if s.modules[i].name != "HUD" {
                    s.set_on(i, false);
                }
            }
            s.say("Every module is off");
        }
        "gui" | "clickgui" => s.gui.open = true,
        "friends" | "friend" | "f" => match (arg(1), arg(2)) {
            (Some("add"), Some(name)) => {
                s.friends.insert(name.to_owned());
                s.dirty = true;
                s.say(format!("Added {name} to friends"));
            }
            (Some("remove" | "del"), Some(name)) => {
                s.friends.retain(|f| !f.eq_ignore_ascii_case(name));
                s.dirty = true;
                s.say(format!("Removed {name} from friends"));
            }
            _ => {
                let list: Vec<String> = s.friends.iter().cloned().collect();
                s.say(if list.is_empty() { "No friends yet: .friends add <name>".to_owned() } else { format!("Friends: {}", list.join(", ")) });
            }
        },
        "waypoint" | "waypoints" | "wp" => {
            let dim = g.dim.id().to_owned();
            match (arg(1), arg(2)) {
                (Some("add"), Some(name)) => {
                    let pos = match (arg(3), arg(4), arg(5)) {
                        (Some(x), Some(y), Some(z)) => {
                            match (x.parse::<f64>(), y.parse::<f64>(), z.parse::<f64>()) {
                                (Ok(x), Ok(y), Ok(z)) => Some([x, y, z]),
                                _ => None,
                            }
                        }
                        _ => g.player.as_ref().map(|p| [p.position.x, p.position.y, p.position.z]),
                    };
                    let Some(pos) = pos else {
                        s.say("Usage: .waypoint add <name> [x y z]");
                        return;
                    };
                    s.waypoints.retain(|w| w.name != name);
                    s.waypoints.push(Waypoint { name: name.to_owned(), pos, dim });
                    s.dirty = true;
                    s.say(format!("Waypoint {name} at {:.0} {:.0} {:.0}", pos[0], pos[1], pos[2]));
                }
                (Some("remove" | "del"), Some(name)) => {
                    s.waypoints.retain(|w| w.name != name);
                    s.dirty = true;
                    s.say(format!("Removed waypoint {name}"));
                }
                _ => {
                    let list: Vec<String> = s
                        .waypoints
                        .iter()
                        .map(|w| format!("{} ({:.0} {:.0} {:.0}, {})", w.name, w.pos[0], w.pos[1], w.pos[2], w.dim.trim_start_matches("minecraft:")))
                        .collect();
                    if list.is_empty() {
                        s.say("No waypoints: .waypoint add <name>");
                    }
                    for l in list {
                        s.say(l);
                    }
                }
            }
        }
        "vclip" | "hclip" => {
            let Some(d) = arg(1).and_then(|v| v.parse::<f64>().ok()) else {
                s.say(format!("Usage: .{cmd} <blocks>"));
                return;
            };
            let yaw = f64::from(g.yaw).to_radians();
            if let Some(p) = g.player.as_mut() {
                if cmd.eq_ignore_ascii_case("vclip") {
                    p.position.y += d;
                } else {
                    p.position.x += -yaw.sin() * d;
                    p.position.z += yaw.cos() * d;
                }
                p.old_position = p.position;
            }
        }
        "say" => {
            let text = words[1..].join(" ");
            chat(g, &text);
        }
        "drop" => g.drop_held(true),
        "fov" => match arg(1).and_then(|v| v.parse::<f32>().ok()) {
            Some(f) => {
                g.settings.fov = f.clamp(30.0, 150.0);
                s.say(format!("FOV {:.0}", g.settings.fov));
            }
            None => s.say(format!("FOV is {:.0}", g.settings.fov)),
        },
        "coords" | "pos" => {
            if let Some(p) = g.player.as_ref() {
                let line = format!("{:.1} {:.1} {:.1} in {}", p.position.x, p.position.y, p.position.z, g.dim.id());
                s.say(line);
            }
        }
        _ => {
            // `.killaura` toggles a module of that name.
            match s.index(cmd) {
                Some(i) => s.toggle(i),
                None => s.say(format!("Unknown command {cmd}: .help lists them")),
            }
        }
    }
}
