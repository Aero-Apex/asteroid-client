fn asteroid_send_state(game: &mut crate::game::Game<'_>) -> std::ops::ControlFlow<()> {
    if crate::game::asteroid_movement::send_state(game) {
        std::ops::ControlFlow::Break(())
    } else {
        std::ops::ControlFlow::Continue(())
    }
}
