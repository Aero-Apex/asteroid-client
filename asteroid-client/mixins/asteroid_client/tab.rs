// Inside the game's remote module, for its private fields: NameChanger's name on
// the player's own tab list entry and on the death screen.
pub fn rename(game: &mut crate::game::Game<'_>) {
    let Some(r) = game.remote.as_mut() else {
        return;
    };
    let real = r.profile_name.clone();
    if real.is_empty() {
        return;
    }
    for player in r.tab_players.values_mut() {
        if let Some(name) = crate::game::asteroid_core::tab_name(&player.name, &real) {
            player.name = name;
        }
    }
    if let Some(shown) = horizon_client::chat::asteroid_names::shown()
        && let Some(message) = r.death_message.as_mut()
    {
        horizon_client::chat::asteroid_names::rename_text(message, &real, &shown);
    }
}

/// This player's ping as the server last gave it.
pub fn own_ping(game: &crate::game::Game<'_>) -> Option<i32> {
    let r = game.remote.as_ref()?;
    r.tab_players.get(&r.profile).map(|p| p.latency)
}
