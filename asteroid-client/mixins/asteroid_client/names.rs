// NameChanger's names where the chat component can read them: the real one and
// the one to show instead. The mod sets them each tick.
use std::sync::Mutex;

static NAMES: Mutex<Option<(String, String)>> = Mutex::new(None);

pub fn set(names: Option<(String, String)>) {
    if let Ok(mut n) = NAMES.lock() {
        *n = names;
    }
}
/// The name shown, when NameChanger is on.
pub fn shown() -> Option<String> {
    NAMES.lock().ok()?.as_ref().map(|n| n.1.clone())
}
pub fn rename_text(text: &mut crate::text::Text, real: &str, shown: &str) {
    for span in &mut text.spans {
        if let Some(renamed) = replace_ignore_case(&span.text, real, shown) {
            span.text = renamed;
        }
        // What hovering, clicking or shift-clicking the name would show.
        let style = &mut span.style;
        if let Some(hover) = style.hover.as_mut() {
            rename_text(hover, real, shown);
        }
        if let Some(entity) = style.hover_entity.as_mut()
            && let Some(renamed) = replace_ignore_case(&entity.to_string(), real, shown)
            && let Ok(value) = serde_json::from_str(&renamed)
        {
            **entity = value;
        }
        if let Some(insertion) = style.insertion.as_mut()
            && let Some(renamed) = replace_ignore_case(insertion, real, shown)
        {
            *insertion = renamed;
        }
        use crate::text::ClickEvent as C;
        if let Some(C::SuggestCommand(c) | C::RunCommand(c) | C::CopyToClipboard(c)) = style.click.as_mut()
            && let Some(renamed) = replace_ignore_case(c, real, shown)
        {
            *c = renamed;
        }
    }
}
/// `text` with each `real` replaced by `shown`, whatever its case; None when
/// `real` is not in it, or `shown` already is (a rename applied twice).
fn replace_ignore_case(text: &str, real: &str, shown: &str) -> Option<String> {
    // ASCII lowering keeps every byte index, so matches found in `lower` cut `text` too.
    let lower = text.to_ascii_lowercase();
    let (needle, done) = (real.to_ascii_lowercase(), shown.to_ascii_lowercase());
    if needle.is_empty() || !lower.contains(&needle) || lower.contains(&done) {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(i) = lower[at..].find(&needle) {
        out.push_str(&text[at..at + i]);
        out.push_str(shown);
        at += i + needle.len();
    }
    out.push_str(&text[at..]);
    Some(out)
}
/// A line on its way to the chat.
pub fn renamed(mut text: crate::text::Text) -> crate::text::Text {
    if let Ok(names) = NAMES.lock()
        && let Some((real, shown)) = names.as_ref()
    {
        rename_text(&mut text, real, shown);
    }
    text
}
