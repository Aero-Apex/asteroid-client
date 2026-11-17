// Asteroid Client: an unofficial, from-scratch Rust client for Horizon, built as
// a native mod. Credits: its modules are modelled on Meteor Client (MineGame159 and
// the Meteor Development contributors), its pathing on Baritone (leijurv and the
// cabaletta contributors); it uses none of their code. This module is the framework:
// the module list and its settings, key binds, the config file, the keyboard
// ClickGUI, the `.` chat commands, and the drawing and math helpers the
// feature modules (asteroid_combat, asteroid_player, asteroid_movement,
// asteroid_render, asteroid_world, asteroid_misc) share.
//
// Not made or endorsed by Meteor Development, Baritone or the Horizon project.
use super::{Game, Screen};
use horizon_client::hud::{HudBatch, rgba};
use horizon_client::lan::guest::asteroid_flags as flags;
use winit::window::CursorGrabMode;
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeSet, HashMap},
    path::PathBuf,
    time::Duration,
};

pub const ID: &str = "asteroid_client";
pub const NAME: &str = "Asteroid Client";
pub const VERSION: &str = "2.1.0";
pub const DESCRIPTION: &str = "An unofficial client mod for Horizon: combat, movement, render, world and HUD modules, and pathing. Right Shift opens the ClickGUI; .help and #help in chat list the commands. Credits: its modules are modelled on Meteor Client by MineGame159 and the Meteor Development contributors, and its pathing on Baritone by leijurv and the cabaletta contributors. It is written from scratch in Rust, uses none of their code, and is not made or endorsed by them or by Horizon.";
/// The ClickGUI's key: its name in Options > Mods, and its default.
pub const GUI_KEY_NAME: &str = "gui";
pub const GUI_KEY: &str = "key.keyboard.right.shift";
pub const PREFIX: char = '.';
/// The prefix of the pathing commands.
pub const PATH_PREFIX: char = '#';

