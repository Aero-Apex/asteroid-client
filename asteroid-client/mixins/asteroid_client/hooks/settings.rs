fn asteroid_page(mod_id: &str, buttons: &mut Vec<String>) {
    if mod_id == crate::game::asteroid_core::ID {
        buttons.extend(crate::game::asteroid_core::settings_labels());
    }
}
