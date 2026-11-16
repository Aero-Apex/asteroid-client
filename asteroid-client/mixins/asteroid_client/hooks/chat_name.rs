fn asteroid_chat_name(mut value: Message) -> Message {
    value.content = crate::chat::asteroid_names::renamed(value.content);
    value
}
