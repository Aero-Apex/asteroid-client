fn asteroid_typing(pressed: bool) -> std::ops::ControlFlow<Result<()>> {
    // A letter typed in the ClickGUI is not a key bind; a release still goes through.
    if pressed && crate::game::asteroid_core::TYPING.with(|c| c.get()) {
        std::ops::ControlFlow::Break(Ok(()))
    } else {
        std::ops::ControlFlow::Continue(())
    }
}
