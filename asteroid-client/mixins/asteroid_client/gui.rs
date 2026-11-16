// Asteroid Client's ClickGUI: a panel with the categories and the general pages
// down the side, modules as rows with a switch, and each module's settings
// opening under its row. Drawn at a finer scale than the game's GUI, so far more
// fits; every control is laid out here and clicked through the list it leaves.
use super::Game;
use crate::game::asteroid_core::{self as core, CATS, Focus, State, Val};
use horizon_client::hud::HudBatch;
use std::cell::{Cell, RefCell};

const PAGE_CONFIGS: usize = CATS.len();
const PAGE_THEME: usize = CATS.len() + 1;
const PAGE_KEYS: usize = CATS.len() + 2;
const PAGE_FRIENDS: usize = CATS.len() + 3;
const PAGE_PRIVACY: usize = CATS.len() + 4;
const GENERAL: [&str; 5] = ["Configs", "Theme", "Keybinds", "Friends", "Privacy"];

#[derive(Clone, Debug)]
enum Hit {
    Page(usize),
    Toggle(usize),
    Expand(usize),
    Bool(usize, usize),
    /// Module, setting, track x and width.
    Slider(usize, usize, f32, f32),
    Choice(usize, usize),
    Text(usize, usize),
    Bind(usize),
    Visible(usize),
    Focus(Focus),
    ConfigLoad(String),
    ConfigSave(String),
    ConfigDelete(String),
    ConfigNew,
    FriendRemove(String),
    FriendAdd,
    Move,
    Inside,
}

thread_local! {
    static HITS: RefCell<Vec<([f32; 4], Hit)>> = const { RefCell::new(Vec::new()) };
    /// The drawing scale of the last frame.
    static K: Cell<f32> = const { Cell::new(0.5) };
}

fn mix(a: u32, b: u32, t: f32) -> u32 {
    let c = |shift: u32| {
        let (x, y) = (((a >> shift) & 255) as f32, ((b >> shift) & 255) as f32);
        ((x + (y - x) * t) as u32) << shift
    };
    c(16) | c(8) | c(0)
}

/// Drawing in the panel's units.
struct Ui<'a, 'g> {
    g: &'a Game<'g>,
    batch: &'a mut HudBatch,
    k: f32,
    mouse: (f32, f32),
    accent: u32,
    alpha: u32,
    hits: Vec<([f32; 4], Hit)>,
}
impl Ui<'_, '_> {
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, c: u32) {
        core::fill(self.g, self.batch, x * self.k, y * self.k, w * self.k, h * self.k, c);
    }
    fn round(&mut self, x: f32, y: f32, w: f32, h: f32, r: f32, c: u32) {
        let r = r.min(w / 2.0).min(h / 2.0);
        let rows = (r * self.k * self.batch.scale).ceil().clamp(1.0, 10.0) as i32;
        let step = r / rows as f32;
        for i in 0..rows {
            let dy = (i as f32 + 0.5) * step;
            let inset = r - (r * r - (r - dy) * (r - dy)).max(0.0).sqrt();
            self.rect(x + inset, y + i as f32 * step, w - inset * 2.0, step, c);
            self.rect(x + inset, y + h - (i + 1) as f32 * step, w - inset * 2.0, step, c);
        }
        self.rect(x, y + r, w, h - r * 2.0, c);
    }
    fn text(&mut self, s: &str, x: f32, y: f32, rgb: u32, size: f32) {
        core::text_k(self.g, self.batch, s, x * self.k, y * self.k, rgb, size * self.k);
    }
    fn width(&self, s: &str, size: f32) -> f32 {
        self.g.native_text_width(s) * size
    }
    fn clip(&self, s: &str, max: f32, size: f32) -> String {
        core::clip_text(self.g, s, max / size)
    }
    fn over(&self, x: f32, y: f32, w: f32, h: f32) -> bool {
        self.mouse.0 >= x && self.mouse.0 < x + w && self.mouse.1 >= y && self.mouse.1 < y + h
    }
    fn hit(&mut self, x: f32, y: f32, w: f32, h: f32, what: Hit) {
        self.hits.push(([x, y, w, h], what));
    }
    fn switch(&mut self, x: f32, y: f32, on: bool) {
        let (w, h) = (24.0, 12.0);
        let track = if on { 0xFF00_0000 | self.accent } else { 0xFF_3A3742 };
        self.round(x, y, w, h, 6.0, track);
        let knob = if on { x + w - 11.0 } else { x + 1.0 };
        self.round(knob, y + 1.0, 10.0, 10.0, 5.0, 0xFFF4_F2F6);
    }
    fn button(&mut self, label: &str, x: f32, y: f32, w: f32, h: f32, strong: bool, what: Hit) {
        let hover = self.over(x, y, w, h);
        let c = if strong { 0xFF00_0000 | self.accent } else { 0xFF_2A2830 };
        self.rect(x, y, w, h, c);
        // The hover is a 1 px outline, not a fade.
        if hover {
            let o = 0xFF_F4F2F6;
            self.rect(x, y, w, 1.0, o);
            self.rect(x, y + h - 1.0, w, 1.0, o);
            self.rect(x, y, 1.0, h, o);
            self.rect(x + w - 1.0, y, 1.0, h, o);
        }
        let tw = self.width(label, 1.0);
        self.text(label, x + (w - tw) / 2.0, y + (h - 8.0) / 2.0, 0xF4_F2F6, 1.0);
        self.hit(x, y, w, h, what);
    }
    /// A text box: `value`, or the buffer with a caret while it has the focus.
    fn field(&mut self, s: &State, value: &str, hint: &str, x: f32, y: f32, w: f32, h: f32, me: Focus) {
        let on = s.gui.focus == me;
        self.rect(x, y, w, h, if on { 0xFF_2E2B36 } else { 0xFF_1E1C24 });
        if on {
            self.rect(x, y + h - 1.0, w, 1.0, 0xFF00_0000 | self.accent);
        }
        let shown = if on {
            let caret = if s.ticks % 12 < 6 { "_" } else { " " };
            format!("{}{caret}", s.gui.buffer)
        } else {
            value.to_owned()
        };
        let empty = shown.trim().is_empty();
        let (t, c) = if empty && !on { (hint.to_owned(), 0x9A_95A6) } else { (shown, 0xF4_F2F6) };
        // The end of what is typed stays in view.
        let mut cut = t.as_str();
        while self.width(cut, 1.0) > w - 8.0 && !cut.is_empty() {
            let mut it = cut.chars();
            it.next();
            cut = it.as_str();
        }
        let cut = cut.to_owned();
        self.text(&cut, x + 4.0, y + (h - 8.0) / 2.0, c, 1.0);
        self.hit(x, y, w, h, Hit::Focus(me));
    }
}