// ---------------------------------------------------------------- settings

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cat {
    Combat,
    Player,
    Movement,
    Render,
    World,
    Misc,
    Baritone,
}
pub const CATS: [Cat; 7] = [
    Cat::Combat,
    Cat::Player,
    Cat::Movement,
    Cat::Render,
    Cat::World,
    Cat::Misc,
    Cat::Baritone,
];
impl Cat {
    pub fn name(self) -> &'static str {
        match self {
            Cat::Combat => "Combat",
            Cat::Player => "Player",
            Cat::Movement => "Movement",
            Cat::Render => "Render",
            Cat::World => "World",
            Cat::Misc => "Misc",
            Cat::Baritone => "Baritone",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Bool(bool),
    /// Value, min, max, step.
    Num(f64, f64, f64, f64),
    /// Index into the options.
    Choice(usize, &'static [&'static str]),
    Text(String),
}
impl Val {
    pub fn show(&self) -> String {
        match self {
            Val::Bool(b) => if *b { "ON" } else { "OFF" }.to_owned(),
            Val::Num(v, _, _, step) => {
                if *step >= 1.0 {
                    format!("{v:.0}")
                } else if *step >= 0.1 {
                    format!("{v:.1}")
                } else {
                    format!("{v:.2}")
                }
            }
            Val::Choice(i, o) => o.get(*i).copied().unwrap_or("?").to_owned(),
            Val::Text(t) => t.clone(),
        }
    }
    /// Steps the value: `dir` is -1 or 1 (bools flip, choices cycle).
    pub fn step(&mut self, dir: i32) {
        match self {
            Val::Bool(b) => *b = !*b,
            Val::Num(v, min, max, step) => {
                let n = (*v + f64::from(dir) * *step).clamp(*min, *max);
                *v = (n / *step).round() * *step;
            }
            Val::Choice(i, o) => {
                let len = o.len().max(1) as i32;
                *i = ((*i as i32 + dir).rem_euclid(len)) as usize;
            }
            Val::Text(_) => {}
        }
    }
    /// Parses `text` as this kind of value.
    pub fn parse(&mut self, text: &str) -> Result<(), String> {
        match self {
            Val::Bool(b) => {
                *b = match text.to_ascii_lowercase().as_str() {
                    "true" | "on" | "yes" | "1" => true,
                    "false" | "off" | "no" | "0" => false,
                    other => return Err(format!("{other} is not true or false")),
                }
            }
            Val::Num(v, min, max, _) => {
                let n: f64 = text.parse().map_err(|_| format!("{text} is not a number"))?;
                *v = n.clamp(*min, *max);
            }
            Val::Choice(i, o) => {
                match o.iter().position(|c| c.eq_ignore_ascii_case(text)) {
                    Some(p) => *i = p,
                    None => return Err(format!("choose one of {}", o.join(", "))),
                }
            }
            Val::Text(t) => *t = text.to_owned(),
        }
        Ok(())
    }
    /// How the config file keeps it.
    pub fn stored(&self) -> String {
        match self {
            Val::Bool(b) => b.to_string(),
            Val::Num(v, ..) => v.to_string(),
            Val::Choice(i, o) => o.get(*i).copied().unwrap_or("").to_owned(),
            Val::Text(t) => t.clone(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Set {
    pub name: &'static str,
    pub val: Val,
}
pub fn b(name: &'static str, v: bool) -> Set {
    Set { name, val: Val::Bool(v) }
}
pub fn n(name: &'static str, v: f64, min: f64, max: f64, step: f64) -> Set {
    Set { name, val: Val::Num(v, min, max, step) }
}
pub fn c(name: &'static str, i: usize, options: &'static [&'static str]) -> Set {
    Set { name, val: Val::Choice(i, options) }
}
pub fn t(name: &'static str, v: &str) -> Set {
    Set { name, val: Val::Text(v.to_owned()) }
}

#[derive(Clone, Debug)]
pub struct Module {
    pub name: &'static str,
    pub cat: Cat,
    pub desc: &'static str,
    pub on: bool,
    /// The key name (`key.keyboard.r`), empty for none.
    pub bind: String,
    /// Shown in the HUD's module list.
    pub visible: bool,
    pub sets: Vec<Set>,
}
fn m(name: &'static str, cat: Cat, desc: &'static str, sets: Vec<Set>) -> Module {
    Module { name, cat, desc, on: false, bind: String::new(), visible: true, sets }
}

/// The theme's colours.
pub const ACCENTS: [(&str, u32); 8] = [
    ("Rose", 0xF0_4F_9A),
    ("Violet", 0x9B_5C_F6),
    ("Blue", 0x4C_8D_F6),
    ("Cyan", 0x2F_C6_D8),
    ("Green", 0x3C_CF_7E),
    ("Amber", 0xF2_A9_3B),
    ("Red", 0xEE_50_50),
    ("Silver", 0xB8_BC_C8),
];
const ACCENT_NAMES: &[&str] = &["Rose", "Violet", "Blue", "Cyan", "Green", "Amber", "Red", "Silver"];

/// What a recording shows whatever the overlay does: these change the game's own
/// picture, your view or what you say in chat.
pub fn stream_leak(name: &str) -> bool {
    matches!(
        name,
        "Fullbright" | "Freecam" | "CustomFOV" | "TimeChanger" | "NoRender" | "AimAssist" | "BowAimbot"
            | "KillAura" | "Rotation" | "EXPThrower" | "AutoMace" | "AutoPearlCatch" | "WindChargeDeflect"
            | "Baritone" | "Scaffold" | "Spam" | "Announcer" | "MessageAura" | "EndermanLook" | "Creative"
    )
}

const TARGETS: [&str; 3] = ["Players", "Hostiles", "Animals"];
const BLOCK_CHOICE: &[&str] = &["Obsidian", "Any solid"];

/// Every module and its settings, in the ClickGUI's order.
pub fn defs() -> Vec<Module> {
    use Cat::*;
    let tg = |p: bool, h: bool, a: bool| -> Vec<Set> {
        vec![b(TARGETS[0], p), b(TARGETS[1], h), b(TARGETS[2], a)]
    };
    let plus = |mut v: Vec<Set>, more: Vec<Set>| -> Vec<Set> {
        v.extend(more);
        v
    };
    vec![
        // Combat
        m("GodMode", Combat, "You can't be hurt by mobs, players, falls, fire or drowning.", vec![]),
        m("KillAura", Combat, "Attacks entities around you.", plus(tg(true, true, false), vec![
            n("Range", 3.5, 1.0, 6.0, 0.1),
            c("Priority", 0, &["Distance", "Health", "Angle"]),
            b("WaitCooldown", true),
            b("ThroughWalls", false),
            b("Rotate", true),
        ])),
        m("TriggerBot", Combat, "Attacks the entity under your crosshair.", plus(tg(true, true, false), vec![
            b("WaitCooldown", true),
        ])),
        m("AimAssist", Combat, "Eases your view onto a target near your crosshair, and stops once you are on it.", plus(tg(true, true, false), vec![
            n("Range", 5.0, 1.0, 10.0, 0.5),
            n("Speed", 6.0, 0.5, 20.0, 0.5),
            n("FOV", 60.0, 5.0, 180.0, 5.0),
            c("Aim", 0, &["Nearest", "Chest", "Head", "Feet"]),
            c("Priority", 0, &["Angle", "Distance", "Health"]),
            b("OnlyVisible", true),
            b("Predict", true),
            b("OnlyWhenAttacking", false),
        ])),
        m("BowAimbot", Combat, "Aims your bow, crossbow or trident at targets.", plus(tg(true, true, false), vec![
            n("Range", 50.0, 5.0, 120.0, 5.0),
        ])),
        m("AutoTotem", Combat, "Keeps a totem of undying in your off hand.", vec![
            c("Mode", 0, &["Always", "Smart"]),
            n("Health", 10.0, 1.0, 20.0, 1.0),
        ]),
        m("Offhand", Combat, "Keeps an item of your choice in your off hand, a totem when low.", vec![
            c("Item", 0, &["Golden Apple", "Shield", "End Crystal", "Totem"]),
            n("TotemHealth", 8.0, 1.0, 20.0, 1.0),
        ]),
        m("AutoArmor", Combat, "Wears the best armor in your inventory.", vec![
            n("Delay", 4.0, 0.0, 20.0, 1.0),
            b("PreferElytra", false),
        ]),
        m("AutoWeapon", Combat, "Switches to your best weapon when attacking.", vec![
            c("Weapon", 0, &["Sword", "Axe", "Any"]),
        ]),
        m("Criticals", Combat, "Makes KillAura and TriggerBot hit critical hits.", vec![
            c("Mode", 0, &["MiniJump", "Jump"]),
        ]),
        m("Velocity", Combat, "Reduces the knockback you take.", vec![
            n("Horizontal", 0.0, 0.0, 100.0, 5.0),
            n("Vertical", 0.0, 0.0, 100.0, 5.0),
        ]),
        m("Hitboxes", Combat, "Makes entity hitboxes bigger for your attacks.", vec![
            n("Expand", 0.4, 0.0, 2.0, 0.1),
        ]),
        m("AutoLog", Combat, "Disconnects when your health gets low.", vec![
            n("Health", 6.0, 1.0, 19.0, 1.0),
            b("OnlyNoTotems", false),
            b("ToggleOff", true),
        ]),
        m("AutoMace", Combat, "Attacks your target with a mace while you fall. A glide with an elytra doesn't count as falling.", plus(tg(true, true, false), vec![
            n("Range", 3.0, 1.0, 6.0, 0.1),
            n("MinFall", 1.5, 0.5, 20.0, 0.5),
            b("AutoAim", true),
            b("SwapBack", true),
        ])),
        m("AttributeSwap", Combat, "Hits with your Breach or Density item for the enchantment's bonus, then swaps straight back.", vec![
            c("Enchant", 0, &["Auto", "Breach", "Density"]),
            b("SwapBack", true),
        ]),
        m("ElytraSwapper", Combat, "Right-click swaps an elytra or chestplate from your hotbar instantly, and never equips a broken elytra.", vec![
            b("EmptyHand", true),
        ]),
        m("AutoPearlCatch", Combat, "Follows your thrown pearl and aims a wind charge to intercept it, even when it goes straight up.", vec![
            n("MaxTicks", 60.0, 10.0, 200.0, 5.0),
            b("SwapBack", true),
        ]),
        m("StunWeb", Combat, "Places a cobweb at the target's feet right after your hit lands, so they don't get knocked away.", vec![]),
        m("WindChargeDeflect", Combat, "Hits enemy wind charges away to stop their pearl catches.", vec![
            n("Range", 3.0, 1.0, 6.0, 0.1),
        ]),
        m("CartPop", Combat, "Places and arms a TNT minecart on players who just popped a totem (activator rail, TNT minecart, redstone).", vec![
            n("Range", 4.5, 1.0, 6.0, 0.5),
        ]),
        m("Surround", Combat, "Places blocks around your feet.", vec![
            c("Block", 0, BLOCK_CHOICE),
            b("Center", true),
            b("DisableOnJump", true),
        ]),
        m("SelfTrap", Combat, "Places blocks around your head and above you.", vec![
            c("Block", 0, BLOCK_CHOICE),
        ]),
        m("AutoTrap", Combat, "Traps the nearest player in blocks.", vec![
            n("Range", 4.5, 1.0, 6.0, 0.5),
            c("Block", 0, BLOCK_CHOICE),
        ]),
        m("HoleFiller", Combat, "Fills safe holes near you.", vec![
            n("Range", 4.0, 1.0, 6.0, 0.5),
            c("Block", 0, BLOCK_CHOICE),
        ]),
        m("CrystalAura", Combat, "Places and explodes end crystals near targets.", vec![
            n("Range", 4.5, 1.0, 6.0, 0.5),
            b("Place", true),
            b("Break", true),
            n("Delay", 2.0, 0.0, 20.0, 1.0),
            b("Players", true),
            b("Hostiles", false),
        ]),
        m("BedAura", Combat, "Places and blows up beds near players in the Nether and End.", vec![
            n("Range", 4.5, 1.0, 6.0, 0.5),
            n("Delay", 10.0, 0.0, 40.0, 1.0),
        ]),
        m("AutoCity", Combat, "Mines the blocks surrounding the nearest player.", vec![
            n("Range", 4.5, 1.0, 6.0, 0.5),
        ]),
        // Player
        m("Creative", Player, "Creative mode without cheats: the creative menu and item picks work here.", vec![]),
        m("Reach", Player, "Reaches further for blocks and entities.", vec![
            n("Blocks", 5.5, 3.0, 7.0, 0.1),
            n("Entities", 4.0, 3.0, 6.0, 0.1),
        ]),
        m("AutoEat", Player, "Eats food when you are hungry.", vec![
            n("Hunger", 16.0, 1.0, 19.0, 1.0),
            n("Health", 0.0, 0.0, 19.0, 1.0),
            b("Gapples", false),
        ]),
        m("AutoGap", Player, "Eats a golden apple when your health is low.", vec![
            n("Health", 10.0, 1.0, 19.0, 1.0),
        ]),
        m("AutoFish", Player, "Reels in fish and casts again.", vec![
            n("RecastDelay", 20.0, 5.0, 100.0, 5.0),
        ]),
        m("AutoRespawn", Player, "Respawns as soon as you die.", vec![]),
        m("AutoTool", Player, "Switches to the best tool for the block you mine.", vec![
            b("SwitchBack", true),
        ]),
        m("AutoReplenish", Player, "Refills your hotbar stacks from your inventory.", vec![
            n("Threshold", 8.0, 1.0, 63.0, 1.0),
        ]),
        m("AutoClicker", Player, "Clicks for you.", vec![
            c("Button", 0, &["Left", "Right"]),
            n("CPS", 10.0, 1.0, 20.0, 1.0),
            b("OnlyWhenHeld", false),
        ]),
        m("ChestSwap", Player, "Swaps your chestplate and elytra, then turns off.", vec![]),
        m("EXPThrower", Player, "Throws bottles o' enchanting at your feet.", vec![
            b("LookDown", true),
        ]),
        m("FastUse", Player, "Removes the delay between item uses.", vec![]),
        m("NoBreakDelay", Player, "Removes the delay between breaking blocks.", vec![]),
        m("SpeedMine", Player, "Mines blocks faster.", vec![
            n("Multiplier", 1.3, 1.0, 3.0, 0.1),
        ]),
        m("MiddleClickFriend", Player, "Middle click a player to add or remove a friend.", vec![]),
        m("ChestSteal", Player, "Takes everything out of the container you open.", vec![]),
        m("NoHunger", Player, "Keeps your food bar full (single player and worlds you host; a server decides your hunger itself).", vec![]),
        m("NameChanger", Player, "Shows another name for you, on your screen only: chat, the tab list, your name tag, the HUD. Nobody else sees it (.name <name>).", vec![
            t("Name", "Asteroid"),
        ]),
        m("AutoDrop", Player, "Drops the items in its list.", vec![
            t("Filter", "minecraft:dirt,minecraft:cobblestone,minecraft:gravel,minecraft:netherrack"),
        ]),
        m("Rotation", Player, "Locks your view.", vec![
            b("LockYaw", true),
            b("SnapYaw", true),
            n("Yaw", 0.0, -180.0, 180.0, 5.0),
            b("LockPitch", false),
            n("Pitch", 0.0, -90.0, 90.0, 5.0),
        ]),
        // Movement
        m("Flight", Movement, "Lets you fly.", vec![
            c("Mode", 0, &["Velocity", "Vanilla"]),
            n("Speed", 1.0, 0.1, 5.0, 0.1),
            b("AntiKick", true),
        ]),
        m("Speed", Movement, "Moves you faster.", vec![
            c("Mode", 0, &["Strafe", "OnGround"]),
            n("Speed", 1.4, 1.0, 4.0, 0.05),
            b("Jump", false),
        ]),
        m("Sprint", Movement, "Sprints for you.", vec![
            b("Omni", false),
        ]),
        m("NoFall", Movement, "Stops fall damage (Packet also fools servers that trust the client).", vec![
            c("Mode", 0, &["Packet", "Reset"]),
        ]),
        m("Step", Movement, "Steps up full blocks.", vec![]),
        m("Spider", Movement, "Climbs walls like a spider.", vec![
            n("Speed", 0.2, 0.1, 1.0, 0.05),
        ]),
        m("Jesus", Movement, "Walk on water and lava as if they were solid.", vec![]),
        m("BHop", Movement, "Jumps whenever you move on the ground.", vec![]),
        m("AutoWalk", Movement, "Walks forward for you.", vec![]),
        m("Sneak", Movement, "Sneaks for you.", vec![]),
        m("SafeWalk", Movement, "Keeps you from walking off edges.", vec![]),
        m("Parkour", Movement, "Jumps at the edge of blocks.", vec![]),
        m("NoSlow", Movement, "Stops items you use from slowing you down.", vec![]),
        m("ElytraFly", Movement, "Better elytra flight.", vec![
            n("Speed", 1.5, 0.5, 5.0, 0.1),
            n("Vertical", 1.0, 0.1, 3.0, 0.1),
        ]),
        m("HighJump", Movement, "Jumps higher.", vec![
            n("Multiplier", 2.0, 1.0, 5.0, 0.1),
        ]),
        m("LongJump", Movement, "Jumps further.", vec![
            n("Boost", 2.0, 1.0, 5.0, 0.1),
        ]),
        m("AirJump", Movement, "Jumps in mid-air.", vec![]),
        m("AntiLevitation", Movement, "Stops levitation from lifting you.", vec![]),
        m("FastClimb", Movement, "Climbs ladders and vines faster.", vec![
            n("Speed", 0.3, 0.1, 1.0, 0.05),
        ]),
        m("Anchor", Movement, "Pulls you into holes you walk over.", vec![]),
        m("ReverseStep", Movement, "Steps down blocks instantly.", vec![]),
        m("BoatFly", Movement, "Flies and speeds up the boat you ride.", vec![
            n("Speed", 1.0, 0.1, 5.0, 0.1),
            n("Vertical", 0.5, 0.1, 2.0, 0.1),
        ]),
        m("GUIMove", Movement, "Lets you walk with your inventory open.", vec![]),
        m("Blink", Movement, "Holds your movement back from the server until turned off.", vec![]),
        m("Glide", Movement, "Falls slowly, like a parachute.", vec![
            n("Speed", 0.12, 0.02, 0.5, 0.01),
        ]),
        m("Jetpack", Movement, "Hold jump to rise, sneak to drop, and hover otherwise.", vec![
            n("Speed", 0.6, 0.1, 2.0, 0.05),
        ]),
        m("AutoSwim", Movement, "Keeps you at the surface of water.", vec![
            n("Speed", 0.3, 0.05, 1.0, 0.05),
        ]),
        m("ExtraElytra", Movement, "Adds speed while you glide with elytra.", vec![
            n("Boost", 1.3, 1.0, 3.0, 0.05),
        ]),
        m("SpeedHack", Movement, "Multiplies your horizontal speed on the ground.", vec![
            n("Multiplier", 1.5, 1.0, 3.0, 0.05),
        ]),
        m("NoWeb", Movement, "Cobwebs don't slow you down.", vec![]),
        // Aliases: each turns on the module it names, and off with it.
        m("AutoSprint", Movement, "Sprints for you (turns on Sprint).", vec![]),
        m("BunnyHop", Movement, "Jumps whenever you move on the ground (turns on BHop).", vec![]),
        m("FastLadder", Movement, "Climbs ladders and vines faster (turns on FastClimb).", vec![]),
        m("NoSlowdown", Movement, "Items you use don't slow you (turns on NoSlow).", vec![]),
        m("NoLevitation", Movement, "Stops levitation lifting you (turns on AntiLevitation).", vec![]),
        m("InvWalk", Movement, "Walks with your inventory open (turns on GUIMove).", vec![]),
        m("AntiKnockback", Combat, "Takes less knockback (turns on Velocity).", vec![]),
        m("FastBreak", World, "Breaks blocks with no delay between them (turns on NoBreakDelay).", vec![]),
        m("HealthTags", Render, "Shows health on name tags (turns on NameTags).", vec![]),
        m("ChestEsp", Render, "Highlights chests and containers (turns on StorageESP).", vec![]),
        m("CreativeFlight", Movement, "Lets you fly the way creative players do, double jump to toggle.", vec![
            n("Speed", 1.0, 0.1, 5.0, 0.1),
        ]),
        m("VClip", Movement, "Moves you up or down through blocks once, then turns off.", vec![
            n("Distance", -3.0, -20.0, 20.0, 1.0),
        ]),
        m("Dolphin", Movement, "Swims faster through water.", vec![
            n("Speed", 0.35, 0.1, 1.0, 0.05),
        ]),
        m("Fish", Movement, "Sinks through water when you aren't holding jump.", vec![
            n("Speed", 0.1, 0.02, 0.5, 0.01),
        ]),
        m("NoClip", Movement, "Moves through blocks, as a spectator does (single player: sets spectator mode while on).", vec![]),
        m("ScaffoldWalk", Movement, "Places blocks under you as you walk (turns on Scaffold).", vec![]),
        m("NoFog", Render, "Takes the fog out of the world.", vec![]),
        m("NoBackground", Render, "Takes the sky and clouds away.", vec![]),
        m("CameraDistance", Render, "Sets how far the third-person camera sits from you.", vec![
            n("Distance", 4.0, 0.0, 64.0, 1.0),
        ]),
        m("TrueSight", Render, "Outlines every entity, through walls.", vec![]),
        m("Radar", Render, "Lists the nearest players and how far away they are.", vec![
            n("Range", 64.0, 8.0, 256.0, 8.0),
        ]),
        m("MobEsp", Render, "Outlines mobs, through walls.", vec![]),
        m("PlayerEsp", Render, "Outlines players, through walls.", vec![]),
        m("ItemEsp", Render, "Outlines dropped items, through walls.", vec![]),
        m("BarrierEsp", Render, "Highlights barrier blocks.", vec![]),
        m("PortalEsp", Render, "Highlights nether and end portals.", vec![]),
        m("OpenWaterEsp", Render, "Highlights water.", vec![]),
        m("AntiSpam", Misc, "Drops a chat line that repeats within a few seconds.", vec![
            n("Seconds", 5.0, 1.0, 60.0, 1.0),
        ]),
        m("Excavator", World, "Tunnels straight ahead through blocks (Baritone's #tunnel).", vec![]),
        m("TreeBot", World, "Chops the trees it finds (Baritone's #mine *_log).", vec![]),
        m("AutoFarm", World, "Harvests and replants the crops around it (Baritone's #farm).", vec![]),
        m("TillAura", World, "Tills the dirt in reach with a hoe.", vec![
            n("Range", 4.0, 1.0, 4.5, 0.5),
        ]),
        m("BonemealAura", World, "Bone-meals the crops and saplings in reach.", vec![
            n("Range", 4.0, 1.0, 4.5, 0.5),
        ]),
        m("Scaffold", Movement, "Places blocks under you as you walk.", vec![
            b("Tower", true),
        ]),
        // Render
        m("HUD", Render, "The HUD: watermark, module list, info lines.", vec![
            b("Watermark", true),
            b("ArrayList", true),
            b("Coords", true),
            b("FPS", true),
            b("Speed", true),
            b("Direction", true),
            b("Time", true),
            b("Armor", true),
            b("Potions", true),
            b("Totems", true),
            b("Target", true),
            b("Welcome", true),
            b("Rainbow", false),
            b("Pills", true),
            b("Resources", true),
            b("Keybinds", true),
        ]),
        m("ESP", Render, "Draws boxes around entities, with their health and distance.", plus(tg(true, true, true), vec![
            b("Items", true),
            c("Style", 0, &["Corners", "Outline", "3D"]),
            b("Fill", false),
            b("HealthBar", true),
            b("Names", false),
            b("Distance", false),
            c("Color", 0, &["Type", "Health", "Theme"]),
            b("Fade", true),
            n("MaxRange", 128.0, 16.0, 256.0, 8.0),
        ])),
        m("Tracers", Render, "Draws lines to entities.", plus(tg(true, true, false), vec![
            b("Items", false),
        ])),
        m("Nametags", Render, "Shows names, health and distance above entities.", vec![
            b("Players", true),
            b("Mobs", false),
            b("Items", false),
            b("Health", true),
            b("Distance", true),
        ]),
        m("StorageESP", Render, "Highlights chests and other containers.", vec![
            b("Tracers", false),
        ]),
        m("BlockESP", Render, "Highlights the blocks you list.", vec![
            t("Blocks", "minecraft:diamond_ore,minecraft:deepslate_diamond_ore,minecraft:ancient_debris,minecraft:spawner"),
            b("Tracers", false),
        ]),
        m("Xray", Render, "Shows ores through walls.", vec![
            t("Ores", "minecraft:diamond_ore,minecraft:deepslate_diamond_ore,minecraft:emerald_ore,minecraft:deepslate_emerald_ore,minecraft:gold_ore,minecraft:deepslate_gold_ore,minecraft:nether_gold_ore,minecraft:iron_ore,minecraft:deepslate_iron_ore,minecraft:ancient_debris,minecraft:redstone_ore,minecraft:deepslate_redstone_ore,minecraft:lapis_ore,minecraft:deepslate_lapis_ore"),
        ]),
        m("HoleESP", Render, "Highlights safe 1x1 holes.", vec![
            n("Range", 8.0, 3.0, 16.0, 1.0),
        ]),
        m("Fullbright", Render, "Lights up the whole world.", vec![]),
        m("Freecam", Render, "Moves your camera freely while your body stays.", vec![
            n("Speed", 1.0, 0.1, 5.0, 0.1),
        ]),
        m("NoRender", Render, "Stops some things from drawing.", vec![
            b("Weather", true),
            b("ViewBobbing", true),
        ]),
        m("NoWeather", Render, "Clears rain and thunder.", vec![]),
        m("NoHurtcam", Render, "Stops the camera shaking when you are hurt.", vec![]),
        m("Zoom", Render, "Narrows your view while it is on.", vec![
            n("Factor", 4.0, 1.5, 10.0, 0.5),
        ]),
        m("AntiBlind", Render, "Hides the blindness and darkness effects.", vec![]),
        m("CustomFOV", Render, "Sets your field of view.", vec![
            n("FOV", 100.0, 30.0, 150.0, 1.0),
        ]),
        m("TimeChanger", Render, "Changes the time of day you see.", vec![
            n("Time", 6000.0, 0.0, 24000.0, 500.0),
        ]),
        m("Breadcrumbs", Render, "Leaves a trail behind you.", vec![
            n("MaxPoints", 500.0, 50.0, 2000.0, 50.0),
        ]),
        m("Trajectories", Render, "Shows where projectiles will land.", vec![]),
        m("Predictions", Render, "Shows the full flight path of pearls, tridents, snowballs, arrows, potions, wind charges and more, with time to land, who threw it and potion effects.", vec![
            b("Held", true),
            b("Labels", true),
            n("MaxTicks", 200.0, 20.0, 400.0, 10.0),
        ]),
        m("BaseESP", Render, "Outlines the chunks BaseFinder suspects, the blocks that gave them away and why.", vec![
            n("MinScore", 10.0, 1.0, 100.0, 1.0),
            b("Evidence", true),
            b("Labels", true),
        ]),
        m("BaseFinderHUD", Render, "Lists the most suspicious chunks with an arrow pointing to each.", vec![
            n("Rows", 5.0, 1.0, 10.0, 1.0),
        ]),
        m("LogoutSpots", Render, "Marks where players logged out.", vec![]),
        m("BlockSelection", Render, "Outlines the block you look at.", vec![]),
        m("BreakIndicators", Render, "Shows how far blocks being broken are.", vec![]),
        m("Waypoints", Render, "Draws your waypoints (.waypoint add).", vec![
            b("Distance", true),
        ]),
        // World
        m("Nuker", World, "Breaks blocks around you.", vec![
            n("Range", 4.0, 1.0, 6.0, 0.5),
            c("Mode", 0, &["All", "Flatten", "Filter"]),
            t("Filter", "minecraft:stone,minecraft:dirt,minecraft:netherrack"),
        ]),
        m("AutoMine", World, "Holds the attack button down for you.", vec![]),
        m("Timer", World, "Changes the tick speed of the game.", vec![
            n("Speed", 1.5, 0.1, 10.0, 0.1),
        ]),
        m("LiquidFiller", World, "Places blocks in fluid sources around you.", vec![
            n("Range", 4.0, 1.0, 6.0, 0.5),
        ]),
        m("AutoBreed", World, "Feeds animals near you with the item in hand.", vec![
            n("Range", 4.0, 1.0, 6.0, 0.5),
        ]),
        m("AutoShearer", World, "Shears sheep near you.", vec![
            n("Range", 4.0, 1.0, 6.0, 0.5),
        ]),
        m("EndermanLook", World, "Looks away from endermen.", vec![]),
        m("StashFinder", World, "Reports chunks full of containers.", vec![
            n("Minimum", 5.0, 1.0, 50.0, 1.0),
        ]),
        m("BaseFinder", World, "Finds player bases by scoring chunks for placed blocks, storage, spawners, dug shafts and pets, and says why. The world seed improves deepslate detection.", vec![
            n("MinScore", 10.0, 1.0, 100.0, 1.0),
            n("Radius", 8.0, 2.0, 32.0, 1.0),
            b("Alerts", true),
            t("Seed", ""),
        ]),
        // Misc
        m("StreamProof", Misc, "Hides the menu, HUD and overlays from OBS, Discord and other recording software; you still see them. Modules that can't be hidden show a warning icon.", vec![]),
        m("Theme", Misc, "The panel's and the HUD's colours (see the Theme page).", vec![
            c("Accent", 0, ACCENT_NAMES),
            n("Opacity", 94.0, 40.0, 100.0, 1.0),
            c("HudStyle", 0, &["Modern", "Classic"]),
        ]),
        m("AntiAFK", Misc, "Moves now and then so you aren't kicked.", vec![
            b("Jump", true),
            b("Spin", true),
            b("Swing", true),
            n("Interval", 5.0, 1.0, 60.0, 1.0),
        ]),
        m("Panic", Misc, "Turns every module off at once.", vec![]),
        m("FastPlace", World, "Removes the delay between placing blocks.", vec![]),
        m("Spam", Misc, "Sends a chat message again and again.", vec![
            t("Message", "Asteroid Client on top!"),
            n("Delay", 10.0, 1.0, 300.0, 1.0),
        ]),
        m("Announcer", Misc, "Announces how far you walked.", vec![
            n("Delay", 60.0, 10.0, 600.0, 10.0),
        ]),
        m("HealthAlert", Misc, "Says so in chat when your health drops below a level.", vec![
            n("Below", 6.0, 1.0, 19.0, 1.0),
        ]),
        m("Notifier", Misc, "Tells you when players come in or leave render distance.", vec![
            b("Enter", true),
            b("Leave", true),
        ]),
        m("MessageAura", Misc, "Messages players who come near you.", vec![
            t("Message", "Hello from Asteroid Client"),
        ]),
        m("AutoReconnect", Misc, "Reconnects to the server you were kicked from.", vec![
            n("Delay", 5.0, 1.0, 60.0, 1.0),
        ]),
        // Baritone
        m("Baritone", Baritone, "Walks, mines and builds its way there: #help lists the commands. Turning it off stops it.", vec![
            b("AllowBreak", true),
            b("AllowPlace", true),
            b("AllowDownward", true),
            b("AllowSprint", true),
            b("PauseNearPlayer", true),
            b("AllowParkour", false),
            b("AutoTool", true),
            b("RenderPath", true),
            b("Survival", true),
            b("DebugLog", false),
            n("MaxFall", 3.0, 1.0, 20.0, 1.0),
            n("PlaceCost", 20.0, 0.0, 100.0, 1.0),
            n("BreakCost", 1.0, 0.1, 10.0, 0.1),
            n("FollowRange", 3.0, 1.0, 16.0, 1.0),
            t("Throwaway", "minecraft:cobblestone,minecraft:dirt,minecraft:netherrack,minecraft:cobbled_deepslate,minecraft:stone"),
        ]),
    ]
}

// ------------------------------------------------------------------- state

#[derive(Clone, Debug)]
pub struct Waypoint {
    pub name: String,
    pub pos: [f64; 3],
    pub dim: String,
}

/// Which of the panel's text boxes has the keyboard.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Focus {
    #[default]
    None,
    Search,
    Set(usize, usize),
    Config,
    Friend,
}

#[derive(Default)]
pub struct Gui {
    pub open: bool,
    /// The category or general page shown.
    pub page: usize,
    pub scroll: f32,
    pub expanded: Option<usize>,
    pub search: String,
    pub focus: Focus,
    pub buffer: String,
    pub shift: bool,
    /// Waiting for a key to bind to this module.
    pub binding: Option<usize>,
    /// The slider held: module, setting, track x and width.
    pub drag: Option<(usize, usize, f32, f32)>,
    /// The panel being moved, and where it has been moved to.
    pub moving: Option<(f32, f32)>,
    pub offset: (f32, f32),
    pub config_name: String,
    pub friend_name: String,
    pub configs: Vec<String>,
    pub configs_at: usize,
    /// Whether the GUI had the pointer free last frame.
    pub applied: bool,
    /// The view the GUI holds still while it is open (yaw, pitch).
    pub view: Option<(f32, f32)>,
    /// Escape closed the GUI this frame: the game's own Escape would pause, so
    /// the frame undoes that.
    pub escape: bool,
}

pub struct State {
    pub modules: Vec<Module>,
    pub loaded: bool,
    pub dirty: bool,
    pub path: Option<PathBuf>,
    pub friends: BTreeSet<String>,
    pub waypoints: Vec<Waypoint>,
    pub gui: Gui,
    /// Turned on (true) or off since the last tick.
    pub events: Vec<(usize, bool)>,
    pub ticks: u64,
    /// The last server joined (address, name), for AutoReconnect.
    pub last_server: Option<(String, String)>,
    /// The server just joined, whose profile loads once the player is in.
    pub server_profile: Option<String>,
    /// The current target of a combat module, for the HUD.
    pub target: Option<u32>,
    /// Speed in blocks per second, for the HUD.
    pub speed: f64,
    pub last_pos: Option<[f64; 3]>,
    /// Chat lines to show once the borrow ends.
    pub out: Vec<String>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            modules: defs(),
            loaded: false,
            dirty: false,
            path: None,
            friends: BTreeSet::new(),
            waypoints: Vec::new(),
            gui: Gui::default(),
            events: Vec::new(),
            ticks: 0,
            last_server: None,
            server_profile: None,
            target: None,
            speed: 0.0,
            last_pos: None,
            out: Vec::new(),
        }
    }
}

impl State {
    pub fn index(&self, name: &str) -> Option<usize> {
        self.modules.iter().position(|m| m.name.eq_ignore_ascii_case(name))
    }
    pub fn module(&self, name: &str) -> Option<&Module> {
        self.modules.iter().find(|m| m.name == name)
    }
    pub fn on(&self, name: &str) -> bool {
        self.module(name).is_some_and(|m| m.on)
    }
    fn val(&self, module: &str, set: &str) -> Option<&Val> {
        self.module(module)?.sets.iter().find(|s| s.name == set).map(|s| &s.val)
    }
    pub fn b(&self, module: &str, set: &str) -> bool {
        matches!(self.val(module, set), Some(Val::Bool(true)))
    }
    pub fn n(&self, module: &str, set: &str) -> f64 {
        match self.val(module, set) {
            Some(Val::Num(v, ..)) => *v,
            _ => 0.0,
        }
    }
    pub fn c(&self, module: &str, set: &str) -> &'static str {
        match self.val(module, set) {
            Some(Val::Choice(i, o)) => o.get(*i).copied().unwrap_or(""),
            _ => "",
        }
    }
    pub fn t(&self, module: &str, set: &str) -> &str {
        match self.val(module, set) {
            Some(Val::Text(s)) => s.as_str(),
            _ => "",
        }
    }
    /// A comma separated text setting as a list of namespaced ids.
    pub fn list(&self, module: &str, set: &str) -> Vec<String> {
        self.t(module, set)
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(namespaced)
            .collect()
    }
    pub fn set_on(&mut self, i: usize, on: bool) {
        if self.modules[i].on != on {
            self.modules[i].on = on;
            self.events.push((i, on));
            self.dirty = true;
        }
    }
    pub fn toggle(&mut self, i: usize) {
        let on = !self.modules[i].on;
        self.set_on(i, on);
    }
    pub fn turn_off(&mut self, name: &str) {
        if let Some(i) = self.index(name) {
            self.set_on(i, false);
        }
    }
    pub fn say(&mut self, text: impl Into<String>) {
        self.out.push(text.into());
    }
    pub fn is_friend(&self, name: &str) -> bool {
        self.friends.iter().any(|f| f.eq_ignore_ascii_case(name))
    }
    /// The modules of a category, in order.
    pub fn in_cat(&self, cat: Cat) -> Vec<usize> {
        (0..self.modules.len()).filter(|&i| self.modules[i].cat == cat).collect()
    }
}

pub fn namespaced(id: &str) -> String {
    if id.contains(':') { id.to_owned() } else { format!("minecraft:{id}") }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    // What the game's own functions read while a hook may hold STATE.
    pub static REACH_BLOCKS: Cell<f64> = const { Cell::new(0.0) };
    pub static REACH_ENTITIES: Cell<f64> = const { Cell::new(0.0) };
    pub static HITBOX: Cell<f64> = const { Cell::new(0.0) };
    pub static BLINK: Cell<bool> = const { Cell::new(false) };
    pub static NOFALL_PACKET: Cell<bool> = const { Cell::new(false) };
    /// Flight tells the server it is on the ground while it goes down.
    pub static FLIGHT_SAFE: Cell<bool> = const { Cell::new(false) };
    /// Jesus holds the player on a fluid's surface this tick.
    pub static JESUS_SURFACE: Cell<bool> = const { Cell::new(false) };
    /// NameChanger: what the tab list was last given.
    pub static TAB_NAME: RefCell<String> = const { RefCell::new(String::new()) };
    /// The view and hotbar slot to put back once this tick's clicks went out.
    pub static RESTORE: Cell<Option<(f32, f32, usize)>> = const { Cell::new(None) };
    pub static NO_BREAK_DELAY: Cell<bool> = const { Cell::new(false) };
    pub static SPEED_MINE: Cell<f64> = const { Cell::new(1.0) };
    pub static ACTIVE: Cell<bool> = const { Cell::new(false) };
    /// HealthAlert has said so for the current drop below its level.
    pub static LOW_ALERTED: Cell<bool> = const { Cell::new(false) };
    /// Where the HUD's left column ends, for what goes under it.
    pub static LEFT_Y: Cell<f32> = const { Cell::new(14.0) };
    /// One of the ClickGUI's text boxes has the keyboard.
    pub static TYPING: Cell<bool> = const { Cell::new(false) };
    /// What the mod would have said in chat while StreamProof is on.
    static TOASTS: RefCell<std::collections::VecDeque<(String, std::time::Instant)>> = const { RefCell::new(std::collections::VecDeque::new()) };
}

/// Runs `f` with the state, unless a hook further up already holds it.
pub fn with<R>(f: impl FnOnce(&mut State) -> R) -> Option<R> {
    STATE.with(|s| s.try_borrow_mut().ok().map(|mut s| f(&mut s)))
}

// ------------------------------------------------------------ entry points

fn active(g: &Game<'_>) -> bool {
    g.native_window() && g.native_on(ID)
}

/// Shows what the state queued for chat.
fn flush(g: &mut Game<'_>, lines: Vec<String>) {
    let hidden = crate::game::asteroid_stream::recording();
    for line in lines {
        if hidden {
            // The game's chat would be recorded: StreamProof shows it on the overlay.
            TOASTS.with(|t| {
                let mut t = t.borrow_mut();
                t.push_back((line, std::time::Instant::now()));
                while t.len() > 6 {
                    t.pop_front();
                }
            });
        } else {
            g.native_chat(&format!("[Asteroid] {line}"));
        }
    }
}

fn load(g: &Game<'_>, s: &mut State) {
    s.loaded = true;
    let dir = g.native_data_dir(ID);
    let path = dir.join("config.txt");
    s.path = Some(path.clone());
    // The settings from when the mod was called Meteor Horizon come along.
    let text = std::fs::read_to_string(&path)
        .or_else(|_| std::fs::read_to_string(g.native_data_dir("meteor_horizon").join("config.txt")));
    let Ok(text) = text else {
        // Defaults: the HUD on, the GUI on Right Shift.
        if let Some(i) = s.index("HUD") {
            s.modules[i].on = true;
        }
        s.dirty = true;
        return;
    };
    for line in text.lines() {
        let mut w = line.splitn(4, ' ');
        let (Some(kind), Some(a)) = (w.next(), w.next()) else {
            continue;
        };
        let rest: Vec<&str> = w.collect();
        match kind {
            "on" => {
                if let Some(i) = s.index(a) {
                    s.modules[i].on = rest.first() == Some(&"true");
                }
            }
            "bind" => {
                if let Some(i) = s.index(a) {
                    s.modules[i].bind = rest.first().copied().unwrap_or("").to_owned();
                }
            }
            "visible" => {
                if let Some(i) = s.index(a) {
                    s.modules[i].visible = rest.first() != Some(&"false");
                }
            }
            "set" => {
                let (Some(i), Some(set)) = (s.index(a), rest.first()) else {
                    continue;
                };
                let value = rest.get(1).copied().unwrap_or("");
                if let Some(st) = s.modules[i].sets.iter_mut().find(|x| x.name == *set) {
                    let _ = st.val.parse(value);
                }
            }
            "friend" => {
                s.friends.insert(a.to_owned());
            }
            "waypoint" => {
                // waypoint NAME x y z dim → rest = [x, "y z dim"]
                let all: Vec<&str> = line.split(' ').collect();
                if all.len() >= 6 {
                    let p = |i: usize| all[i].parse::<f64>().unwrap_or(0.0);
                    s.waypoints.push(Waypoint {
                        name: all[1].to_owned(),
                        pos: [p(2), p(3), p(4)],
                        dim: all[5].to_owned(),
                    });
                }
            }
            _ => {}
        }
    }
    // Modules that were on start again.
    for i in 0..s.modules.len() {
        if s.modules[i].on {
            s.events.push((i, true));
        }
    }
}

/// A profile's file name for a name or a server address.
pub fn profile_key(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).collect()
}

