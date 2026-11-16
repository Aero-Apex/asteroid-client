fn asteroid_own_name() -> std::ops::ControlFlow<String> {
    match crate::game::asteroid_core::shown_name() {
        Some(name) => std::ops::ControlFlow::Break(name),
        None => std::ops::ControlFlow::Continue(()),
    }
}
