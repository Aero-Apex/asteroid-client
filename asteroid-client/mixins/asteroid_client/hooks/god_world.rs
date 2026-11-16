fn asteroid_god_world(game: &crate::game::Game<'_>) -> std::ops::ControlFlow<()> {
    let _ = game;
    if horizon_client::lan::guest::asteroid_flags::god_mode() {
        std::ops::ControlFlow::Break(())
    } else {
        std::ops::ControlFlow::Continue(())
    }
}