fn profile_file(g: &Game<'_>, key: &str) -> std::path::PathBuf {
    g.native_data_dir(ID).join("profiles").join(format!("{key}.txt"))
}

/// Writes the current settings as the profile `key`; the config stays as it was.
pub fn save_profile(g: &Game<'_>, s: &mut State, key: &str) -> std::io::Result<()> {
    let path = profile_file(g, key);
    std::fs::create_dir_all(path.parent().unwrap_or(&path))?;
    let live = s.path.replace(path);
    save(s);
    s.path = live;
    s.dirty = true;
    Ok(())
}

/// Replaces the current settings with the profile `key`. False if there is none.
pub fn load_profile(g: &Game<'_>, s: &mut State, key: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(profile_file(g, key)) else {
        return false;
    };
    if std::fs::write(g.native_data_dir(ID).join("config.txt"), text).is_err() {
        return false;
    }
    load(g, s);
    s.dirty = true;
    true
}

fn save(s: &mut State) {
    s.dirty = false;
    let Some(path) = s.path.clone() else {
        return;
    };
    let mut out = String::new();
    for m in &s.modules {
        out.push_str(&format!("on {} {}\n", m.name, m.on));
        if !m.bind.is_empty() {
            out.push_str(&format!("bind {} {}\n", m.name, m.bind));
        }
        if !m.visible {
            out.push_str(&format!("visible {} false\n", m.name));
        }
        for st in &m.sets {
            out.push_str(&format!("set {} {} {}\n", m.name, st.name, st.val.stored()));
        }
    }
    for f in &s.friends {
        out.push_str(&format!("friend {f}\n"));
    }
    for w in &s.waypoints {
        out.push_str(&format!(
            "waypoint {} {} {} {} {}\n",
            w.name, w.pos[0], w.pos[1], w.pos[2], w.dim
        ));
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, out);
}

