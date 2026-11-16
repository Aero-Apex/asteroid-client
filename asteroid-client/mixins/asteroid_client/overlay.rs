// The picture side of Asteroid Client's StreamProof, in a crate that may call
// Windows: a layered window over the game, painted from a list of shapes and left
// out of every capture (WDA_EXCLUDEFROMCAPTURE, Windows 10 2004 and later).
use std::{path::PathBuf, sync::Mutex};

#[derive(Clone, Debug)]
pub enum Prim {
    Fill([f32; 4], u32),
    Quad([[f32; 2]; 4], u32),
    /// Text, x, y, rgb, size (1 is the game's), and each character's advance.
    Text(String, f32, f32, u32, f32, Vec<f32>),
}

struct Shared {
    frame: Option<(Vec<Prim>, f32)>,
    running: bool,
    quit: bool,
    status: String,
    dump: Option<PathBuf>,
}
static SHARED: Mutex<Shared> =
    Mutex::new(Shared { frame: None, running: false, quit: false, status: String::new(), dump: None });

/// Once a frame: turns the overlay on or off, and hands it what was recorded.
pub fn publish(owner: Option<isize>, on: bool, list: Vec<Prim>, scale: f32) {
    let Ok(mut sh) = SHARED.lock() else {
        return;
    };
    if !on {
        if sh.running {
            sh.quit = true;
        }
        return;
    }
    if !sh.running {
        if let Some(owner) = owner {
            sh.running = true;
            sh.quit = false;
            sh.status = "starting".to_owned();
            overlay::start(owner);
        } else {
            sh.status = "no game window to cover".to_owned();
        }
    }
    sh.frame = Some((list, scale));
}

/// What the overlay last reported.
pub fn status() -> String {
    SHARED.lock().map(|s| s.status.clone()).unwrap_or_default()
}
/// The overlay works and is hidden from capture.
pub fn hidden_ok() -> bool {
    status().starts_with("hidden")
}
/// Asks the overlay to write its next picture to `path`.
pub fn dump(path: PathBuf) {
    if let Ok(mut s) = SHARED.lock() {
        s.dump = Some(path);
    }
}

#[cfg(not(windows))]
mod overlay {
    pub fn start(_: isize) {
        if let Ok(mut s) = super::SHARED.lock() {
            s.status = "not available on this system: the mod's drawing is simply hidden".to_owned();
        }
    }
}

#[cfg(windows)]
mod overlay {
    use super::{Prim, SHARED};
    use std::{collections::HashMap, ffi::c_void, ptr::null, ptr::null_mut};

    type Hwnd = isize;
    type Hdc = isize;
    #[repr(C)]
    struct WndClass {
        style: u32,
        proc_: unsafe extern "system" fn(Hwnd, u32, usize, isize) -> isize,
        cls: i32,
        wnd: i32,
        instance: isize,
        icon: isize,
        cursor: isize,
        background: isize,
        menu: *const u16,
        name: *const u16,
    }
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Point {
        x: i32,
        y: i32,
    }
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct Rect {
        l: i32,
        t: i32,
        r: i32,
        b: i32,
    }
    #[repr(C)]
    struct Blend {
        op: u8,
        flags: u8,
        alpha: u8,
        format: u8,
    }
    #[repr(C)]
    struct BitmapInfo {
        size: u32,
        width: i32,
        height: i32,
        planes: u16,
        bits: u16,
        compression: u32,
        image: u32,
        xppm: i32,
        yppm: i32,
        used: u32,
        important: u32,
        colors: u32,
    }
    #[repr(C)]
    struct Msg {
        hwnd: Hwnd,
        message: u32,
        w: usize,
        l: isize,
        time: u32,
        pt: Point,
        private: u32,
    }