fn scale_of(batch: &HudBatch) -> f32 {
    // Two screen pixels to a font pixel, where the GUI scale leaves room for that.
    (2.0 / batch.scale).min(1.0)
}

pub fn draw(g: &Game<'_>, s: &mut State, batch: &mut HudBatch) {
    let k = scale_of(batch);
    K.with(|c| c.set(k));
    if s.gui.page == PAGE_CONFIGS && s.gui.configs_at != s.gui.page + 1 {
        s.gui.configs_at = s.gui.page + 1;
        s.gui.configs = core::profiles(g);
    }
    if s.gui.page != PAGE_CONFIGS {
        s.gui.configs_at = 0;
    }
    let (vw, vh) = (batch.width / k, batch.height / k);
    let mouse = (g.cursor.0 / k, g.cursor.1 / k);
    let accent = core::accent(s);
    let opacity = (s.n("Theme", "Opacity").clamp(40.0, 100.0) * 2.55) as u32;
    let alpha = opacity << 24;
    let mut ui = Ui { g, batch, k, mouse, accent, alpha, hits: Vec::new() };
    let (pw, ph) = ((vw - 16.0).min(660.0), (vh - 16.0).min(420.0));
    let px = ((vw - pw) / 2.0 + s.gui.offset.0).clamp(0.0, vw - pw);
    let py = ((vh - ph) / 2.0 + s.gui.offset.1).clamp(0.0, vh - ph);
    let side = 134.0;
    // One flat surface: a neutral panel, a slightly darker side bar, a 1 px edge.
    let bg = 0xFF_141217;
    let bar = 0xFF_0F0E13;
    ui.round(px - 1.0, py - 1.0, pw + 2.0, ph + 2.0, 9.0, 0xFF_2A2830);
    ui.round(px, py, pw, ph, 8.0, alpha | bg);
    ui.hit(px, py, pw, ph, Hit::Inside);
    ui.round(px, py, side, ph, 8.0, alpha | bar);
    ui.rect(px + side - 8.0, py, 8.0, ph, alpha | bar);
    ui.rect(px + side, py, 1.0, ph, 0xFF_2A2830);
    // The title moves the panel.
    ui.text("Asteroid", px + 12.0, py + 11.0, 0xFF_F4F2F6, 1.25);
    ui.text(&format!("Client {}", core::VERSION), px + 12.0, py + 25.0, 0x9A_95A6, 0.75);
    ui.hit(px, py, side, 36.0, Hit::Move);
    let mut y = py + 46.0;
    ui.text("MODULES", px + 12.0, y, 0x9A_95A6, 0.75);
    y += 14.0;
    for (i, cat) in CATS.iter().enumerate() {
        let list = s.in_cat(*cat);
        let on = list.iter().filter(|m| s.modules[**m].on).count();
        side_row(&mut ui, s, px, y, side, i, cat.name(), &on.to_string());
        y += 22.0;
    }
    y += 8.0;
    ui.text("GENERAL", px + 12.0, y, 0x9A_95A6, 0.75);
    y += 14.0;
    for (j, name) in GENERAL.iter().enumerate() {
        let i = PAGE_CONFIGS + j;
        let extra = if i == PAGE_PRIVACY && s.on("StreamProof") { "ON".to_owned() } else { String::new() };
        side_row(&mut ui, s, px, y, side, i, name, &extra);
        y += 22.0;
    }
    let (cx, cy, cw, ch) = (px + side + 12.0, py + 12.0, pw - side - 24.0, ph - 24.0);
    match s.gui.page {
        PAGE_CONFIGS => configs(&mut ui, s, cx, cy, cw, ch),
        PAGE_THEME => theme(&mut ui, s, cx, cy, cw, ch),
        PAGE_KEYS => keybinds(&mut ui, s, cx, cy, cw, ch),
        PAGE_FRIENDS => friends(&mut ui, s, cx, cy, cw, ch),
        PAGE_PRIVACY => privacy(&mut ui, s, cx, cy, cw, ch),
        _ => modules(&mut ui, s, cx, cy, cw, ch),
    }
    let hits = std::mem::take(&mut ui.hits);
    HITS.with(|h| *h.borrow_mut() = hits);
    // A slider being dragged follows the pointer.
    if let Some((m, set, x, w)) = s.gui.drag {
        set_slider(s, m, set, x, w, mouse.0);
    }
    if let Some((fx, fy)) = s.gui.moving {
        s.gui.offset = (mouse.0 - fx, mouse.1 - fy);
    }
}