/// Keeps the values the game's hooked functions read in step with the settings.
fn publish(s: &State) {
    ACTIVE.with(|c| c.set(true));
    REACH_BLOCKS.with(|c| c.set(if s.on("Reach") { s.n("Reach", "Blocks") } else { 0.0 }));
    // KillAura attacks through the crosshair, so its range is an entity reach too.
    let mut entities = if s.on("Reach") { s.n("Reach", "Entities") } else { 0.0 };
    if s.on("KillAura") {
        entities = entities.max(s.n("KillAura", "Range"));
    }
    REACH_ENTITIES.with(|c| c.set(entities));
    HITBOX.with(|c| c.set(if s.on("Hitboxes") { s.n("Hitboxes", "Expand") } else { 0.0 }));
    BLINK.with(|c| c.set(s.on("Blink")));
    NOFALL_PACKET.with(|c| c.set(s.on("NoFall") && s.c("NoFall", "Mode") == "Packet"));
    FLIGHT_SAFE.with(|c| c.set(s.on("Flight")));
    NO_BREAK_DELAY.with(|c| c.set(s.on("NoBreakDelay")));
    SPEED_MINE.with(|c| {
        c.set(if s.on("SpeedMine") { s.n("SpeedMine", "Multiplier") } else { 1.0 })
    });
}

