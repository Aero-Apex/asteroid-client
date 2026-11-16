fn asteroid_key(game: &mut crate::game::Game<'_>, key: &str, pressed: bool) {
    crate::game::asteroid_core::on_key(game, key, pressed);
}