    #[link(name = "user32")]
    unsafe extern "system" {
        fn DefWindowProcW(h: Hwnd, m: u32, w: usize, l: isize) -> isize;
        fn RegisterClassW(c: *const WndClass) -> u16;
        fn CreateWindowExW(ex: u32, class: *const u16, name: *const u16, style: u32, x: i32, y: i32, w: i32, h: i32, parent: Hwnd, menu: isize, inst: isize, param: *const c_void) -> Hwnd;
        fn DestroyWindow(h: Hwnd) -> i32;
        fn ShowWindow(h: Hwnd, cmd: i32) -> i32;
        fn IsWindow(h: Hwnd) -> i32;
        fn IsWindowVisible(h: Hwnd) -> i32;
        fn IsIconic(h: Hwnd) -> i32;
        fn GetClientRect(h: Hwnd, r: *mut Rect) -> i32;
        fn ClientToScreen(h: Hwnd, p: *mut Point) -> i32;
        fn UpdateLayeredWindow(h: Hwnd, dst: Hdc, pos: *const Point, size: *const Point, src: Hdc, srcpos: *const Point, key: u32, blend: *const Blend, flags: u32) -> i32;
        fn SetWindowDisplayAffinity(h: Hwnd, a: u32) -> i32;
        fn SetLayeredWindowAttributes(h: Hwnd, key: u32, alpha: u8, flags: u32) -> i32;
        fn GetWindowLongW(h: Hwnd, index: i32) -> i32;
        fn SetWindowLongW(h: Hwnd, index: i32, value: i32) -> i32;
        fn GetWindowDisplayAffinity(h: Hwnd, a: *mut u32) -> i32;
        fn PeekMessageW(m: *mut Msg, h: Hwnd, a: u32, b: u32, remove: u32) -> i32;
        fn TranslateMessage(m: *const Msg) -> i32;
        fn DispatchMessageW(m: *const Msg) -> isize;
        fn GetDC(h: Hwnd) -> Hdc;
        fn ReleaseDC(h: Hwnd, dc: Hdc) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLastError() -> u32;
    }
    #[link(name = "gdi32")]
    unsafe extern "system" {
        fn CreateCompatibleDC(dc: Hdc) -> Hdc;
        fn DeleteDC(dc: Hdc) -> i32;
        fn CreateDIBSection(dc: Hdc, info: *const BitmapInfo, usage: u32, bits: *mut *mut c_void, section: isize, offset: u32) -> isize;
        fn SelectObject(dc: Hdc, o: isize) -> isize;
        fn DeleteObject(o: isize) -> i32;
        fn CreateFontW(h: i32, w: i32, esc: i32, ori: i32, weight: i32, i: u32, u: u32, s: u32, cs: u32, op: u32, cp: u32, q: u32, pf: u32, face: *const u16) -> isize;
        fn SetBkMode(dc: Hdc, m: i32) -> i32;
        fn SetTextColor(dc: Hdc, c: u32) -> u32;
        fn ExtTextOutW(dc: Hdc, x: i32, y: i32, o: u32, r: *const Rect, s: *const u16, n: u32, dx: *const i32) -> i32;
        fn GdiFlush() -> i32;
    }

    const WDA_EXCLUDEFROMCAPTURE: u32 = 0x11;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
    fn report(text: String) {
        if let Ok(mut s) = SHARED.lock() {
            s.status = text;
        }
    }

    /// A 32-bit top-down picture with its device context.
    struct Canvas {
        dc: Hdc,
        bmp: isize,
        bits: *mut u32,
        w: i32,
        h: i32,
    }
    impl Canvas {
        unsafe fn new(w: i32, h: i32) -> Option<Self> {
            unsafe {
                let dc = CreateCompatibleDC(0);
                let info = BitmapInfo { size: 40, width: w, height: -h, planes: 1, bits: 32, compression: 0, image: 0, xppm: 0, yppm: 0, used: 0, important: 0, colors: 0 };
                let mut bits: *mut c_void = null_mut();
                let bmp = CreateDIBSection(0, &info, 0, &mut bits, 0, 0);
                if bmp == 0 || bits.is_null() {
                    DeleteDC(dc);
                    return None;
                }
                SelectObject(dc, bmp);
                Some(Self { dc, bmp, bits: bits.cast(), w, h })
            }
        }
        fn pixels(&mut self) -> &mut [u32] {
            // Sized w * h by `new`, and only this thread touches it.
            unsafe { std::slice::from_raw_parts_mut(self.bits, (self.w * self.h) as usize) }
        }
    }
    impl Drop for Canvas {
        fn drop(&mut self) {
            unsafe {
                DeleteDC(self.dc);
                DeleteObject(self.bmp);
            }
        }
    }