/// `native_tick`: once a game tick.
pub fn on_tick(g: &mut Game<'_>) {
    g.forced_hit = None;
    if !active(g) {
        ACTIVE.with(|c| c.set(false));
        // The server thread also ticks here (`native_window` is false), and
        // must not clear the flags the window set.
        if g.native_window() {
            flags::set(false, false);
            flags::set_safety(false, false);
        }
        return;
    }
    let lines = with(|s| {
        if !s.loaded {
            load(g, s);
        }
        s.ticks += 1;
        // A profile saved for this server loads once the player is in.
        if g.player.is_some()
            && let Some(address) = s.server_profile.take()
        {
            let key = profile_key(&address);
            if load_profile(g, s, &key) {
                s.say(format!("Loaded the profile {key} for this server"));
            }
        }
        // HealthAlert: once each time health drops below its level.
        if let Some(p) = g.player.as_ref() {
            let health = p.health.health();
            let low = s.on("HealthAlert") && f64::from(health) < s.n("HealthAlert", "Below");
            if low && !LOW_ALERTED.with(|c| c.get()) {
                LOW_ALERTED.with(|c| c.set(true));
                s.say(format!("Low health: {:.1} hearts", health / 2.0));
            } else if !low {
                LOW_ALERTED.with(|c| c.set(false));
            }
        }
        // Speed for the HUD.
        if let Some(p) = g.player.as_ref() {
            let now = [p.position.x, p.position.y, p.position.z];
            if let Some(last) = s.last_pos {
                let d = ((now[0] - last[0]).powi(2) + (now[2] - last[2]).powi(2)).sqrt();
                s.speed = s.speed * 0.7 + d * 20.0 * 0.3;
            }
            s.last_pos = Some(now);
        }
        let events = std::mem::take(&mut s.events);
        flags::set(s.on("GodMode"), s.on("Creative"));
        flags::set_safety(s.on("NoFall") || s.on("Flight"), s.on("NoHunger"));
        for (i, on) in events {
            let name = s.modules[i].name;
            if let Some(target) = alias_target(name)
                && let Some(t) = s.index(target)
                && s.modules[t].on != on
            {
                s.set_on(t, on);
            }
            if name == "Creative" {
                use horizon_client::command::args::GameMode;
                g.set_mode(if on { GameMode::Creative } else { GameMode::Survival });
            }
            crate::game::asteroid_combat::toggled(g, s, name, on);
            crate::game::asteroid_player::toggled(g, s, name, on);
            crate::game::asteroid_movement::toggled(g, s, name, on);
            crate::game::asteroid_render::toggled(g, s, name, on);
            crate::game::asteroid_world::toggled(g, s, name, on);
            crate::game::asteroid_misc::toggled(g, s, name, on);
            crate::game::asteroid_baritone::toggled(g, s, name, on);
            crate::game::asteroid_extra::toggled(g, s, name, on);
            if s.on("HUD") || name != "HUD" {
                s.say(format!("{name} {}", if on { "ON" } else { "OFF" }));
            }
        }
        publish(s);
        publish_name(g, s);
        if g.player.is_some() {
            crate::game::asteroid_base::tick(g, s);
            if !crate::game::asteroid_pvp::tick(g, s) {
                crate::game::asteroid_combat::tick(g, s);
            }
            crate::game::asteroid_player::tick(g, s);
            crate::game::asteroid_movement::tick(g, s);
            crate::game::asteroid_baritone::tick(g, s);
            crate::game::asteroid_extra::tick(g, s);
            crate::game::asteroid_render::tick(g, s);
            crate::game::asteroid_world::tick(g, s);
        }
        crate::game::asteroid_misc::tick(g, s);
        if s.dirty && s.ticks.is_multiple_of(10) {
            save(s);
        }
        std::mem::take(&mut s.out)
    });
    if let Some(lines) = lines {
        flush(g, lines);
    }
}

/// `native_frame`: once a rendered frame.
pub fn on_frame(g: &mut Game<'_>, elapsed: Duration) {
    if !active(g) {
        return;
    }
    if let Some((yaw, pitch, slot)) = RESTORE.with(|c| c.take()) {
        g.yaw = yaw;
        g.pitch = pitch;
        g.selected = slot;
    }
    let lines = with(|s| {
        if !s.loaded {
            return Vec::new();
        }
        let scale = g.renderer.as_ref().map_or(1.0, |r| HudBatch::new([r.config.width, r.config.height]).scale);
        crate::game::asteroid_stream::frame(g.window.as_deref(), s.on("StreamProof"), scale);
        TYPING.with(|c| c.set(s.gui.open && s.gui.focus != Focus::None));
        if s.gui.open != s.gui.applied {
            s.gui.applied = s.gui.open;
            if s.gui.open {
                s.gui.view = Some((g.yaw, g.pitch));
            } else {
                s.gui.view = None;
                // The mod grabs the pointer itself, so the game's flag is set back.
                g.captured = true;
            }
            set_pointer(g, !s.gui.open);
        }
        if std::mem::take(&mut s.gui.escape) && g.screen == Screen::Paused {
            g.screen = Screen::Playing;
            g.pause_menu = false;
        }
        // While Baritone runs the view is its to aim: the saved view follows it.
        if s.gui.open && crate::game::asteroid_baritone::running() {
            s.gui.view = Some((g.yaw, g.pitch));
        }
        if let Some((yaw, pitch)) = s.gui.view {
            g.yaw = yaw;
            g.pitch = pitch;
        }
        if g.player.is_some() {
            crate::game::asteroid_combat::frame(g, s, elapsed);
            crate::game::asteroid_player::frame(g, s, elapsed);
            crate::game::asteroid_movement::frame(g, s, elapsed);
            crate::game::asteroid_render::frame(g, s, elapsed);
        }
        crate::game::asteroid_misc::frame(g, s);
        std::mem::take(&mut s.out)
    });
    if let Some(lines) = lines {
        flush(g, lines);
    }
}

