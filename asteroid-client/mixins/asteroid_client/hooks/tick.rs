fn asteroid_tick(game: &mut crate::game::Game<'_>) {
    // Before the game's own check for losing focus: Baritone keeps the pause menu from opening.
    crate::game::asteroid_baritone::pause_guard(game);
    crate::game::asteroid_core::on_tick(game);
}
