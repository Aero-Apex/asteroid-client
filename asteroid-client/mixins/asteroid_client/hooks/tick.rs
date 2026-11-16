fn asteroid_tick(game: &mut crate::game::Game<'_>) {
    // The chat-open spoof comes first, so Baritone's own tick below sees a playing screen.
    crate::game::asteroid_baritone::spoof_begin(game);
    // Before the game's own check for losing focus: Baritone keeps the pause menu from opening.
    crate::game::asteroid_baritone::pause_guard(game);
    crate::game::asteroid_core::on_tick(game);
}