/// `native_hud`: the overlays, the HUD and the ClickGUI.
pub fn on_hud(g: &mut Game<'_>, batch: &mut HudBatch, corners: &mut horizon_client::extras_hud::Corners) {
    if !active(g) {
        return;
    }
    with(|s| {
        if !s.loaded {
            return;
        }
        if g.player.is_some() {
            crate::game::asteroid_render::hud(g, s, batch, corners);
            crate::game::asteroid_baritone::hud(g, s, batch);
            crate::game::asteroid_extra::hud(g, s, batch);
            crate::game::asteroid_base::hud(g, s, batch);
        }
    });
}

/// The last of the HUD (the hotbar, the hearts and the chat are drawn by now):
/// the ClickGUI goes over all of them.
pub fn on_late_hud(g: &mut Game<'_>, batch: &mut HudBatch) {
    if !active(g) {
        return;
    }
    with(|s| {
        if s.loaded && s.gui.open {
            crate::game::asteroid_gui::draw(g, s, batch);
        }
        if s.loaded {
            draw_toasts(g, s, batch);
        }
    });
}

/// The ClickGUI is open: the mouse moves the pointer, not the view.
pub fn gui_open() -> bool {
    with(|s| s.gui.open).unwrap_or(false)
}

/// Keeps NameChanger's names up to date.
fn publish_name(g: &mut Game<'_>, s: &State) {
    // The session's name on a server; single player and LAN have none, so the name the game plays under.
    let real = g.remote.as_ref().map(|r| r.profile_name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| g.options.player_name());
    let shown = s.t("NameChanger", "Name").trim().to_owned();
    let names = (s.on("NameChanger") && !shown.is_empty() && !real.is_empty()).then(|| (real, shown));
    horizon_client::chat::asteroid_names::set(names);
    crate::game::remote::asteroid_tab::rename(g);
}

/// The name NameChanger shows, when it is on.
pub fn shown_name() -> Option<String> {
    horizon_client::chat::asteroid_names::shown()
}

/// The player's own tab list entry: what its name should be, given what it is
/// and the session's real name. None for everyone else's.
pub fn tab_name(current: &str, real: &str) -> Option<String> {
    let shown = shown_name().unwrap_or_else(|| real.to_owned());
    TAB_NAME.with(|last| {
        let mut last = last.borrow_mut();
        (current == real || current == last.as_str()).then(|| {
            last.clone_from(&shown);
            shown
        })
    })
}

/// The module an alias turns on and off with it.
fn alias_target(name: &str) -> Option<&'static str> {
    Some(match name {
        "AutoSprint" => "Sprint",
        "BunnyHop" => "BHop",
        "FastLadder" => "FastClimb",
        "NoSlowdown" => "NoSlow",
        "NoLevitation" => "AntiLevitation",
        "InvWalk" => "GUIMove",
        "AntiKnockback" => "Velocity",
        "FastBreak" => "NoBreakDelay",
        "HealthTags" => "Nametags",
        "ChestEsp" => "StorageESP",
        "ScaffoldWalk" => "Scaffold",
        _ => return None,
    })
}

/// `native_key`: binds and the ClickGUI.
pub fn on_key(g: &mut Game<'_>, key: &str, pressed: bool) {
    if !active(g) {
        return;
    }
    let lines = with(|s| {
        if !s.loaded {
            load(g, s);
        }
        if pressed && s.gui.open && key == "key.keyboard.escape" {
            s.gui.escape = true;
        }
        // Mouse buttons are never typing: they go to the panel's clicks below, even
        // while a text box has the focus (which would otherwise swallow them).
        let mouse = key == "key.mouse.left" || key == "key.mouse.right" || key == "key.mouse.middle";
        if !mouse && s.gui.open && crate::game::asteroid_gui::key(g, s, key, pressed) {
            return std::mem::take(&mut s.out);
        }
        let gui_key = g.native_mod_key(ID, GUI_KEY_NAME, GUI_KEY);
        if pressed && key == gui_key && (g.native_playing() || s.gui.open) {
            s.gui.open = !s.gui.open;
            s.gui.binding = None;
            s.gui.drag = None;
            s.gui.moving = None;
            return std::mem::take(&mut s.out);
        }
        if s.gui.open && (key == "key.mouse.left" || key == "key.mouse.right") {
            if pressed {
                crate::game::asteroid_gui::click(g, s, key == "key.mouse.right");
            } else {
                crate::game::asteroid_gui::release(s);
            }
            return std::mem::take(&mut s.out);
        }
        if pressed && !s.gui.open && (key == "key.mouse.left" || key == "key.mouse.right") {
            crate::game::asteroid_pvp::on_mouse(g, s, key == "key.mouse.right");
        }
        if key == "key.mouse.middle" && pressed {
            crate::game::asteroid_player::middle_click(g, s);
        }
        if pressed && g.native_playing() {
            for i in 0..s.modules.len() {
                if !s.modules[i].bind.is_empty() && s.modules[i].bind == key {
                    s.toggle(i);
                }
            }
        }
        std::mem::take(&mut s.out)
    });
    if let Some(lines) = lines {
        flush(g, lines);
    }
}

/// The wheel: the panel scrolls, and the hotbar keeps still.
pub fn on_wheel(g: &Game<'_>, steps: f32, claimed: &mut bool) {
    if !active(g) {
        return;
    }
    with(|s| {
        if s.gui.open {
            crate::game::asteroid_gui::wheel(s, steps);
            *claimed = true;
        }
    });
}

/// A chat line typed by the player: true when it was a `.` command.
pub fn on_chat(g: &mut Game<'_>, message: &str) -> bool {
    if !active(g) {
        return false;
    }
    if let Some(line) = message.strip_prefix(PATH_PREFIX) {
        // `##` sends a line starting with one #.
        if line.starts_with(PATH_PREFIX) || line.is_empty() {
            return false;
        }
        let lines = with(|s| {
            if !s.loaded {
                load(g, s);
            }
            crate::game::asteroid_baritone::command(g, s, line.trim());
            std::mem::take(&mut s.out)
        });
        if let Some(lines) = lines {
            flush(g, lines);
        }
        return true;
    }
    let Some(line) = message.strip_prefix(PREFIX) else {
        return false;
    };
    // `..` sends a line starting with one dot.
    if line.starts_with(PREFIX) {
        return false;
    }
    let lines = with(|s| {
        if !s.loaded {
            load(g, s);
        }
        crate::game::asteroid_misc::command(g, s, line.trim());
        std::mem::take(&mut s.out)
    });
    if let Some(lines) = lines {
        flush(g, lines);
    }
    true
}

/// `/meteor ...`, the same commands as `.`.
pub fn on_command(g: &mut Game<'_>, command: &str) -> bool {
    let Some(rest) = command.strip_prefix("meteor") else {
        return false;
    };
    if !rest.is_empty() && !rest.starts_with(' ') {
        return false;
    }
    let line = format!("{PREFIX}{}", if rest.trim().is_empty() { "help" } else { rest.trim() });
    on_chat(g, &line)
}

/// A server the player joins, for AutoReconnect.
pub fn on_connect(address: &str, name: &str) {
    with(|s| {
        s.last_server = Some((address.to_owned(), name.to_owned()));
        s.server_profile = Some(address.to_owned());
    });
}

/// Options > Mods' page for the mod.
pub fn settings_labels() -> Vec<String> {
    with(|s| {
        let on = s.modules.iter().filter(|m| m.on).count();
        vec![
            format!("Modules on: {on} (Right Shift: ClickGUI)"),
            "Turn every module off (panic)".to_owned(),
            "Reset every setting".to_owned(),
        ]
    })
    .unwrap_or_default()
}
pub fn settings_click(button: usize) {
    with(|s| match button {
        1 => {
            for i in 0..s.modules.len() {
                s.set_on(i, false);
            }
        }
        2 => {
            let friends = std::mem::take(&mut s.friends);
            let waypoints = std::mem::take(&mut s.waypoints);
            for i in 0..s.modules.len() {
                s.set_on(i, false);
            }
            s.modules = defs();
            s.friends = friends;
            s.waypoints = waypoints;
            s.dirty = true;
        }
        _ => {}
    });
}

// ----------------------------------------------------------------- the GUI

/// Frees the pointer for the GUI, or grabs it back for play. The game keeps
/// `captured` set meanwhile, so its next click doesn't grab the pointer again
/// and swallow the click the GUI needs.
fn set_pointer(g: &Game<'_>, grab: bool) {
    let Some(w) = g.window.as_ref() else {
        return;
    };
    if grab {
        let _ = w.set_cursor_grab(CursorGrabMode::Locked).or_else(|_| w.set_cursor_grab(CursorGrabMode::Confined));
        w.set_cursor_visible(false);
    } else {
        let _ = w.set_cursor_grab(CursorGrabMode::None);
        w.set_cursor_visible(true);
    }
}

/// A key name as people read it: `key.keyboard.right.shift` → `RIGHT SHIFT`.
pub fn key_label(key: &str) -> String {
    key.trim_start_matches("key.keyboard.")
        .trim_start_matches("key.")
        .replace('.', " ")
        .to_uppercase()
}

/// The theme's accent, `0xRRGGBB`.
pub fn accent(s: &State) -> u32 {
    let name = s.c("Theme", "Accent");
    ACCENTS.iter().find(|(n, _)| *n == name).map_or(ACCENTS[0].1, |(_, c)| *c)
}

/// The saved configs' names.
pub fn profiles(g: &Game<'_>) -> Vec<String> {
    let dir = g.native_data_dir(ID).join("profiles");
    let mut out: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".txt").map(str::to_owned))
        .collect();
    out.sort();
    out
}

pub fn delete_profile(g: &Game<'_>, key: &str) {
    let _ = std::fs::remove_file(profile_file(g, key));
}

/// What StreamProof kept out of chat.
fn draw_toasts(g: &Game<'_>, s: &State, batch: &mut HudBatch) {
    if !s.on("StreamProof") {
        return;
    }
    TOASTS.with(|t| {
        let mut t = t.borrow_mut();
        t.retain(|(_, at)| at.elapsed().as_secs() < 6);
        let k = 0.75;
        let mut y = batch.height - 60.0;
        for (line, _) in t.iter().rev() {
            let w = g.native_text_width(line) * k;
            fill(g, batch, 3.0, y - 1.0, w + 6.0, 8.5, 0xC0_10_0A_14);
            fill(g, batch, 3.0, y - 1.0, 1.0, 8.5, 0xFF00_0000 | accent(s));
            text_k(g, batch, line, 6.0, y, 0xFF_FF_FF, k);
            y -= 9.5;
        }
    });
}

