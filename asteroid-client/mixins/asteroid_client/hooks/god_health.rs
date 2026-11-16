fn asteroid_god_health(resources: &HealthResources, source: DamageTypeId) -> std::ops::ControlFlow<Result<BlockOutcome>> {
    let flags = (crate::lan::guest::asteroid_flags::god_mode(), crate::lan::guest::asteroid_flags::no_fall());
    if flags.0 || (flags.1 && source == resources.landing_source(LandingDamage::Fall)) {
        std::ops::ControlFlow::Break(Ok(BlockOutcome::default()))
    } else {
        std::ops::ControlFlow::Continue(())
    }
}
