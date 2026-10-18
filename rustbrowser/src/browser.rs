//! Browser window: toolbar, address bar, history, scrolling, zoom and alert dialogs.
//!
//! Input arrives through minifb. Key presses are queued from minifb's event callback rather than
//! polled per frame so quick taps are never missed.

use crate::css::Color;
use crate::font::FontStore;
use crate::layout::Rect;
use crate::page::{Action, Page};
use crate::paint::{Canvas, render};
use crate::style::Family;
use minifb::{CursorStyle, InputCallback, Key, MouseButton, MouseMode, Window, WindowOptions};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use url::Url;

const TOOLBAR_H: usize = 44;
const UI_TEXT: f32 = 14.0;

#[derive(Clone, Copy)]
struct KeyEvent {
    key: Key,
    ctrl: bool,
    alt: bool,
    shift: bool,
}

#[derive(Default)]
struct InputState {
    chars: Vec<char>,
    keys: Vec<KeyEvent>,
    ctrl: bool,
    alt: bool,
    shift: bool,
}

/// minifb input callback that records characters and key presses with the modifier state at that moment.
struct InputQueue(Rc<RefCell<InputState>>);

impl InputCallback for InputQueue {
    fn add_char(&mut self, c: u32) {
        let mut s = self.0.borrow_mut();
        if let Some(ch) = char::from_u32(c) {
            if !ch.is_control() && !s.ctrl && !s.alt {
                s.chars.push(ch);
            }
        }
    }