pub fn clip_text(g: &Game<'_>, text: &str, max: f32) -> String {
    if g.native_text_width(text) <= max {
        return text.to_owned();
    }
    let mut out = String::new();
    for ch in text.chars() {
        out.push(ch);
        if g.native_text_width(&out) > max - 6.0 {
            out.pop();
            out.push_str("..");
            break;
        }
    }
    out
}

// ---------------------------------------------------------------- drawing

pub fn fill(g: &Game<'_>, batch: &mut HudBatch, x: f32, y: f32, w: f32, h: f32, argb: u32) {
    if crate::game::asteroid_stream::recording() {
        crate::game::asteroid_stream::push(crate::game::asteroid_stream::Prim::Fill([x, y, w, h], argb));
        return;
    }
    g.native_fill(batch, [x, y, w, h], argb);
}
pub fn text(g: &Game<'_>, batch: &mut HudBatch, s: &str, x: f32, y: f32, rgb: u32) {
    text_k(g, batch, s, x, y, rgb, 1.0);
}
/// Text at `k` times the game's size.
pub fn text_k(g: &Game<'_>, batch: &mut HudBatch, s: &str, x: f32, y: f32, rgb: u32, k: f32) {
    if crate::game::asteroid_stream::recording() {
        // Each character's advance, so what was laid out with the game's font still fits.
        let mut buf = [0u8; 4];
        let adv: Vec<f32> = s.chars().map(|c| g.native_text_width(c.encode_utf8(&mut buf)) * k).collect();
        crate::game::asteroid_stream::push(crate::game::asteroid_stream::Prim::Text(s.to_owned(), x, y, rgb & 0xFF_FF_FF, k, adv));
        return;
    }
    let start = batch.vertices.len();
    g.native_text(batch, s, [x, y], rgb, true);
    if k != 1.0 {
        batch.scale_since(start, x, y, k, k);
    }
}
/// A filled four-cornered shape.
pub fn quad(g: &Game<'_>, batch: &mut HudBatch, q: [(f32, f32); 4], argb: u32) {
    let corners = q.map(|p| [p.0, p.1]);
    if crate::game::asteroid_stream::recording() {
        crate::game::asteroid_stream::push(crate::game::asteroid_stream::Prim::Quad(corners, argb));
        return;
    }
    let [al, r, gr, bl] = argb.to_be_bytes();
    batch.fill_quad(&g.gui.atlas, corners, rgba(r, gr, bl, al));
}
/// A line `w` GUI pixels wide.
pub fn line(g: &Game<'_>, batch: &mut HudBatch, a: (f32, f32), b: (f32, f32), w: f32, argb: u32) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.01 || !len.is_finite() {
        return;
    }
    let (nx, ny) = (-dy / len * w / 2.0, dx / len * w / 2.0);
    quad(
        g,
        batch,
        [
            (a.0 + nx, a.1 + ny),
            (b.0 + nx, b.1 + ny),
            (b.0 - nx, b.1 - ny),
            (a.0 - nx, a.1 - ny),
        ],
        argb,
    );
}
pub fn rect_outline(g: &Game<'_>, batch: &mut HudBatch, x: f32, y: f32, w: f32, h: f32, argb: u32) {
    fill(g, batch, x, y, w, 1.0, argb);
    fill(g, batch, x, y + h - 1.0, w, 1.0, argb);
    fill(g, batch, x, y, 1.0, h, argb);
    fill(g, batch, x + w - 1.0, y, 1.0, h, argb);
}
/// A colour around the hue wheel, `0xRRGGBB`.
pub fn rainbow(offset: f64, ticks: u64) -> u32 {
    let h = ((ticks as f64 * 0.01 + offset).fract() + 1.0).fract() * 6.0;
    let x = 1.0 - (h % 2.0 - 1.0).abs();
    let (r, gr, bl) = match h as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    let c = |v: f64| (v * 200.0 + 55.0) as u32;
    (c(r) << 16) | (c(gr) << 8) | c(bl)
}

/// The camera of the last frame, to project world points on the HUD.
pub struct Cam {
    pos: [f64; 3],
    f: [f64; 3],
    r: [f64; 3],
    u: [f64; 3],
    scale: f64,
    cx: f64,
    cy: f64,
}
pub fn camera(g: &Game<'_>, batch: &HudBatch) -> Option<Cam> {
    let (pos, yaw, pitch, fov) = g.locator_camera?;
    let (y, p) = (f64::from(yaw).to_radians(), f64::from(pitch).to_radians());
    let f = [-y.sin() * p.cos(), -p.sin(), y.cos() * p.cos()];
    let r = [-y.cos(), 0.0, -y.sin()];
    // up = r × f
    let u = [
        r[1] * f[2] - r[2] * f[1],
        r[2] * f[0] - r[0] * f[2],
        r[0] * f[1] - r[1] * f[0],
    ];
    let h = f64::from(batch.height);
    let fov = f64::from(fov).clamp(1.0, 179.0).to_radians();
    Some(Cam {
        pos,
        f,
        r,
        u,
        scale: (h / 2.0) / (fov / 2.0).tan(),
        cx: f64::from(batch.width) / 2.0,
        cy: h / 2.0,
    })
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
impl Cam {
    fn view(&self, p: [f64; 3]) -> [f64; 3] {
        let d = [p[0] - self.pos[0], p[1] - self.pos[1], p[2] - self.pos[2]];
        [dot(d, self.r), dot(d, self.u), dot(d, self.f)]
    }
    fn screen(&self, v: [f64; 3]) -> (f32, f32) {
        (
            (self.cx + v[0] / v[2] * self.scale) as f32,
            (self.cy - v[1] / v[2] * self.scale) as f32,
        )
    }
    /// A world point on the screen, none behind the camera.
    pub fn project(&self, p: [f64; 3]) -> Option<(f32, f32)> {
        let v = self.view(p);
        (v[2] > 0.05).then(|| self.screen(v))
    }
    /// A segment cut at the near plane, on the screen.
    pub fn segment(&self, a: [f64; 3], b: [f64; 3]) -> Option<((f32, f32), (f32, f32))> {
        let (mut va, mut vb) = (self.view(a), self.view(b));
        const NEAR: f64 = 0.05;
        if va[2] < NEAR && vb[2] < NEAR {
            return None;
        }
        let cut = |p: [f64; 3], q: [f64; 3]| {
            let t = (NEAR - p[2]) / (q[2] - p[2]);
            [p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t, NEAR]
        };
        if va[2] < NEAR {
            va = cut(va, vb);
        } else if vb[2] < NEAR {
            vb = cut(vb, va);
        }
        Some((self.screen(va), self.screen(vb)))
    }
    /// The screen rectangle of a world box, none when part is behind.
    pub fn rect(&self, min: [f64; 3], max: [f64; 3]) -> Option<[f32; 4]> {
        let mut lo = (f32::MAX, f32::MAX);
        let mut hi = (f32::MIN, f32::MIN);
        for i in 0..8 {
            let p = [
                if i & 1 == 0 { min[0] } else { max[0] },
                if i & 2 == 0 { min[1] } else { max[1] },
                if i & 4 == 0 { min[2] } else { max[2] },
            ];
            let (x, y) = self.project(p)?;
            lo = (lo.0.min(x), lo.1.min(y));
            hi = (hi.0.max(x), hi.1.max(y));
        }
        Some([lo.0, lo.1, hi.0 - lo.0, hi.1 - lo.1])
    }
    /// The twelve edges of a world box.
    pub fn box_edges(&self, g: &Game<'_>, batch: &mut HudBatch, min: [f64; 3], max: [f64; 3], argb: u32) {
        let c = |i: usize| -> [f64; 3] {
            [
                if i & 1 == 0 { min[0] } else { max[0] },
                if i & 2 == 0 { min[1] } else { max[1] },
                if i & 4 == 0 { min[2] } else { max[2] },
            ]
        };
        const EDGES: [(usize, usize); 12] = [
            (0, 1), (2, 3), (4, 5), (6, 7),
            (0, 2), (1, 3), (4, 6), (5, 7),
            (0, 4), (1, 5), (2, 6), (3, 7),
        ];
        for (a, b) in EDGES {
            if let Some((p, q)) = self.segment(c(a), c(b)) {
                line(g, batch, p, q, 1.0, argb);
            }
        }
    }
    pub fn center(&self) -> (f32, f32) {
        (self.cx as f32, self.cy as f32)
    }
}

// ------------------------------------------------------------- game helpers

/// What a combat module may aim at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Player,
    Hostile,
    Animal,
}
/// An entity a module may act on.
#[derive(Clone, Debug)]
pub struct Target {
    pub index: usize,
    pub id: u32,
    pub kind: Kind,
    pub name: String,
    pub center: [f64; 3],
    pub min: [f64; 3],
    pub max: [f64; 3],
    pub health: f32,
    /// From the eye to the nearest point of its box.
    pub distance: f64,
}

pub fn player_names(g: &Game<'_>) -> HashMap<u32, String> {
    g.remote_player_names().into_iter().collect()
}

/// The living entities around the player, nearest first.
pub fn targets(g: &Game<'_>, s: &State, range: f64, players: bool, hostiles: bool, animals: bool) -> Vec<Target> {
    let Some(eye) = g.eye_position() else {
        return Vec::new();
    };
    let names = player_names(g);
    let mut out = Vec::new();
    for (index, mob) in g.mobs.iter().enumerate() {
        if !mob.alive() || Some(mob.id) == g.riding {
            continue;
        }
        let kind = if mob.kind.id.ends_with("player") {
            Kind::Player
        } else if mob.kind.category == "monster" {
            Kind::Hostile
        } else {
            Kind::Animal
        };
        let wanted = match kind {
            Kind::Player => players,
            Kind::Hostile => hostiles,
            Kind::Animal => animals,
        };
        if !wanted {
            continue;
        }
        let name = match kind {
            Kind::Player => names.get(&mob.id).cloned().unwrap_or_else(|| "Player".to_owned()),
            _ => pretty(&mob.kind.id),
        };
        if kind == Kind::Player && s.is_friend(&name) {
            continue;
        }
        let half = mob.kind.width / 2.0;
        let h = f64::from(mob.height());
        let p = mob.position;
        let min = [p.x - half, p.y, p.z - half];
        let max = [p.x + half, p.y + h, p.z + half];
        let near = [0, 1, 2].map(|a| eye[a].clamp(min[a], max[a]));
        let distance = dist(eye, near);
        if distance > range {
            continue;
        }
        out.push(Target {
            index,
            id: mob.id,
            kind,
            name,
            center: [p.x, p.y + h / 2.0, p.z],
            min,
            max,
            health: mob.health,
            distance,
        });
    }
    out.sort_by(|a, b| a.distance.total_cmp(&b.distance));
    out
}