    /// `argb` over a premultiplied pixel, its alpha scaled by `cover` (0..=255).
    #[inline]
    fn blend(px: &mut u32, argb: u32, cover: u32) {
        let a = ((argb >> 24) * cover) / 255;
        if a == 0 {
            return;
        }
        let inv = 255 - a;
        let ch = |shift: u32| {
            let s = ((argb >> shift) & 255) * a / 255;
            let d = ((*px >> shift) & 255) * inv / 255;
            (s + d).min(255) << shift
        };
        let out = ch(0) | ch(8) | ch(16);
        let da = ((*px >> 24) & 255) * inv / 255;
        *px = out | ((a + da).min(255) << 24);
    }

    fn fill(c: &mut Canvas, r: [f32; 4], argb: u32, s: f32) {
        let (w, h) = (c.w, c.h);
        let x0 = ((r[0] * s).round() as i32).clamp(0, w);
        let y0 = ((r[1] * s).round() as i32).clamp(0, h);
        let x1 = (((r[0] + r[2]) * s).round() as i32).clamp(0, w);
        let y1 = (((r[1] + r[3]) * s).round() as i32).clamp(0, h);
        let px = c.pixels();
        for y in y0..y1 {
            let row = &mut px[(y * w + x0) as usize..(y * w + x1) as usize];
            for p in row {
                blend(p, argb, 255);
            }
        }
    }

    fn quad(c: &mut Canvas, q: [[f32; 2]; 4], argb: u32, s: f32) {
        let (w, h) = (c.w, c.h);
        let p = q.map(|v| [v[0] * s, v[1] * s]);
        let lo = p.iter().map(|v| v[1]).fold(f32::MAX, f32::min).floor().max(0.0) as i32;
        let hi = p.iter().map(|v| v[1]).fold(f32::MIN, f32::max).ceil().min(h as f32) as i32;
        let px = c.pixels();
        for y in lo..hi {
            let fy = y as f32 + 0.5;
            let (mut a, mut b) = (f32::MAX, f32::MIN);
            for i in 0..4 {
                let (u, v) = (p[i], p[(i + 1) % 4]);
                if (u[1] <= fy) != (v[1] <= fy) {
                    let x = u[0] + (fy - u[1]) / (v[1] - u[1]) * (v[0] - u[0]);
                    a = a.min(x);
                    b = b.max(x);
                }
            }
            if a > b {
                continue;
            }
            // A thin line still covers a pixel of each row it crosses.
            let x0 = (a.floor() as i32).clamp(0, w);
            let x1 = ((b.ceil() as i32).max(a.floor() as i32 + 1)).clamp(0, w);
            for x in x0..x1 {
                blend(&mut px[(y * w + x) as usize], argb, 255);
            }
        }
    }

