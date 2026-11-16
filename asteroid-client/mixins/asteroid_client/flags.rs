// What the game's hooks read on the server thread (a single player's integrated
// server runs there), set each tick by the mod on the window's. Lives in the
// library, so both the library's guest code and the game can read it.
use std::sync::atomic::{AtomicBool, Ordering};

static GOD: AtomicBool = AtomicBool::new(false);
static CREATIVE_ALL: AtomicBool = AtomicBool::new(false);
static NO_FALL: AtomicBool = AtomicBool::new(false);
static NO_HUNGER: AtomicBool = AtomicBool::new(false);

/// Sets god mode and the creative override.
pub fn set(god: bool, creative: bool) {
    GOD.store(god, Ordering::Relaxed);
    CREATIVE_ALL.store(creative, Ordering::Relaxed);
}
/// Sets no fall damage and no hunger.
pub fn set_safety(no_fall: bool, no_hunger: bool) {
    NO_FALL.store(no_fall, Ordering::Relaxed);
    NO_HUNGER.store(no_hunger, Ordering::Relaxed);
}
/// Whether the player can't be hurt.
pub fn god_mode() -> bool {
    GOD.load(Ordering::Relaxed)
}
/// Whether the server takes creative item picks from guests in survival.
pub fn creative_all() -> bool {
    CREATIVE_ALL.load(Ordering::Relaxed)
}
/// Whether landing doesn't hurt.
pub fn no_fall() -> bool {
    NO_FALL.load(Ordering::Relaxed)
}
/// Whether the food bar stays full.
pub fn no_hunger() -> bool {
    NO_HUNGER.load(Ordering::Relaxed)
}
