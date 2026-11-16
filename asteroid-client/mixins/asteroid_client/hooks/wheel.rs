fn asteroid_wheel(game: &crate::game::Game<'_>, steps: f32, claimed: &mut bool) {
    crate::game::asteroid_core::on_wheel(game, steps, claimed);
}
