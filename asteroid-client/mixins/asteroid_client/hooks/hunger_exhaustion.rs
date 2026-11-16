fn asteroid_hunger_exhaustion() -> std::ops::ControlFlow<()> {
    if crate::lan::guest::asteroid_flags::no_hunger() {
        std::ops::ControlFlow::Break(())
    } else {
        std::ops::ControlFlow::Continue(())
    }
}
