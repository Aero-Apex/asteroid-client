fn asteroid_look(game: &mut crate::game::Game<'_>) -> std::ops::ControlFlow<()> {
    if crate::game::asteroid_movement::freecam_on() && !crate::game::asteroid_core::gui_open() {
        // Freecam: the mouse turns the camera; the body keeps where it (or Baritone) looks.
        let look = game.look_options();
        let time = game.started.elapsed().as_secs_f64();
        let pointer = game.captured || game.agent_pointer;
        let grabbed = pointer && matches!(game.screen, crate::game::Screen::Playing) && game.player.is_some();
        let active = game.focused || game.agent_pointer;
        if let Some((x, y)) = game.mouse.handle_accumulated_movement(time, active, grabbed, look) {
            crate::game::asteroid_movement::freecam_turn(x, y);
        }
        std::ops::ControlFlow::Break(())
    } else if crate::game::asteroid_core::gui_open() {
        // The ClickGUI has the mouse: what moved it is thrown away, not saved up.
        let look = game.look_options();
        let time = game.started.elapsed().as_secs_f64();
        let _ = game.mouse.handle_accumulated_movement(time, true, false, look);
        std::ops::ControlFlow::Break(())
    } else {
        std::ops::ControlFlow::Continue(())
    }
}
