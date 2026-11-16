fn asteroid_click(mod_id: &str, button: usize) {
    if mod_id == crate::game::asteroid_core::ID {
        crate::game::asteroid_core::settings_click(button);
    }
}