#[allow(clippy::too_many_arguments)]
fn side_row(ui: &mut Ui<'_, '_>, s: &State, px: f32, y: f32, side: f32, page: usize, name: &str, count: &str) {
    let (x, w) = (px + 6.0, side - 12.0);
    let selected = s.gui.page == page && s.gui.search.is_empty();
    if selected {
        ui.rect(x, y - 1.0, w, 18.0, 0xFF_2A2830);
        ui.rect(x, y + 2.0, 2.0, 12.0, 0xFF00_0000 | ui.accent);
    } else if ui.over(x, y - 1.0, w, 18.0) {
        ui.rect(x, y - 1.0, w, 18.0, 0xFF_1E1C24);
    }
    let c = if selected { 0xFF_F4F2F6 } else { 0xC9_C5D0 };
    ui.text(name, x + 10.0, y + 4.0, c, 1.0);
    let cw = ui.width(count, 0.75);
    ui.text(count, x + w - cw - 6.0, y + 5.0, 0x9A_95A6, 0.75);
    ui.hit(x, y - 1.0, w, 18.0, Hit::Page(page));
}

/// A heading in one colour, with a plain line under it.
fn heading(ui: &mut Ui<'_, '_>, x: f32, y: f32, first: &str, second: &str, sub: &str) {
    ui.text(&format!("{first} {second}"), x, y, 0xFF_F4F2F6, 1.5);
    ui.text(sub, x, y + 15.0, 0x9A_95A6, 0.75);
}

fn modules(ui: &mut Ui<'_, '_>, s: &mut State, x: f32, y: f32, w: f32, h: f32) {
    let searching = !s.gui.search.is_empty();
    let list: Vec<usize> = if searching {
        let want = s.gui.search.to_lowercase();
        (0..s.modules.len()).filter(|&i| s.modules[i].name.to_lowercase().contains(&want)).collect()
    } else {
        s.in_cat(CATS[s.gui.page.min(CATS.len() - 1)])
    };
    let on = list.iter().filter(|m| s.modules[**m].on).count();
    let title = if searching { "Search" } else { CATS[s.gui.page.min(CATS.len() - 1)].name() };
    heading(ui, x, y, title, "modules", &format!("{} shown, {on} enabled", list.len()));
    let search = s.gui.search.clone();
    ui.field(s, &search, "Search modules", x + w - 150.0, y, 150.0, 18.0, Focus::Search);
    if s.on("StreamProof") && !crate::game::asteroid_stream::hidden_ok() {
        ui.text("Warning: Stream Proof is not hiding yet", x + w - 150.0, y + 20.0, 0xFF_C040, 0.75);
    }
    let (top, bottom) = (y + 32.0, y + h);
    let mut ry = top - s.gui.scroll;
    for m in list {
        ry += module_row(ui, s, m, x, ry, w, top, bottom);
        ry += 4.0;
    }
    let content = ry + s.gui.scroll - top;
    let room = bottom - top;
    s.gui.scroll = s.gui.scroll.clamp(0.0, (content - room).max(0.0));
    if content > room {
        // Where the list is.
        let bar = (room * room / content).max(16.0);
        let at = top + (room - bar) * (s.gui.scroll / (content - room).max(1.0));
        ui.round(x + w + 4.0, at, 3.0, bar, 1.5, 0x40FF_FFFF);
    }
}