    struct Painter {
        canvas: Canvas,
        mask: Canvas,
        fonts: HashMap<i32, isize>,
    }
    impl Painter {
        fn font(&mut self, h: i32) -> isize {
            *self.fonts.entry(h).or_insert_with(|| unsafe {
                // ANTIALIASED_QUALITY: grey coverage, read back as alpha.
                CreateFontW(-h, 0, 0, 0, 500, 0, 0, 0, 1, 0, 0, 4, 0, wide("Segoe UI").as_ptr())
            })
        }
        fn text(&mut self, t: &str, x: f32, y: f32, rgb: u32, size: f32, adv: &[f32], s: f32) {
            let h = (size * 9.4 * s).round().max(6.0) as i32;
            let font = self.font(h);
            let mut units: Vec<u16> = Vec::with_capacity(t.len());
            let mut dx: Vec<i32> = Vec::with_capacity(t.len());
            let mut carry = 0.0_f32;
            for (i, ch) in t.chars().enumerate() {
                let mut b = [0u16; 2];
                let e = ch.encode_utf16(&mut b);
                let want = adv.get(i).copied().unwrap_or(4.0) * s + carry;
                let step = want.round();
                carry = want - step;
                for (j, u) in e.iter().enumerate() {
                    units.push(*u);
                    dx.push(if j == 0 { step as i32 } else { 0 });
                }
            }
            let (px, py) = ((x * s).round() as i32, (y * s).round() as i32 - h / 8);
            let tw: i32 = dx.iter().sum::<i32>() + h;
            let (mw, mh) = (self.mask.w, self.mask.h);
            let (x0, x1) = (px.clamp(0, mw), (px + tw).clamp(0, mw));
            let (y0, y1) = (py.clamp(0, mh), (py + h + h / 3).clamp(0, mh));
            if x0 >= x1 || y0 >= y1 {
                return;
            }
            unsafe {
                SelectObject(self.mask.dc, font);
                ExtTextOutW(self.mask.dc, px, py, 0, null(), units.as_ptr(), units.len() as u32, null());
                GdiFlush();
            }
            let color = 0xFF00_0000 | rgb;
            // Read and clear the mask, then a soft shadow and the text.
            let (cw, ch) = (self.canvas.w, self.canvas.h);
            let shadow = (s / 2.0).round().max(1.0) as i32;
            let m = self.mask.pixels();
            let c = self.canvas.pixels();
            for yy in y0..y1 {
                for xx in x0..x1 {
                    let i = (yy * mw + xx) as usize;
                    let cover = m[i] & 255;
                    if cover == 0 {
                        continue;
                    }
                    m[i] = 0;
                    if xx + shadow < cw && yy + shadow < ch {
                        blend(&mut c[((yy + shadow) * cw + xx + shadow) as usize], 0xB000_0000, cover);
                    }
                    blend(&mut c[(yy * cw + xx) as usize], color, cover);
                }
            }
        }
        fn paint(&mut self, prims: &[Prim], s: f32) {
            self.canvas.pixels().fill(0);
            for p in prims {
                match p {
                    Prim::Fill(r, c) => fill(&mut self.canvas, *r, *c, s),
                    Prim::Quad(q, c) => quad(&mut self.canvas, *q, *c, s),
                    Prim::Text(t, x, y, rgb, k, adv) => self.text(t, *x, *y, *rgb, *k, adv, s),
                }
            }
        }
    }

    unsafe fn new_painter(w: i32, h: i32) -> Option<Painter> {
        unsafe {
            let canvas = Canvas::new(w, h)?;
            let mask = Canvas::new(w, h)?;
            SetBkMode(mask.dc, 1);
            SetTextColor(mask.dc, 0x00FF_FFFF);
            Some(Painter { canvas, mask, fonts: HashMap::new() })
        }
    }

