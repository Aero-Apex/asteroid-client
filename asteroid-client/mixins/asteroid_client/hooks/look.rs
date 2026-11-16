fn asteroid_look(game: &mut crate::game::Game<'_>) -> std::ops::ControlFlow<()> {
    if crate::game::asteroid_core::gui_open() {
        // The ClickGUI has the mouse: what moved it is thrown away, not saved up.
        let look = game.look_options();
        let time = game.started.elapsed().as_secs_f64();
        let _ = game.mouse.handle_accumulated_movement(time, true, false, look);
        std::ops::ControlFlow::Break(())
    } else {
        std::ops::ControlFlow::Continue(())
    }
}
