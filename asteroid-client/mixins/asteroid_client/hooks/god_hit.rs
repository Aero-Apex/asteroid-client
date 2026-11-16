fn asteroid_god_hit(game: &crate::game::Game<'_>) -> std::ops::ControlFlow<bool> {
    let _ = game;
    if horizon_client::lan::guest::asteroid_flags::god_mode() {
        std::ops::ControlFlow::Break(false)
    } else {
        std::ops::ControlFlow::Continue(())
    }
}