    fn write_bmp(path: &std::path::Path, c: &mut Canvas) -> std::io::Result<()> {
        let (w, h) = (c.w as u32, c.h as u32);
        let size = w * h * 4;
        let mut out = Vec::with_capacity(size as usize + 54);
        out.extend_from_slice(b"BM");
        out.extend_from_slice(&(54 + size).to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&54u32.to_le_bytes());
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&c.w.to_le_bytes());
        out.extend_from_slice(&(-c.h).to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&[0; 24]);
        for p in c.pixels().iter() {
            out.extend_from_slice(&p.to_le_bytes());
        }
        std::fs::write(path, out)
    }

    pub fn start(owner: isize) {
        let _ = std::thread::Builder::new().name("asteroid-stream".to_owned()).spawn(move || unsafe { run(owner) });
    }

    unsafe fn run(owner: Hwnd) {
        unsafe {
            let class = wide("AsteroidStreamOverlay");
            let wc = WndClass { style: 0, proc_: DefWindowProcW, cls: 0, wnd: 0, instance: 0, icon: 0, cursor: 0, background: 0, menu: null(), name: class.as_ptr() };
            RegisterClassW(&wc);
            // Layered, click-through, no taskbar button, never active; owned, so it
            // keeps just above the game and goes away when the game is minimized.
            let ex = 0x0008_0000 | 0x0000_0020 | 0x0000_0080 | 0x0800_0000;
            let hwnd = CreateWindowExW(ex, class.as_ptr(), wide("").as_ptr(), 0x8000_0000, 0, 0, 1, 1, owner, 0, 0, null());
            if hwnd == 0 {
                report("could not create the overlay window".to_owned());
                if let Ok(mut s) = SHARED.lock() {
                    s.running = false;
                }
                return;
            }
            // Windows only takes the affinity on a layered window that has had its
            // attributes set (else error 8), and such a window can't be painted per
            // pixel until its layered bit is reset, which keeps the affinity.
            const EXSTYLE: i32 = -20;
            const LAYERED: i32 = 0x0008_0000;
            SetLayeredWindowAttributes(hwnd, 0, 255, 2);
            let set = SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) != 0;
            let error = if set { 0 } else { GetLastError() };
            let style = GetWindowLongW(hwnd, EXSTYLE);
            SetWindowLongW(hwnd, EXSTYLE, style & !LAYERED);
            SetWindowLongW(hwnd, EXSTYLE, style | LAYERED);
            let mut got = 0u32;
            let hidden = set && GetWindowDisplayAffinity(hwnd, &mut got) != 0 && got == WDA_EXCLUDEFROMCAPTURE;
            report(if hidden {
                "hidden from capture".to_owned()
            } else {
                format!("Windows refused to hide the overlay (error {error}, needs Windows 10 2004 or later): it stays off, so the HUD is not shown")
            });
            let mut painter: Option<Painter> = None;
            let mut shown = false;
            let screen = GetDC(0);
            loop {
                let mut msg: Msg = std::mem::zeroed();
                while PeekMessageW(&mut msg, 0, 0, 0, 1) != 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                let (frame, quit, dump) = match SHARED.lock() {
                    Ok(mut s) => (s.frame.take(), s.quit, s.dump.take()),
                    Err(_) => (None, true, None),
                };
                if quit || IsWindow(owner) == 0 {
                    break;
                }
                let mut r = Rect::default();
                GetClientRect(owner, &mut r);
                let (w, h) = ((r.r - r.l).max(1), (r.b - r.t).max(1));
                let mut at = Point::default();
                ClientToScreen(owner, &mut at);
                if painter.as_ref().is_none_or(|p| p.canvas.w != w || p.canvas.h != h) {
                    painter = new_painter(w, h);
                }
                let Some(p) = painter.as_mut() else {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                };
                let fresh = frame.is_some();
                if let Some((prims, scale)) = frame {
                    p.paint(&prims, scale);
                }
                if let Some(path) = dump {
                    let done = write_bmp(&path, &mut p.canvas);
                    report(match done {
                        Ok(()) => format!("{} (picture written)", if hidden { "hidden from capture" } else { "not hidden" }),
                        Err(e) => format!("{} (picture failed: {e})", if hidden { "hidden from capture" } else { "not hidden" }),
                    });
                }
                if fresh && hidden {
                    let size = Point { x: w, y: h };
                    let src = Point::default();
                    let bf = Blend { op: 0, flags: 0, alpha: 255, format: 1 };
                    UpdateLayeredWindow(hwnd, screen, &at, &size, p.canvas.dc, &src, 0, &bf, 2);
                }
                let visible = hidden && IsWindowVisible(owner) != 0 && IsIconic(owner) == 0;
                if visible != shown {
                    // SW_SHOWNOACTIVATE or SW_HIDE.
                    ShowWindow(hwnd, if visible { 4 } else { 0 });
                    shown = visible;
                }
                std::thread::sleep(std::time::Duration::from_millis(if fresh { 4 } else { 8 }));
            }
            if let Some(p) = painter.take() {
                for f in p.fonts.values() {
                    DeleteObject(*f);
                }
            }
            ReleaseDC(0, screen);
            DestroyWindow(hwnd);
            if let Ok(mut s) = SHARED.lock() {
                s.running = false;
                s.quit = false;
                s.status = String::new();
            }
        }
    }
}