/// `minecraft:zombie_villager` → `Zombie Villager`.
pub fn pretty(id: &str) -> String {
    id.rsplit(':')
        .next()
        .unwrap_or(id)
        .split('_')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// The yaw and pitch that look from `eye` at `at`.
pub fn angles(eye: [f64; 3], at: [f64; 3]) -> (f32, f32) {
    let d = [at[0] - eye[0], at[1] - eye[1], at[2] - eye[2]];
    let h = (d[0] * d[0] + d[2] * d[2]).sqrt();
    let yaw = (-d[0]).atan2(d[2]).to_degrees();
    let pitch = (-d[1]).atan2(h).to_degrees();
    (yaw as f32, pitch.clamp(-90.0, 90.0) as f32)
}
/// Turns the view to `at`, keeping the yaw continuous.
pub fn look_at(g: &mut Game<'_>, at: [f64; 3]) {
    let Some(eye) = g.eye_position() else {
        return;
    };
    let (yaw, pitch) = angles(eye, at);
    let delta = wrap(yaw - g.yaw);
    g.yaw += delta;
    g.pitch = pitch;
}
/// An angle in -180..180.
pub fn wrap(a: f32) -> f32 {
    (a + 180.0).rem_euclid(360.0) - 180.0
}

/// The movement keys held: (forward, left) in -1..=1, jump, sneak.
pub fn input(g: &mut Game<'_>) -> (f64, f64, bool, bool) {
    fn held(g: &mut Game<'_>, m: &str) -> bool {
        g.native_mapping(m).is_some_and(|(d, _)| d)
    }
    let f = f64::from(i8::from(held(g, "key.forward")) - i8::from(held(g, "key.back")));
    let l = f64::from(i8::from(held(g, "key.left")) - i8::from(held(g, "key.right")));
    (f, l, held(g, "key.jump"), held(g, "key.sneak"))
}
/// A horizontal direction for (forward, left) at the view's yaw, of length 1.
pub fn direction(yaw: f32, forward: f64, left: f64) -> Option<(f64, f64)> {
    if forward == 0.0 && left == 0.0 {
        return None;
    }
    let y = f64::from(yaw).to_radians();
    let fx = -y.sin();
    let fz = y.cos();
    let (lx, lz) = (y.cos(), y.sin());
    let (x, z) = (fx * forward + lx * left, fz * forward + lz * left);
    let len = (x * x + z * z).sqrt();
    Some((x / len, z / len))
}

pub fn block_name(g: &Game<'_>, pos: [i32; 3]) -> std::sync::Arc<str> {
    let state = g.world.block(pos[0], pos[1], pos[2]);
    g.res.facts().get(state).name.clone()
}
pub fn is_air(g: &Game<'_>, pos: [i32; 3]) -> bool {
    let state = g.world.block(pos[0], pos[1], pos[2]);
    g.res.facts().is_air(state)
}
/// A block a new block can go into: air, grass, fluids.
pub fn replaceable(g: &Game<'_>, pos: [i32; 3]) -> bool {
    let state = g.world.block(pos[0], pos[1], pos[2]);
    let f = g.res.facts();
    f.is_air(state) || f.get(state).replaceable
}
/// A block to place against or stand on.
pub fn solid(g: &Game<'_>, pos: [i32; 3]) -> bool {
    let state = g.world.block(pos[0], pos[1], pos[2]);
    let f = g.res.facts();
    !f.is_air(state) && !f.get(state).replaceable && !f.get(state).outline.is_empty()
}
pub fn feet(g: &Game<'_>) -> Option<[i32; 3]> {
    let p = g.player.as_ref()?;
    Some([
        p.position.x.floor() as i32,
        (p.position.y + 0.2).floor() as i32,
        p.position.z.floor() as i32,
    ])
}

/// The hotbar slot holding an item `want` accepts.
pub fn hotbar_find(g: &Game<'_>, want: impl Fn(&str) -> bool) -> Option<usize> {
    (0..9).find(|&i| g.inventory[i].as_ref().is_some_and(|s| want(&s.item)))
}
/// Any inventory slot (0..36) holding an item `want` accepts, hotbar last.
pub fn inventory_find(g: &Game<'_>, want: impl Fn(&str) -> bool) -> Option<usize> {
    (9..36)
        .chain(0..9)
        .find(|&i| g.inventory.get(i).and_then(|s| s.as_ref()).is_some_and(|s| want(&s.item)))
}
pub fn count(g: &Game<'_>, want: impl Fn(&str) -> bool) -> u32 {
    g.inventory
        .iter()
        .chain(std::iter::once(&g.offhand))
        .flatten()
        .filter(|s| want(&s.item))
        .map(|s| s.count)
        .sum()
}
/// A block item the placing modules may use.
pub fn placeable(g: &Game<'_>, item: &str, choice: &str) -> bool {
    if choice == "Obsidian" {
        return item == "minecraft:obsidian" || item == "minecraft:crying_obsidian";
    }
    let Some(info) = g.res.items().get(item) else {
        return false;
    };
    let Some(block) = info.block else {
        return false;
    };
    let f = g.res.facts().get(block);
    f.full_collision && !f.block_entity && f.hardness > 0.0 && !f.random_ticks
}

/// No container but the player's own inventory is open (a click there is safe).
pub fn inventory_menu(g: &Game<'_>) -> bool {
    match g.remote.as_ref() {
        Some(_) => g.remote_menu().0 == 0 && g.carried.is_none(),
        None => g.container.is_none() && g.station.is_none() && g.carried.is_none(),
    }
}

/// Swaps inventory slot `slot` (0..36) with hotbar slot `hotbar` (0..9), or
/// with the off hand when `hotbar` is 40.
pub fn swap_slot(g: &mut Game<'_>, slot: usize, hotbar: usize) {
    if g.remote.is_some() {
        g.remote_menu_ops(&[(Some(slot), hotbar as i32, horizon_client::menu::Input::Swap)]);
        return;
    }
    if hotbar == 40 {
        let item = g.inventory[slot].take();
        g.inventory[slot] = g.offhand.take();
        g.offhand = item;
    } else if slot != hotbar {
        g.inventory.swap(slot, hotbar);
    }
}
/// Shift-clicks a slot of the player's inventory menu (0..36, 500..504).
pub fn quick_move(g: &mut Game<'_>, slot: usize) -> bool {
    if g.remote.is_some() {
        return g.remote_menu_ops(&[(Some(slot), 0, horizon_client::menu::Input::QuickMove)]);
    }
    false
}

/// Throws the whole stack in inventory slot `slot` out.
pub fn throw_stack(g: &mut Game<'_>, slot: usize) -> bool {
    let ops = [(Some(slot), 1, horizon_client::menu::Input::Throw)];
    if g.remote.is_some() { g.remote_menu_ops(&ops) } else { g.menu_ops(&ops) }
}

/// Puts the view and the hotbar slot back once this tick's clicks went out,
/// keeping what was there before the first change of the tick.
pub fn restore_after_tick(yaw: f32, pitch: f32, slot: usize) {
    RESTORE.with(|c| {
        if c.get().is_none() {
            c.set(Some((yaw, pitch, slot)));
        }
    });
}

/// A left or right click as the mouse makes it.
pub fn click(g: &mut Game<'_>, right: bool) {
    let button = if right {
        winit::event::MouseButton::Right
    } else {
        winit::event::MouseButton::Left
    };
    let _ = g.play_mouse(button, true);
    let _ = g.play_mouse(button, false);
}
/// Holds or releases a mouse button.
pub fn hold(g: &mut Game<'_>, right: bool, down: bool) {
    let button = if right {
        winit::event::MouseButton::Right
    } else {
        winit::event::MouseButton::Left
    };
    let held = if right { g.right } else { g.left };
    if held != down {
        let _ = g.play_mouse(button, down);
    }
}
/// The attack has charged (no weaker hit for attacking now).
pub fn charged(g: &Game<'_>) -> bool {
    g.hand_strength.scale(0.5, g.main_hand_attack_speed()) >= 0.92
}
pub fn playing(g: &Game<'_>) -> bool {
    g.native_playing() && g.screen == Screen::Playing
}

/// Looks at a face of the block beside `pos` and right-clicks it with the
/// item in hotbar slot `slot`, to place a block at `pos`. False when no face
/// that would put the block there is in reach from here.
pub fn place(g: &mut Game<'_>, pos: [i32; 3], slot: usize) -> bool {
    if !replaceable(g, pos) {
        return false;
    }
    let Some(eye) = g.eye_position() else {
        return false;
    };
    const DIRS: [[i32; 3]; 6] = [[0, -1, 0], [0, 1, 0], [1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1]];
    let reach = g.reach();
    let mut best: Option<(super::Hit, f64)> = None;
    for d in DIRS {
        let support = [pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]];
        if !solid(g, support) {
            continue;
        }
        // The middle of the face of `support` that touches `pos`.
        let face = [-d[0], -d[1], -d[2]];
        let point = [0, 1, 2].map(|a| f64::from(support[a]) + 0.5 + f64::from(face[a]) * 0.5);
        let dd = dist(eye, point);
        if dd <= reach && best.as_ref().is_none_or(|x| dd < x.1) {
            best = Some((super::Hit { pos: support, face, point }, dd));
        }
    }
    let Some((hit, _)) = best else {
        return false;
    };
    let (yaw, pitch) = angles(eye, hit.point);
    g.selected = slot;
    g.yaw += wrap(yaw - g.yaw);
    g.pitch = pitch;
    // The click acts after this tick's physics, where a ray from the moved eye
    // can miss: it names the face instead, as a guest's packet does.
    g.forced_hit = Some(hit);
    click(g, true);
    true
}

/// Right-clicks a face of the block at `pos` with hotbar slot `slot`.
pub fn use_on(g: &mut Game<'_>, pos: [i32; 3], face: [i32; 3], slot: usize) -> bool {
    let Some(eye) = g.eye_position() else {
        return false;
    };
    let point = [0, 1, 2].map(|a| f64::from(pos[a]) + 0.5 + f64::from(face[a]) * 0.5);
    if dist(eye, point) > g.reach() {
        return false;
    }
    let (yaw, pitch) = angles(eye, point);
    g.selected = slot;
    g.yaw += wrap(yaw - g.yaw);
    g.pitch = pitch;
    g.forced_hit = Some(super::Hit { pos, face, point });
    click(g, true);
    true
}