/// Draws a module and, if open, its settings; returns the height used.
#[allow(clippy::too_many_arguments)]
fn module_row(ui: &mut Ui<'_, '_>, s: &State, m: usize, x: f32, y: f32, w: f32, top: f32, bottom: f32) -> f32 {
    let md = &s.modules[m];
    let open = s.gui.expanded == Some(m);
    let row_h = 30.0;
    let set_h = 20.0;
    let total = row_h + if open { (md.sets.len() + 2) as f32 * set_h + 6.0 } else { 0.0 };
    if y + total < top || y > bottom {
        return total;
    }
    let shown = |yy: f32, hh: f32| yy >= top - 0.5 && yy + hh <= bottom + 0.5;
    // Rows that don't fit whole are skipped, as the panel can't cut a row in half.
    if shown(y, row_h) {
        let hover = ui.over(x, y, w, row_h);
        let bg = if open { 0xFF_221F28 } else if hover { 0xFF_1C1A21 } else { 0xFF_17161B };
        ui.round(x, y, w, total.min(bottom - y), 5.0, bg);
        if md.on {
            ui.round(x, y + 6.0, 2.5, row_h - 12.0, 1.25, 0xFF00_0000 | ui.accent);
        }
        ui.text(md.name, x + 9.0, y + 5.0, if md.on { 0xFF_FFFF } else { 0xD8_D4DE }, 1.0);
        let mut after = x + 9.0 + ui.width(md.name, 1.0) + 5.0;
        if s.on("StreamProof") && core::stream_leak(md.name) {
            ui.text("!", after, y + 5.0, 0xFF_C040, 1.0);
            after += 8.0;
        }
        if !md.bind.is_empty() {
            let label = format!("[{}]", core::key_label(&md.bind));
            ui.text(&label, after, y + 6.0, 0x9A_95A6, 0.75);
        }
        let desc = ui.clip(md.desc, w - 52.0, 0.75);
        ui.text(&desc, x + 9.0, y + 17.0, 0x9A_95A6, 0.75);
        ui.hit(x, y, w - 38.0, row_h, Hit::Expand(m));
        ui.switch(x + w - 32.0, y + 9.0, md.on);
        ui.hit(x + w - 38.0, y, 38.0, row_h, Hit::Toggle(m));
    }
    if !open {
        return total;
    }
    let mut ry = y + row_h;
    let (lx, vx) = (x + 12.0, x + w * 0.45);
    let vw = x + w - 10.0 - vx;
    for (j, st) in md.sets.iter().enumerate() {
        if shown(ry, set_h) {
            ui.text(st.name, lx, ry + 6.0, 0xD8_D4DE, 1.0);
            match &st.val {
                Val::Bool(b) => {
                    ui.switch(x + w - 34.0, ry + 4.0, *b);
                    ui.hit(lx, ry, w - 22.0, set_h, Hit::Bool(m, j));
                }
                Val::Num(v, min, max, _) => {
                    let (tx, tw) = (vx, vw - 38.0);
                    let t = ((*v - *min) / (*max - *min).max(1e-9)).clamp(0.0, 1.0) as f32;
                    ui.round(tx, ry + 8.0, tw, 4.0, 2.0, 0xFF_3A3742);
                    ui.round(tx, ry + 8.0, (tw * t).max(4.0), 4.0, 2.0, 0xFF00_0000 | ui.accent);
                    ui.round(tx + tw * t - 4.0, ry + 6.0, 8.0, 8.0, 4.0, 0xFFF4_F2F6);
                    ui.hit(tx - 4.0, ry, tw + 8.0, set_h, Hit::Slider(m, j, tx, tw));
                    let shown = st.val.show();
                    let vwid = ui.width(&shown, 1.0);
                    ui.round(tx + tw + 6.0, ry + 3.0, 32.0, 14.0, 3.0, 0xFF_1E1C24);
                    ui.text(&shown, tx + tw + 22.0 - vwid / 2.0, ry + 6.0, 0xF0_EEF4, 1.0);
                }
                Val::Choice(..) => {
                    let label = format!("‹ {} ›", st.val.show());
                    ui.button(&label, vx, ry + 2.0, vw, 16.0, false, Hit::Choice(m, j));
                }
                Val::Text(t) => {
                    let value = t.clone();
                    ui.field(s, &value, "", vx, ry + 2.0, vw, 16.0, Focus::Set(m, j));
                }
            }
        }
        ry += set_h;
    }
    if shown(ry, set_h) {
        ui.text("Bind", lx, ry + 6.0, 0xD8_D4DE, 1.0);
        let label = if s.gui.binding == Some(m) {
            "Press a key...".to_owned()
        } else if md.bind.is_empty() {
            "None".to_owned()
        } else {
            core::key_label(&md.bind)
        };
        ui.button(&label, vx, ry + 2.0, vw, 16.0, s.gui.binding == Some(m), Hit::Bind(m));
    }
    ry += set_h;
    if shown(ry, set_h) {
        ui.text("Show in the module list", lx, ry + 6.0, 0xD8_D4DE, 1.0);
        ui.switch(x + w - 34.0, ry + 4.0, md.visible);
        ui.hit(lx, ry, w - 22.0, set_h, Hit::Visible(m));
    }
    total
}

