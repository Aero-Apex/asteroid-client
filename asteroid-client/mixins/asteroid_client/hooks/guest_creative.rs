fn asteroid_guest_creative(guest: &crate::lan::guest::Guest) -> std::ops::ControlFlow<bool> {
    let _ = guest;
    if crate::lan::guest::asteroid_flags::creative_all() {
        std::ops::ControlFlow::Break(true)
    } else {
        std::ops::ControlFlow::Continue(())
    }
}
