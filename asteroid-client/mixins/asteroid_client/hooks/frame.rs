fn asteroid_frame(game: &mut crate::game::Game<'_>, elapsed: std::time::Duration) {
    // The chat comes back before anything is drawn.
    crate::game::asteroid_baritone::spoof_end(game);
    crate::game::asteroid_core::on_frame(game, elapsed);
}
