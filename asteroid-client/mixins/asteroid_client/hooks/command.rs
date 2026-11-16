fn asteroid_command(game: &mut crate::game::Game<'_>, command: &str, ran: &mut bool) {
    if !*ran && crate::game::asteroid_core::on_command(game, command) {
        *ran = true;
    }
}
