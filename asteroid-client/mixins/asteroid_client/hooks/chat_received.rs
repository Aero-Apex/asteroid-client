fn asteroid_client_chat(message: &horizon_client::text::Text, cancel: &mut bool) -> std::ops::ControlFlow<()> {
    if crate::game::asteroid_extra::drop_repeat(&message.plain()) {
        *cancel = true;
        std::ops::ControlFlow::Break(())
    } else {
        std::ops::ControlFlow::Continue(())
    }
}