fn configs(ui: &mut Ui<'_, '_>, s: &mut State, x: f32, y: f32, w: f32, h: f32) {
    heading(ui, x, y, "Saved", "Configs", "A config keeps every module, setting and key bind. Name one after a server's address and it loads when you join.");
    let mut ry = y + 34.0;
    let name = s.gui.config_name.clone();
    ui.field(s, &name, "New config name", x, ry, w - 84.0, 20.0, Focus::Config);
    ui.button("Save new", x + w - 78.0, ry, 78.0, 20.0, true, Hit::ConfigNew);
    ry += 28.0;
    let names = s.gui.configs.clone();
    if names.is_empty() {
        ui.text("No saved configs yet", x + 2.0, ry + 4.0, 0x9A_95A6, 1.0);
    }
    for c in names {
        if ry + 24.0 > y + h {
            break;
        }
        ui.round(x, ry, w, 24.0, 5.0, 0x0DFF_FFFF);
        let label = ui.clip(&c, w - 170.0, 1.0);
        ui.text(&label, x + 8.0, ry + 8.0, 0xE8_E4EE, 1.0);
        ui.button("Load", x + w - 160.0, ry + 3.0, 50.0, 18.0, true, Hit::ConfigLoad(c.clone()));
        ui.button("Save", x + w - 106.0, ry + 3.0, 50.0, 18.0, false, Hit::ConfigSave(c.clone()));
        ui.button("Delete", x + w - 52.0, ry + 3.0, 50.0, 18.0, false, Hit::ConfigDelete(c.clone()));
        ry += 27.0;
    }
}

fn theme(ui: &mut Ui<'_, '_>, s: &mut State, x: f32, y: f32, w: f32, h: f32) {
    heading(ui, x, y, "Client", "Theme", "How the panel and the HUD look.");
    let Some(m) = s.index("Theme") else {
        return;
    };
    let mut ry = y + 34.0;
    // The accent as swatches.
    let names = core::ACCENTS;
    ui.text("Accent", x + 2.0, ry + 5.0, 0xD8_D4DE, 1.0);
    let current = s.c("Theme", "Accent");
    let set = s.modules[m].sets.iter().position(|st| st.name == "Accent").unwrap_or(0);
    for (i, (name, rgb)) in names.iter().enumerate() {
        let sx = x + 70.0 + i as f32 * 24.0;
        if *name == current {
            ui.round(sx - 2.0, ry - 2.0, 22.0, 22.0, 6.0, 0xFFF4_F2F6);
        }
        ui.round(sx, ry, 18.0, 18.0, 5.0, 0xFF00_0000 | rgb);
        ui.hit(sx, ry, 18.0, 18.0, Hit::Choice(m + 100_000 * (i + 1), set));
    }
    ry += 30.0;
    let _ = h;
    let hud = s.index("HUD");
    for (label, module, setting) in [
        ("Panel opacity", "Theme", "Opacity"),
        ("HUD style", "Theme", "HudStyle"),
        ("HUD: stat pills", "HUD", "Pills"),
        ("HUD: resources", "HUD", "Resources"),
        ("HUD: key binds", "HUD", "Keybinds"),
        ("HUD: potions", "HUD", "Potions"),
        ("HUD: module list", "HUD", "ArrayList"),
        ("HUD: rainbow", "HUD", "Rainbow"),
    ] {
        let Some(mi) = s.index(module) else {
            continue;
        };
        let Some(j) = s.modules[mi].sets.iter().position(|st| st.name == setting) else {
            continue;
        };
        ui.round(x, ry, w, 22.0, 5.0, 0x0DFF_FFFF);
        ui.text(label, x + 8.0, ry + 7.0, 0xD8_D4DE, 1.0);
        match &s.modules[mi].sets[j].val {
            Val::Bool(b) => {
                ui.switch(x + w - 32.0, ry + 5.0, *b);
                ui.hit(x, ry, w, 22.0, Hit::Bool(mi, j));
            }
            Val::Num(v, min, max, _) => {
                let (tx, tw) = (x + w * 0.45, w * 0.55 - 44.0);
                let t = ((*v - *min) / (*max - *min).max(1e-9)).clamp(0.0, 1.0) as f32;
                ui.round(tx, ry + 9.0, tw, 4.0, 2.0, 0x30FF_FFFF);
                ui.round(tx, ry + 9.0, (tw * t).max(4.0), 4.0, 2.0, 0xFF00_0000 | ui.accent);
                ui.round(tx + tw * t - 4.0, ry + 7.0, 8.0, 8.0, 4.0, 0xFFF4_F2F6);
                ui.hit(tx - 4.0, ry, tw + 8.0, 22.0, Hit::Slider(mi, j, tx, tw));
                let v = s.modules[mi].sets[j].val.show();
                ui.text(&v, tx + tw + 10.0, ry + 7.0, 0xF0_EEF4, 1.0);
            }
            Val::Choice(..) => {
                let label = format!("‹ {} ›", s.modules[mi].sets[j].val.show());
                ui.button(&label, x + w - 110.0, ry + 3.0, 104.0, 16.0, false, Hit::Choice(mi, j));
            }
            Val::Text(_) => {}
        }
        ry += 25.0;
    }
    let _ = hud;
}

