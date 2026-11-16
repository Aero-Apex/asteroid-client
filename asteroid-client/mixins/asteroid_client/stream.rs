// Asteroid Client's StreamProof: while it is on, the mod's drawing never reaches
// the game's picture. core's fill, text and line record here instead, and the
// overlay in horizon_hdr paints the list on a window recorders can't see.
use std::cell::{Cell, RefCell};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};

pub use horizon_hdr::asteroid_overlay::{Prim, dump, hidden_ok, status};

thread_local! {
    static REC: Cell<bool> = const { Cell::new(false) };
    static LIST: RefCell<Vec<Prim>> = const { RefCell::new(Vec::new()) };
}

pub fn recording() -> bool {
    REC.with(|c| c.get())
}
pub fn push(p: Prim) {
    LIST.with(|l| l.borrow_mut().push(p));
}

/// Once a frame: turns the overlay on or off, and hands it what was recorded.
pub fn frame(window: Option<&winit::window::Window>, on: bool, scale: f32) {
    let list = LIST.with(|l| std::mem::take(&mut *l.borrow_mut()));
    REC.with(|c| c.set(on));
    let owner = window.and_then(|w| match w.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(h.hwnd.get()),
        _ => None,
    });
    horizon_hdr::asteroid_overlay::publish(owner, on, list, scale);
}
