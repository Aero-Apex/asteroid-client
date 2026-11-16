fn asteroid_hunger_tick(player: &mut PlayerMotion) {
    if crate::lan::guest::asteroid_flags::no_hunger() {
        player.health.food.food = 20;
        player.health.food.exhaustion = 0.0;
    }
}