fn keybinds(ui: &mut Ui<'_, '_>, s: &mut State, x: f32, y: f32, w: f32, h: f32) {
    heading(ui, x, y, "Module", "Keybinds", "Click a key to change it, then press the new one (Backspace clears it). Open a module to bind one that has none.");
    let mut ry = y + 34.0 - s.gui.scroll;
    let (top, bottom) = (y + 34.0, y + h);
    let bound: Vec<usize> = (0..s.modules.len()).filter(|&i| !s.modules[i].bind.is_empty()).collect();
    if bound.is_empty() {
        ui.text("Nothing is bound yet", x + 2.0, ry + 4.0, 0x9A_95A6, 1.0);
    }
    for m in &bound {
        if ry >= top - 0.5 && ry + 22.0 <= bottom {
            ui.round(x, ry, w, 22.0, 5.0, 0x0DFF_FFFF);
            ui.text(s.modules[*m].name, x + 8.0, ry + 7.0, 0xD8_D4DE, 1.0);
            let label = if s.gui.binding == Some(*m) { "Press a key...".to_owned() } else { core::key_label(&s.modules[*m].bind) };
            ui.button(&label, x + w - 110.0, ry + 3.0, 104.0, 16.0, s.gui.binding == Some(*m), Hit::Bind(*m));
        }
        ry += 25.0;
    }
    let content = bound.len() as f32 * 25.0;
    s.gui.scroll = s.gui.scroll.clamp(0.0, (content - (bottom - top)).max(0.0));
}

fn friends(ui: &mut Ui<'_, '_>, s: &mut State, x: f32, y: f32, w: f32, h: f32) {
    heading(ui, x, y, "Your", "Friends", "Combat modules and the player pause leave friends alone.");
    let mut ry = y + 34.0;
    let name = s.gui.friend_name.clone();
    ui.field(s, &name, "Player name", x, ry, w - 84.0, 20.0, Focus::Friend);
    ui.button("Add", x + w - 78.0, ry, 78.0, 20.0, true, Hit::FriendAdd);
    ry += 28.0;
    let list: Vec<String> = s.friends.iter().cloned().collect();
    if list.is_empty() {
        ui.text("No friends added yet", x + 2.0, ry + 4.0, 0x9A_95A6, 1.0);
    }
    for f in list {
        if ry + 22.0 > y + h {
            break;
        }
        ui.round(x, ry, w, 22.0, 5.0, 0x0DFF_FFFF);
        ui.text(&f, x + 8.0, ry + 7.0, 0xD8_D4DE, 1.0);
        ui.button("Remove", x + w - 60.0, ry + 3.0, 56.0, 16.0, false, Hit::FriendRemove(f.clone()));
        ry += 25.0;
    }
}

