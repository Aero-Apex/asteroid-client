fn asteroid_mob_hit(game: &crate::game::Game<'_>) -> std::ops::ControlFlow<Option<(usize, f64)>> {
    match crate::game::asteroid_combat::mob_hit(game) {
        Some(hit) => std::ops::ControlFlow::Break(hit),
        None => std::ops::ControlFlow::Continue(()),
    }
}