    fn set_key_state(&mut self, key: Key, down: bool) {
        let mut s = self.0.borrow_mut();
        match key {
            Key::LeftCtrl | Key::RightCtrl | Key::LeftSuper | Key::RightSuper => s.ctrl = down,
            Key::LeftAlt | Key::RightAlt => s.alt = down,
            Key::LeftShift | Key::RightShift => s.shift = down,
            _ if down => {
                let ev = KeyEvent {
                    key,
                    ctrl: s.ctrl,
                    alt: s.alt,
                    shift: s.shift,
                };
                s.keys.push(ev);
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Button {
    Back,
    Forward,
    Reload,
}

/// The browser window and its state.
pub struct Browser {
    window: Window,
    buf: Vec<u32>,
    width: usize,
    height: usize,
    fonts: FontStore,
    page: Page,
    history: Vec<(Url, f32)>,
    hist_idx: usize,
    scroll: f32,
    zoom: f32,
    address: String,
    editing: bool,
    select_all: bool,
    input: Rc<RefCell<InputState>>,
    hover_link: Option<String>,
    alerts: Vec<String>,
    hover_button: Option<Button>,
    status: Option<String>,
    mouse_was_down: bool,
    layout_key: (usize, usize, u32),
    dirty: bool,
}

/// Turns address bar text into a URL: full URLs, local paths, bare domains, or a DuckDuckGo search.
pub fn parse_input(text: &str) -> Option<Url> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(u) = Url::parse(t) {
        if matches!(u.scheme(), "http" | "https" | "file" | "about") {
            return Some(u);
        }
    }
    let p = Path::new(t);
    if p.exists() {
        if let Ok(abs) = p.canonicalize() {
            if let Ok(u) = Url::from_file_path(&abs) {
                return Some(u);
            }
        }
    }
    if !t.contains(char::is_whitespace) {
        if t.starts_with("localhost") || t.starts_with("127.0.0.1") {
            return Url::parse(&format!("http://{}", t)).ok();
        }
        if t.contains('.') {
            return Url::parse(&format!("https://{}", t)).ok();
        }
    }
    Url::parse_with_params("https://lite.duckduckgo.com/lite/", &[("q", t)]).ok()
}

impl Browser {
    pub fn new(start: Url, fonts: FontStore) -> Result<Browser, String> {
        let (width, height) = (1100, 800);
        let mut window = Window::new(
            "RustBrowser",
            width,
            height,
            WindowOptions {
                resize: true,
                ..WindowOptions::default()
            },
        )
        .map_err(|e| e.to_string())?;
        window.set_target_fps(60);
        let input = Rc::new(RefCell::new(InputState::default()));
        window.set_input_callback(Box::new(InputQueue(input.clone())));
        let page = Page::from_html(start.clone(), "");
        let mut b = Browser {
            window,
            buf: vec![0; width * height],
            width,
            height,
            fonts,
            page,
            history: Vec::new(),
            hist_idx: 0,
            scroll: 0.0,
            zoom: 1.0,
            address: start.to_string(),
            editing: false,
            select_all: false,
            input,
            hover_link: None,
            alerts: Vec::new(),
            hover_button: None,
            status: None,
            mouse_was_down: false,
            layout_key: (0, 0, 0),
            dirty: true,
        };
        b.navigate(start, true);
        Ok(b)
    }

    fn viewport_h(&self) -> f32 {
        self.height.saturating_sub(TOOLBAR_H) as f32
    }

    fn max_scroll(&self) -> f32 {
        (self.page.height * self.zoom - self.viewport_h()).max(0.0)
    }

    /// Lays the page out again if the window size, zoom or DOM changed.
    fn relayout(&mut self) {
        let key = (self.width, self.height, (self.zoom * 100.0) as u32);
        if key == self.layout_key && !self.page.needs_layout() {
            return;
        }
        self.layout_key = key;
        let w = self.width as f32 / self.zoom;
        let h = self.viewport_h() / self.zoom;
        self.page.layout(w, h, &self.fonts);
        self.scroll = self.scroll.min(self.max_scroll());
        self.page.set_scroll(self.scroll / self.zoom);
        self.dirty = true;
    }

    /// Loads a URL, or scrolls to a fragment when only the fragment changed.
    fn navigate(&mut self, url: Url, push: bool) {
        let same_doc = {
            let mut a = url.clone();
            let mut b = self.page.url.clone();
            a.set_fragment(None);
            b.set_fragment(None);
            a == b && url.fragment().is_some() && !self.history.is_empty()
        };
        if push {
            if let Some(entry) = self.history.get_mut(self.hist_idx) {
                entry.1 = self.scroll;
            }
            self.history
                .truncate(if self.history.is_empty() { 0 } else { self.hist_idx + 1 });
            self.history.push((url.clone(), 0.0));
            self.hist_idx = self.history.len() - 1;
        }
        self.address = url.to_string();
        self.editing = false;
        if !same_doc {
            self.status = Some(format!("Loading {}…", url));
            self.dirty = true;
            self.draw();
            self.alerts.clear();
            self.page = Page::load(url.clone());
            self.layout_key = (0, 0, 0);
            self.window.set_title(&if self.page.title.is_empty() {
                "RustBrowser".to_string()
            } else {
                format!("{} - RustBrowser", self.page.title)
            });
            if push && self.page.url != url {
                self.history[self.hist_idx].0 = self.page.url.clone();
            }
            self.address = self.page.url.to_string();
        } else {
            self.page.url = url.clone();
        }
        self.status = None;
        self.hover_link = None;
        self.relayout();
        self.scroll = url
            .fragment()
            .and_then(|f| self.page.anchors.get(f).copied())
            .map(|y| (y * self.zoom).min(self.max_scroll()))
            .unwrap_or(0.0);
        self.dirty = true;
    }

    fn go_to_history(&mut self, idx: usize) {
        if idx >= self.history.len() || idx == self.hist_idx {
            return;
        }
        self.history[self.hist_idx].1 = self.scroll;
        self.hist_idx = idx;
        let (u, scroll) = self.history[idx].clone();
        self.navigate(u, false);
        self.scroll = scroll.min(self.max_scroll());
    }

    fn go_back(&mut self) {
        if self.hist_idx > 0 {
            self.go_to_history(self.hist_idx - 1);
        }
    }

    fn go_forward(&mut self) {
        self.go_to_history(self.hist_idx + 1);
    }

    fn reload(&mut self) {
        let u = self.page.url.clone();
        let scroll = self.scroll;
        self.page.url = Url::parse("about:blank").unwrap();
        self.navigate(u, false);
        self.scroll = scroll.min(self.max_scroll());
    }

    fn open(&mut self, u: Url) {
        if matches!(u.scheme(), "http" | "https" | "file" | "about") {
            self.navigate(u, true);
        } else {
            self.status = Some(format!("Can't open {} links", u.scheme()));
            self.dirty = true;
        }
    }

    /// Applies navigation, scrolling, focus and dialog requests made by page scripts.
    fn handle_requests(&mut self) {
        let req = self.page.take_requests();
        if !req.alerts.is_empty() {
            self.alerts.extend(req.alerts);
            self.dirty = true;
        }
        if let Some(f) = req.focus {
            self.page.set_focus(f);
        }
        if let Some(y) = req.scroll_to {
            let target = (y * self.zoom).clamp(0.0, self.max_scroll());
            if target != self.scroll {
                self.scroll = target;
                self.page.set_scroll(self.scroll / self.zoom);
                self.dirty = true;
            }
        }
        if let Some(u) = req.url_update {
            self.page.url = u.clone();
            self.address = u.to_string();
            if let Some(entry) = self.history.get_mut(self.hist_idx) {
                entry.0 = u;
            }
            self.dirty = true;
        }
        if let Some(n) = req.history {
            match n.signum() {
                -1 => self.go_back(),
                1 => self.go_forward(),
                _ => self.reload(),
            }
            return;
        }
        if let Some(u) = req.navigate {
            self.open(u);
        }
    }

    fn mods(&self) -> (bool, bool, bool) {
        let st = self.input.borrow();
        (st.ctrl, st.shift, st.alt)
    }

    fn page_key(&mut self, key: &str) -> bool {
        self.page.set_scroll(self.scroll / self.zoom);
        let mods = self.mods();
        let (handled, action) = self.page.key(key, mods);
        if handled {
            self.dirty = true;
        }
        if let Action::Navigate(u) = action {
            self.open(u);
            return true;
        }
        handled
    }

    fn button_rect(b: Button) -> Rect {
        let i = match b {
            Button::Back => 0.0,
            Button::Forward => 1.0,
            Button::Reload => 2.0,
        };
        Rect {
            x: 8.0 + i * 36.0,
            y: 8.0,
            w: 32.0,
            h: 28.0,
        }
    }

    fn address_rect(&self) -> Rect {
        Rect {
            x: 120.0,
            y: 8.0,
            w: (self.width as f32 - 132.0).max(40.0),
            h: 28.0,
        }
    }

    fn scroll_by(&mut self, dy: f32) {
        let new = (self.scroll + dy).clamp(0.0, self.max_scroll());
        if new != self.scroll {
            self.scroll = new;
            self.page.set_scroll(self.scroll / self.zoom);
            self.dirty = true;
        }
    }

    fn set_zoom(&mut self, z: f32) {
        let z = z.clamp(0.5, 3.0);
        if (z - self.zoom).abs() > 0.001 {
            let ratio = z / self.zoom;
            self.zoom = z;
            self.scroll *= ratio;
            self.relayout();
            self.status = Some(format!("Zoom {}%", (z * 100.0).round()));
        }
    }

    /// Handles one key press. Returns true if it navigated, which ends input processing for the frame.
    fn handle_key(&mut self, ev: KeyEvent) -> bool {
        let KeyEvent { key, ctrl, alt, shift } = ev;
        match key {
            Key::F5 => {
                self.reload();
                return true;
            }
            Key::R if ctrl => {
                self.reload();
                return true;
            }
            Key::F6 => {
                self.editing = true;
                self.select_all = true;
                self.dirty = true;
                return false;
            }
            Key::L if ctrl => {
                self.editing = true;
                self.select_all = true;
                self.dirty = true;
                return false;
            }
            Key::Equal | Key::NumPadPlus if ctrl => {
                self.set_zoom(self.zoom + 0.1);
                return false;
            }
            Key::Minus | Key::NumPadMinus if ctrl => {
                self.set_zoom(self.zoom - 0.1);
                return false;
            }
            Key::Key0 | Key::NumPad0 if ctrl => {
                self.set_zoom(1.0);
                return false;
            }
            Key::Left if alt => {
                self.go_back();
                return true;
            }
            Key::Right if alt => {
                self.go_forward();
                return true;
            }
            _ => {}
        }
        if self.editing {
            match key {
                Key::Backspace => {
                    if self.select_all {
                        self.address.clear();
                        self.select_all = false;
                    } else if ctrl {
                        let kept = self
                            .address
                            .trim_end()
                            .trim_end_matches(|c: char| c.is_alphanumeric())
                            .len();
                        self.address.truncate(kept);
                    } else {
                        self.address.pop();
                    }
                    self.dirty = true;
                }
                Key::Enter | Key::NumPadEnter => {
                    if let Some(u) = parse_input(&self.address) {
                        self.navigate(u, true);
                        return true;
                    }
                }
                Key::Escape => {
                    self.editing = false;
                    self.address = self.page.url.to_string();
                    self.dirty = true;
                }
                Key::A if ctrl => {
                    self.select_all = true;
                    self.dirty = true;
                }
                _ => {}
            }
            return false;
        }
        if let Some(name) = dom_key(key) {
            let before = self.page.url.clone();
            if self.page_key(name) || self.page.url != before {
                return false;
            }
        }
        let page = self.viewport_h() * 0.9;
        match key {
            Key::Down => self.scroll_by(48.0),
            Key::Up => self.scroll_by(-48.0),
            Key::PageDown => self.scroll_by(page),
            Key::PageUp => self.scroll_by(-page),
            Key::Space => {
                let _ = shift;
            }
            Key::Home => self.scroll_by(-f32::MAX / 2.0),
            Key::End => self.scroll_by(f32::MAX / 2.0),
            Key::Backspace => {
                self.go_back();
                return true;
            }
            _ => {}
        }
        false
    }

    fn handle_input(&mut self) {
        let (typed, events, ctrl) = {
            let mut st = self.input.borrow_mut();
            (std::mem::take(&mut st.chars), std::mem::take(&mut st.keys), st.ctrl)
        };
        if !self.alerts.is_empty() {
            let dismiss = events
                .iter()
                .any(|e| matches!(e.key, Key::Enter | Key::NumPadEnter | Key::Escape | Key::Space));
            let down = self.window.get_mouse_down(MouseButton::Left);
            let clicked = down && !self.mouse_was_down;
            self.mouse_was_down = down;
            if dismiss || clicked {
                self.alerts.remove(0);
                self.dirty = true;
            }
            return;
        }
        if self.editing {
            for c in typed {
                if self.select_all {
                    self.address.clear();
                    self.select_all = false;
                }
                self.address.push(c);
                self.dirty = true;
            }
        } else {
            for c in typed {
                let handled = self.page_key(&c.to_string());
                if !handled && c == ' ' {
                    let shift = self.input.borrow().shift;
                    let page = self.viewport_h() * 0.9;
                    self.scroll_by(if shift { -page } else { page });
                }
            }
        }
        for ev in events {
            if self.handle_key(ev) {
                return;
            }
        }

        if let Some((_, dy)) = self.window.get_scroll_wheel() {
            if ctrl {
                self.set_zoom(self.zoom + dy.signum() * 0.1);
            } else {
                self.scroll_by(-dy * 40.0);
            }
        }

        let mouse = self.window.get_mouse_pos(MouseMode::Discard);
        let down = self.window.get_mouse_down(MouseButton::Left);
        let clicked = down && !self.mouse_was_down;
        self.mouse_was_down = down;

        let mut hover_link = None;
        let mut hover_button = None;
        let mut cursor = CursorStyle::Arrow;
        if let Some((mx, my)) = mouse {
            if (my as usize) < TOOLBAR_H {
                for b in [Button::Back, Button::Forward, Button::Reload] {
                    if Self::button_rect(b).contains(mx, my) {
                        hover_button = Some(b);
                    }
                }
                if self.address_rect().contains(mx, my) {
                    cursor = CursorStyle::Ibeam;
                }
            } else {
                let px = mx / self.zoom;
                let py = (my - TOOLBAR_H as f32 + self.scroll) / self.zoom;
                if let Some(h) = self.page.link_at(px, py) {
                    hover_link = Some(h.to_string());
                    cursor = CursorStyle::ClosedHand;
                }
            }
            if clicked {
                if let Some(b) = hover_button {
                    match b {
                        Button::Back => return self.go_back(),
                        Button::Forward => return self.go_forward(),
                        Button::Reload => return self.reload(),
                    }
                } else if self.address_rect().contains(mx, my) {
                    if !self.editing {
                        self.editing = true;
                        self.select_all = true;
                    } else {
                        self.select_all = false;
                    }
                    self.dirty = true;
                } else if (my as usize) >= TOOLBAR_H {
                    if self.editing {
                        self.editing = false;
                        self.address = self.page.url.to_string();
                        self.dirty = true;
                    }
                    if self.status.is_some() {
                        self.status = None;
                        self.dirty = true;
                    }
                    self.page.set_scroll(self.scroll / self.zoom);
                    let px = mx / self.zoom;
                    let py = (my - TOOLBAR_H as f32 + self.scroll) / self.zoom;
                    let client = (mx / self.zoom, (my - TOOLBAR_H as f32) / self.zoom);
                    let mods = self.mods();
                    let action = self.page.click(px, py, client, mods);
                    self.dirty = true;
                    if let Action::Navigate(u) = action {
                        return self.open(u);
                    }
                }
            }
        }
        self.window.set_cursor_style(cursor);
        if hover_link != self.hover_link || hover_button != self.hover_button {
            self.hover_link = hover_link;
            self.hover_button = hover_button;
            self.dirty = true;
        }
    }

    fn draw(&mut self) {
        let (w, h) = (self.width, self.height);
        if self.buf.len() != w * h {
            self.buf = vec![0; w * h];
        }
        let ar = self.address_rect();
        let fonts = &self.fonts;
        let mut canvas = Canvas::new(&mut self.buf, w, h);

        canvas.set_clip(0.0, TOOLBAR_H as f32, w as f32, h as f32 - TOOLBAR_H as f32);
        canvas.fill(self.page.bg);
        render(
            &self.page.cmds,
            &mut canvas,
            0.0,
            TOOLBAR_H as f32 - self.scroll,
            self.zoom,
            fonts,
        );

        let view_h = h as f32 - TOOLBAR_H as f32;
        let total = self.page.height * self.zoom;
        if total > view_h + 1.0 {
            let thumb = (view_h * view_h / total).max(28.0);
            let max_scroll = total - view_h;
            let y = TOOLBAR_H as f32 + (self.scroll / max_scroll) * (view_h - thumb);
            canvas.fill_rect(
                Rect {
                    x: w as f32 - 9.0,
                    y: y + 2.0,
                    w: 6.0,
                    h: thumb - 4.0,
                },
                Color {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 90,
                },
            );
        }

        let status = self.status.clone().or_else(|| {
            self.hover_link
                .as_ref()
                .map(|l| self.page.resolve(l).map(|u| u.to_string()).unwrap_or_else(|| l.clone()))
        });
        if let Some(text) = status {
            let ui = fonts.font_for(Family::Sans, false, false);
            let size = 12.5;
            let tw = fonts.measure(&text, ui, size).min(w as f32 * 0.7);
            let r = Rect {
                x: 0.0,
                y: h as f32 - 22.0,
                w: tw + 16.0,
                h: 22.0,
            };
            canvas.fill_rect(r, Color::rgb(0xf3, 0xf3, 0xf3));
            canvas.fill_rect(
                Rect {
                    x: r.x,
                    y: r.y,
                    w: r.w,
                    h: 1.0,
                },
                Color::rgb(0xc4, 0xc4, 0xc4),
            );
            canvas.fill_rect(
                Rect {
                    x: r.x + r.w - 1.0,
                    y: r.y,
                    w: 1.0,
                    h: r.h,
                },
                Color::rgb(0xc4, 0xc4, 0xc4),
            );
            canvas.set_clip(r.x, r.y, r.w - 8.0, r.h);
            canvas.draw_text(
                8.0,
                h as f32 - 7.0,
                &text,
                ui,
                size,
                Color::rgb(0x30, 0x30, 0x30),
                fonts,
            );
        }

        canvas.set_clip(0.0, 0.0, w as f32, TOOLBAR_H as f32);
        canvas.fill(Color::rgb(0xec, 0xeb, 0xe8));
        canvas.fill_rect(
            Rect {
                x: 0.0,
                y: TOOLBAR_H as f32 - 1.0,
                w: w as f32,
                h: 1.0,
            },
            Color::rgb(0xc8, 0xc6, 0xc0),
        );
        for b in [Button::Back, Button::Forward, Button::Reload] {
            let r = Self::button_rect(b);
            let enabled = match b {
                Button::Back => self.hist_idx > 0,
                Button::Forward => self.hist_idx + 1 < self.history.len(),
                Button::Reload => true,
            };
            if enabled && self.hover_button == Some(b) {
                canvas.fill_rect(r, Color::rgb(0xd9, 0xd7, 0xd2));
            }
            let ic = if enabled {
                Color::rgb(0x33, 0x33, 0x33)
            } else {
                Color::rgb(0xb0, 0xae, 0xa8)
            };
            let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
            match b {
                Button::Back => {
                    canvas.fill_triangle([(cx - 7.0, cy), (cx, cy - 7.0), (cx, cy + 7.0)], ic);
                    canvas.fill_rect(
                        Rect {
                            x: cx - 1.0,
                            y: cy - 2.0,
                            w: 8.0,
                            h: 4.0,
                        },
                        ic,
                    );
                }
                Button::Forward => {
                    canvas.fill_triangle([(cx + 7.0, cy), (cx, cy - 7.0), (cx, cy + 7.0)], ic);
                    canvas.fill_rect(
                        Rect {
                            x: cx - 7.0,
                            y: cy - 2.0,
                            w: 8.0,
                            h: 4.0,
                        },
                        ic,
                    );
                }
                Button::Reload => {
                    canvas.fill_circle(cx, cy, 7.0, ic, false);
                    canvas.fill_rect(
                        Rect {
                            x: cx + 1.0,
                            y: cy - 9.0,
                            w: 8.0,
                            h: 8.0,
                        },
                        Color::rgb(0xec, 0xeb, 0xe8),
                    );
                    if self.hover_button == Some(b) {
                        canvas.fill_rect(
                            Rect {
                                x: cx + 1.0,
                                y: cy - 9.0,
                                w: 8.0,
                                h: 8.0,
                            },
                            Color::rgb(0xd9, 0xd7, 0xd2),
                        );
                    }
                    canvas.fill_triangle([(cx + 1.0, cy - 11.0), (cx + 1.0, cy - 3.0), (cx + 6.0, cy - 7.0)], ic);
                }
            }
        }

        let border = if self.editing {
            Color::rgb(0x2f, 0x6f, 0xd6)
        } else {
            Color::rgb(0xb8, 0xb6, 0xb0)
        };
        let bw = if self.editing { 2.0 } else { 1.0 };
        canvas.fill_rect(ar, border);
        let inner = Rect {
            x: ar.x + bw,
            y: ar.y + bw,
            w: ar.w - 2.0 * bw,
            h: ar.h - 2.0 * bw,
        };
        canvas.fill_rect(inner, Color::WHITE);
        let ui = fonts.font_for(Family::Sans, false, false);
        let tw = fonts.measure(&self.address, ui, UI_TEXT);
        let avail = inner.w - 16.0;
        let shift = if self.editing && tw > avail { avail - tw } else { 0.0 };
        let tx = inner.x + 8.0 + shift;
        let baseline = inner.y + inner.h / 2.0 + UI_TEXT * 0.36;
        canvas.set_clip(inner.x + 4.0, inner.y, inner.w - 8.0, inner.h);
        if self.editing && self.select_all && !self.address.is_empty() {
            canvas.fill_rect(
                Rect {
                    x: tx,
                    y: inner.y + 4.0,
                    w: tw,
                    h: inner.h - 8.0,
                },
                Color::rgb(0xb4, 0xd5, 0xfe),
            );
        }
        canvas.draw_text(
            tx,
            baseline,
            &self.address,
            ui,
            UI_TEXT,
            Color::rgb(0x1a, 0x1a, 0x1a),
            fonts,
        );
        if self.editing && !self.select_all {
            canvas.fill_rect(
                Rect {
                    x: tx + tw + 1.0,
                    y: inner.y + 5.0,
                    w: 1.0,
                    h: inner.h - 10.0,
                },
                Color::BLACK,
            );
        }
        canvas.reset_clip();

        if let Some(msg) = self.alerts.first() {
            canvas.fill_rect(
                Rect {
                    x: 0.0,
                    y: 0.0,
                    w: w as f32,
                    h: h as f32,
                },
                Color {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 90,
                },
            );
            let size = 15.0;
            let bw = (w as f32 - 48.0).min(480.0);
            let inner = bw - 40.0;
            let mut lines: Vec<String> = Vec::new();
            for para in msg.split('\n') {
                let mut line = String::new();
                for word in para.split(' ') {
                    let candidate = if line.is_empty() {
                        word.to_string()
                    } else {
                        format!("{} {}", line, word)
                    };
                    if fonts.measure(&candidate, ui, size) > inner && !line.is_empty() {
                        lines.push(std::mem::take(&mut line));
                        line = word.to_string();
                    } else {
                        line = candidate;
                    }
                }
                lines.push(line);
            }
            lines.truncate(14);
            let lh = size * 1.4;
            let bh = 72.0 + lines.len() as f32 * lh + 44.0;
            let bx = (w as f32 - bw) / 2.0;
            let by = (h as f32 - bh) / 3.0;
            canvas.fill_rect(
                Rect {
                    x: bx - 1.0,
                    y: by - 1.0,
                    w: bw + 2.0,
                    h: bh + 2.0,
                },
                Color::rgb(0xa0, 0xa0, 0xa0),
            );
            canvas.fill_rect(
                Rect {
                    x: bx,
                    y: by,
                    w: bw,
                    h: bh,
                },
                Color::WHITE,
            );
            let origin = self
                .page
                .url
                .host_str()
                .map(|h| format!("{} says", h))
                .unwrap_or_else(|| "This page says".to_string());
            let bold = fonts.font_for(Family::Sans, true, false);
            canvas.draw_text(
                bx + 20.0,
                by + 34.0,
                &origin,
                bold,
                size,
                Color::rgb(0x20, 0x20, 0x20),
                fonts,
            );
            for (i, line) in lines.iter().enumerate() {
                canvas.draw_text(
                    bx + 20.0,
                    by + 66.0 + i as f32 * lh,
                    line,
                    ui,
                    size,
                    Color::rgb(0x30, 0x30, 0x30),
                    fonts,
                );
            }
            let ok = Rect {
                x: bx + bw - 92.0,
                y: by + bh - 46.0,
                w: 72.0,
                h: 30.0,
            };
            canvas.fill_rect(ok, Color::rgb(0x2f, 0x6f, 0xd6));
            let tw = fonts.measure("OK", bold, 14.0);
            canvas.draw_text(
                ok.x + (ok.w - tw) / 2.0,
                ok.y + 20.0,
                "OK",
                bold,
                14.0,
                Color::WHITE,
                fonts,
            );
        }

        let _ = self.window.update_with_buffer(&self.buf, w, h);
        self.dirty = false;
    }

    /// Runs the event loop until the window closes.
    pub fn run(&mut self) {
        while self.window.is_open() {
            let (w, h) = self.window.get_size();
            if w > 0 && h > 0 && (w, h) != (self.width, self.height) {
                self.width = w;
                self.height = h;
                self.relayout();
                self.dirty = true;
            }
            self.handle_input();
            self.page.tick();
            self.handle_requests();
            if self.page.needs_layout() {
                self.relayout();
            }
            if self.dirty {
                self.draw();
            } else {
                self.window.update();
            }
        }
    }
}

/// DOM `key` value for non-character keys; character keys arrive through the text callback.
fn dom_key(k: Key) -> Option<&'static str> {
    Some(match k {
        Key::Enter | Key::NumPadEnter => "Enter",
        Key::Backspace => "Backspace",
        Key::Tab => "Tab",
        Key::Escape => "Escape",
        Key::Delete => "Delete",
        Key::Left => "ArrowLeft",
        Key::Right => "ArrowRight",
        Key::Up => "ArrowUp",
        Key::Down => "ArrowDown",
        Key::Home => "Home",
        Key::End => "End",
        Key::PageUp => "PageUp",
        Key::PageDown => "PageDown",
        _ => return None,
    })
}
