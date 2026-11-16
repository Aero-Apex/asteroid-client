fn asteroid_mining(game: &mut crate::game::Game<'_>) {
    if !crate::game::asteroid_core::ACTIVE.with(|c| c.get()) {
        return;
    }
    let fast = crate::game::asteroid_core::NO_BREAK_DELAY.with(|c| c.get());
    let mult = crate::game::asteroid_core::SPEED_MINE.with(|c| c.get());
    let extra = match game.remote.as_ref().and_then(|r| r.destroying) {
        Some((pos, _, _)) if mult > 1.0 => Some(game.destroy_progress(pos) * (mult as f32 - 1.0)),
        _ => None,
    };
    if let Some(r) = game.remote.as_mut() {
        if fast {
            r.destroy_delay = 0;
        }
        if let (Some(extra), Some(d)) = (extra, r.destroying.as_mut()) {
            d.2 = (d.2 + extra).min(0.99);
        }
    }
}
