fn asteroid_frame(game: &mut crate::game::Game<'_>, elapsed: std::time::Duration) {
    crate::game::asteroid_core::on_frame(game, elapsed);
}
