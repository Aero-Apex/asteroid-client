fn asteroid_listed(mods: &mut Vec<crate::game::native_hooks::NativeMod>) {
    use crate::game::asteroid_core as m;
    mods.push(crate::game::native_hooks::NativeMod {
        id: m::ID.into(),
        title: format!("{} {}", m::NAME, m::VERSION),
        description: m::DESCRIPTION.into(),
        keys: vec![[m::GUI_KEY_NAME.into(), "ClickGUI".into(), m::GUI_KEY.into()]],
    });
}
