fn asteroid_late_hud(game: &mut crate::game::Game<'_>, batch: &mut horizon_client::hud::HudBatch) {
    crate::game::asteroid_core::on_late_hud(game, batch);
}