fn privacy(ui: &mut Ui<'_, '_>, s: &mut State, x: f32, y: f32, w: f32, _h: f32) {
    heading(ui, x, y, "Stream", "Proof", "Keeps the menu, HUD and overlays out of OBS, Discord and other recordings. You still see them.");
    let mut ry = y + 34.0;
    let Some(m) = s.index("StreamProof") else {
        return;
    };
    ui.round(x, ry, w, 24.0, 5.0, 0x0DFF_FFFF);
    ui.text("Stream Proof", x + 8.0, ry + 8.0, 0xFF_FFFF, 1.0);
    ui.switch(x + w - 32.0, ry + 6.0, s.modules[m].on);
    ui.hit(x, ry, w, 24.0, Hit::Toggle(m));
    ry += 28.0;
    let status = crate::game::asteroid_stream::status();
    let (line, color) = if !s.modules[m].on {
        ("Off: everything the mod draws can be recorded".to_owned(), 0x9A_95A6)
    } else if crate::game::asteroid_stream::hidden_ok() {
        ("Hidden: Windows confirms the overlay is left out of captures".to_owned(), 0x60_E090)
    } else {
        (format!("Warning: {status}"), 0xFF_C040)
    };
    let line = ui.clip(&line, w - 4.0, 1.0);
    ui.text(&line, x + 2.0, ry + 2.0, color, 1.0);
    ry += 18.0;
    ui.text("Warning: Viewers can still see these while they are on:", x + 2.0, ry, 0xFF_C040, 1.0);
    ry += 13.0;
    let on: Vec<&str> = s.modules.iter().filter(|md| md.on && core::stream_leak(md.name)).map(|md| md.name).collect();
    let text = if on.is_empty() { "None of them is on".to_owned() } else { on.join(", ") };
    let text = ui.clip(&text, w - 4.0, 1.0);
    ui.text(&text, x + 2.0, ry, 0xD8_D4DE, 1.0);
    ry += 13.0;
    ui.text("They change the game's own picture or your view, which a recording shows. So does your cursor.", x + 2.0, ry, 0x9A_95A6, 0.75);
    ry += 20.0;
    heading(ui, x, ry, "Hide", "Name", "The name shown for you on your screen: chat, tab list, name tag, HUD.");
    ry += 30.0;
    let Some(n) = s.index("NameChanger") else {
        return;
    };
    ui.round(x, ry, w, 24.0, 5.0, 0x0DFF_FFFF);
    ui.text("Name Changer", x + 8.0, ry + 8.0, 0xFF_FFFF, 1.0);
    ui.switch(x + w - 32.0, ry + 6.0, s.modules[n].on);
    ui.hit(x + w - 40.0, ry, 40.0, 24.0, Hit::Toggle(n));
    let j = s.modules[n].sets.iter().position(|st| st.name == "Name").unwrap_or(0);
    let value = s.t("NameChanger", "Name").to_owned();
    ui.field(s, &value, "Name", x + 100.0, ry + 3.0, w - 150.0, 18.0, Focus::Set(n, j));
}

fn set_slider(s: &mut State, m: usize, set: usize, x: f32, w: f32, mouse: f32) {
    let t = f64::from(((mouse - x) / w.max(1.0)).clamp(0.0, 1.0));
    if let Some(Val::Num(v, min, max, step)) = s.modules.get_mut(m).and_then(|md| md.sets.get_mut(set)).map(|st| &mut st.val) {
        let raw = *min + (*max - *min) * t;
        let snapped = ((raw / *step).round() * *step).clamp(*min, *max);
        if (snapped - *v).abs() > 1e-9 {
            *v = snapped;
            s.dirty = true;
        }
    }
}

/// Ends typing, keeping what was typed.
fn commit(g: &Game<'_>, s: &mut State) {
    let text = std::mem::take(&mut s.gui.buffer);
    match std::mem::take(&mut s.gui.focus) {
        Focus::Set(m, j) => {
            if let Some(st) = s.modules.get_mut(m).and_then(|md| md.sets.get_mut(j)) {
                let _ = st.val.parse(text.trim());
                s.dirty = true;
            }
        }
        Focus::Search => s.gui.search = text,
        Focus::Config => s.gui.config_name = text,
        Focus::Friend => s.gui.friend_name = text,
        Focus::None => {}
    }
    let _ = g;
}

