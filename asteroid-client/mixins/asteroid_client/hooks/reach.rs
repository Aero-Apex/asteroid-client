fn asteroid_reach(game: &crate::game::Game<'_>) -> std::ops::ControlFlow<f64> {
    let reach = crate::game::asteroid_core::REACH_BLOCKS.with(|c| c.get());
    if reach > 0.0 && crate::game::asteroid_core::ACTIVE.with(|c| c.get()) && game.native_on(crate::game::asteroid_core::ID) {
        std::ops::ControlFlow::Break(reach)
    } else {
        std::ops::ControlFlow::Continue(())
    }
}
