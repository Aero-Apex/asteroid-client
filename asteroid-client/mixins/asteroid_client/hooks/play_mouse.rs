fn asteroid_play_mouse(pressed: bool, button: MouseButton) -> std::ops::ControlFlow<Result<()>> {
    // The ClickGUI has the mouse; a release still goes through, so nothing stays held.
    if pressed && crate::game::asteroid_core::gui_open() {
        return std::ops::ControlFlow::Break(Ok(()));
    }
    if pressed && button == MouseButton::Right && crate::game::asteroid_pvp::SKIP_RIGHT.with(|c| c.take()) {
        return std::ops::ControlFlow::Break(Ok(()));
    }
    std::ops::ControlFlow::Continue(())
}