/// A mouse button over the panel.
pub fn click(g: &Game<'_>, s: &mut State, right: bool) {
    let k = K.with(|c| c.get());
    let (mx, my) = (g.cursor.0 / k, g.cursor.1 / k);
    let found = HITS.with(|h| {
        h.borrow()
            .iter()
            .rev()
            .find(|(r, _)| mx >= r[0] && mx < r[0] + r[2] && my >= r[1] && my < r[1] + r[3])
            .map(|(_, what)| what.clone())
    });
    let keep_focus = matches!(&found, Some(Hit::Focus(f)) if *f == s.gui.focus);
    if s.gui.focus != Focus::None && !keep_focus {
        commit(g, s);
    }
    let Some(what) = found else {
        return;
    };
    match what {
        Hit::Page(p) => {
            s.gui.page = p;
            s.gui.scroll = 0.0;
            s.gui.search.clear();
            s.gui.expanded = None;
        }
        Hit::Toggle(m) => s.toggle(m),
        Hit::Expand(m) => {
            if right {
                s.toggle(m);
            } else {
                s.gui.expanded = if s.gui.expanded == Some(m) { None } else { Some(m) };
            }
        }
        Hit::Bool(m, j) => {
            s.modules[m].sets[j].val.step(1);
            s.dirty = true;
        }
        Hit::Slider(m, j, x, w) => {
            s.gui.drag = Some((m, j, x, w));
            set_slider(s, m, j, x, w, mx);
        }
        Hit::Choice(m, j) if m >= 100_000 => {
            // A theme swatch: the module index and the swatch in one.
            let (swatch, module) = (m / 100_000 - 1, m % 100_000);
            if let Some(Val::Choice(i, _)) = s.modules.get_mut(module).and_then(|md| md.sets.get_mut(j)).map(|st| &mut st.val) {
                *i = swatch;
                s.dirty = true;
            }
        }
        Hit::Choice(m, j) => {
            s.modules[m].sets[j].val.step(if right { -1 } else { 1 });
            s.dirty = true;
        }
        Hit::Text(..) | Hit::Inside => {}
        Hit::Bind(m) => s.gui.binding = Some(m),
        Hit::Visible(m) => {
            s.modules[m].visible = !s.modules[m].visible;
            s.dirty = true;
        }
        Hit::Focus(f) => {
            if s.gui.focus != f {
                s.gui.buffer = match &f {
                    Focus::Set(m, j) => s.modules[*m].sets[*j].val.stored(),
                    Focus::Search => s.gui.search.clone(),
                    Focus::Config => s.gui.config_name.clone(),
                    Focus::Friend => s.gui.friend_name.clone(),
                    Focus::None => String::new(),
                };
                s.gui.focus = f;
            }
        }
        Hit::ConfigNew => {
            let name = core::profile_key(s.gui.config_name.trim());
            if !name.is_empty() {
                match core::save_profile(g, s, &name) {
                    Ok(()) => s.say(format!("Saved the config {name}")),
                    Err(e) => s.say(format!("Could not save it: {e}")),
                }
                s.gui.config_name.clear();
                s.gui.configs = core::profiles(g);
            }
        }
        Hit::ConfigSave(name) => match core::save_profile(g, s, &name) {
            Ok(()) => s.say(format!("Saved the config {name}")),
            Err(e) => s.say(format!("Could not save it: {e}")),
        },
        Hit::ConfigLoad(name) => {
            if core::load_profile(g, s, &name) {
                s.say(format!("Loaded the config {name}"));
            }
        }
        Hit::ConfigDelete(name) => {
            core::delete_profile(g, &name);
            s.gui.configs = core::profiles(g);
            s.say(format!("Deleted the config {name}"));
        }
        Hit::FriendAdd => {
            let name = s.gui.friend_name.trim().to_owned();
            if !name.is_empty() {
                s.friends.insert(name);
                s.gui.friend_name.clear();
                s.dirty = true;
            }
        }
        Hit::FriendRemove(name) => {
            s.friends.remove(&name);
            s.dirty = true;
        }
        Hit::Move => {
            let o = s.gui.offset;
            s.gui.moving = Some((mx - o.0, my - o.1));
        }
    }
}

/// A mouse button let go.
pub fn release(s: &mut State) {
    s.gui.drag = None;
    s.gui.moving = None;
}

pub fn wheel(s: &mut State, steps: f32) {
    s.gui.scroll -= steps * 24.0;
}

/// The character a key types, for the panel's text boxes.
fn typed(key: &str, shift: bool) -> Option<char> {
    let name = key.strip_prefix("key.keyboard.")?;
    let mut it = name.chars();
    if let (Some(c), None) = (it.next(), it.next()) {
        if c.is_ascii_alphabetic() {
            return Some(if shift { c.to_ascii_uppercase() } else { c });
        }
        if c.is_ascii_digit() {
            return Some(c);
        }
    }
    Some(match (name, shift) {
        ("space", _) => ' ',
        ("minus", false) => '-',
        ("minus", true) => '_',
        ("semicolon", _) => ':',
        ("comma", _) => ',',
        ("period", _) => '.',
        ("slash", _) => '/',
        _ => return None,
    })
}

/// A key while the panel is open; true when the panel used it.
pub fn key(g: &Game<'_>, s: &mut State, key: &str, pressed: bool) -> bool {
    if key.ends_with("shift") && (key.contains("left") || s.gui.focus != Focus::None) {
        s.gui.shift = pressed;
    }
    if !pressed {
        return false;
    }
    if let Some(m) = s.gui.binding.take() {
        let name = s.modules[m].name;
        match key {
            "key.keyboard.escape" => {}
            "key.keyboard.delete" | "key.keyboard.backspace" => {
                s.modules[m].bind.clear();
                s.say(format!("{name} unbound"));
            }
            k => {
                s.modules[m].bind = k.to_owned();
                s.say(format!("{name} bound to {}", core::key_label(k)));
            }
        }
        s.dirty = true;
        return true;
    }
    if s.gui.focus != Focus::None {
        match key {
            "key.keyboard.escape" => {
                s.gui.buffer.clear();
                s.gui.focus = Focus::None;
            }
            "key.keyboard.enter" | "key.keyboard.keypad.enter" => commit(g, s),
            "key.keyboard.backspace" => {
                s.gui.buffer.pop();
                if s.gui.focus == Focus::Search {
                    s.gui.search = s.gui.buffer.clone();
                    s.gui.scroll = 0.0;
                }
            }
            k => {
                if let Some(c) = typed(k, s.gui.shift) {
                    s.gui.buffer.push(c);
                    if s.gui.focus == Focus::Search {
                        s.gui.search = s.gui.buffer.clone();
                        s.gui.scroll = 0.0;
                    }
                }
            }
        }
        return true;
    }
    if key == "key.keyboard.escape" {
        s.gui.open = false;
        return true;
    }
    false
}
