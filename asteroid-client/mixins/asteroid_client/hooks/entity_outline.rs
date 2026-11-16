fn asteroid_entity_outline(entity_id: u32, kind: &str, outline: &mut Option<u32>) {
    let _ = entity_id;
    if let Some(color) = crate::game::asteroid_extra::outline_color(kind) {
        *outline = Some(color);
    }
}
