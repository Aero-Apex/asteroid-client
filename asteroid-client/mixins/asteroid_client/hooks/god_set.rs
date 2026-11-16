fn asteroid_god_set(player: &crate::motion::PlayerMotion, value: f32, absorption: bool) -> std::ops::ControlFlow<Result<()>> {
    // The save's own health, dead or not, loads before the first ticks.
    if player.health.tick_count > 20 && crate::lan::guest::asteroid_flags::god_mode() && !absorption && value <= 0.0 {
        std::ops::ControlFlow::Break(Ok(()))
    } else {
        std::ops::ControlFlow::Continue(())
    }
}
