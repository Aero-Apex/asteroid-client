fn asteroid_night_vision(game: &crate::game::Game<'_>) -> std::ops::ControlFlow<f32> {
    // Fullbright gives full night vision: the lightmap and shader packs (Iris `nightVision`,
    // Horizon packs) brighten caves and nights from it.
    if game.renderer.as_ref().is_some_and(|r| r.fullbright) {
        std::ops::ControlFlow::Break(1.0)
    } else {
        std::ops::ControlFlow::Continue(())
    }
}
