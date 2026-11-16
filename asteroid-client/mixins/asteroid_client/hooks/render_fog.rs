fn asteroid_render_fog(fog: &mut horizon_client_render::fog::FogData) {
    crate::game::asteroid_extra::apply_fog(fog);
}
