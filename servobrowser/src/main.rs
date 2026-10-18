//! Servo Browser: a desktop shell around the Servo engine.
//!
//! The window owns one OpenGL context. Every tab's `WebView` renders into a shared offscreen
//! framebuffer; each frame egui draws the tab strip and toolbar and blits the active tab's
//! framebuffer underneath. `--screenshot` renders a page with Servo's software renderer.

mod keys;
mod protocol;

use egui::text::{CCursor, CCursorRange};
use egui::text_edit::TextEditState;
use egui::{Align2, Id, LayerId, Order, PaintCallback, Vec2};
use egui_glow::{CallbackFn, EguiGlow};
use euclid::{Point2D, Rect as ERect, Scale, Size2D};
use servo::protocol_handler::ProtocolRegistry;
use servo::{
    ConsoleLogLevel, CreateNewWebViewRequest, Cursor, DeviceIntRect, DeviceIntSize, DevicePoint, EmbedderControl,
    EmbedderControlId, EventLoopWaker, InputEvent, LoadStatus, MouseButton as ServoButton, MouseButtonAction,
    MouseButtonEvent, MouseMoveEvent, OffscreenRenderingContext, RenderingContext, SelectElement,
    SelectElementOptionOrOptgroup, Servo, ServoBuilder, SimpleDialog, SoftwareRenderingContext, WebView,
    WebViewBuilder, WebViewDelegate, WebViewId, WheelDelta, WheelEvent, WheelMode, WindowRenderingContext,
};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::{Duration, Instant};
use url::Url;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key as WKey, ModifiersState, NamedKey as WNamed};
use winit::raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::window::{CursorIcon, Window, WindowId};

const HOME_URL: &str = "browser:home";
const LINE_HEIGHT: f32 = 76.0;

fn protocols() -> ProtocolRegistry {
    let mut registry = ProtocolRegistry::default();
    let _ = registry.register("browser", protocol::BrowserPages);
    registry
}

/// Turns address bar text into a URL: full URLs, local paths, bare domains, or a DuckDuckGo search.
pub fn parse_input(text: &str) -> Option<Url> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    if let Ok(u) = Url::parse(t) {
        if matches!(u.scheme(), "http" | "https" | "file" | "data" | "browser" | "about") {
            return Some(u);
        }
    }
    let p = Path::new(t);
    if p.exists() {
        if let Some(u) = p.canonicalize().ok().and_then(|abs| Url::from_file_path(abs).ok()) {
            return Some(u);
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
    Url::parse_with_params("https://html.duckduckgo.com/html/", &[("q", t)]).ok()
}

#[derive(Debug)]
struct Wake;

/// Wakes the winit event loop when Servo has work to do.
#[derive(Clone)]
struct Waker(EventLoopProxy<Wake>);

impl EventLoopWaker for Waker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        let _ = self.0.send_event(Wake);
    }
}

enum DialogKind {
    Simple(Option<SimpleDialog>, String),
    Select(Option<SelectElement>),
}

/// A page dialog or `<select>` picker waiting for the user.
struct PendingDialog {
    id: EmbedderControlId,
    webview: WebViewId,
    kind: DialogKind,
}

/// Toolbar and shortcut actions, applied after the egui frame so no UI state is borrowed.
enum Command {
    Back,
    Forward,
    Reload,
    Stop,
    Go(String),
    NewTab,
    CloseTab(usize),
    Activate(usize),
}

#[derive(Default)]
struct UiState {
    location: String,
    location_dirty: bool,
    focus_location: bool,
    status: Option<String>,
    webview_rect: Option<egui::Rect>,
    pixels_per_point: f32,
}

/// The browser window: Servo instance, tabs, GL contexts and UI state.
struct Browser {
    me: Weak<Browser>,
    window: Window,
    servo: Servo,
    window_ctx: Rc<WindowRenderingContext>,
    offscreen: Rc<OffscreenRenderingContext>,
    egui: RefCell<EguiGlow>,
    tabs: RefCell<Vec<WebView>>,
    active: Cell<usize>,
    ui: RefCell<UiState>,
    dialogs: RefCell<Vec<PendingDialog>>,
    modifiers: Cell<ModifiersState>,
    cursor: Cell<Option<PhysicalPosition<f64>>>,
}

fn display_url(url: &Url) -> String {
    url.to_string()
}

fn cursor_icon(c: Cursor) -> Option<CursorIcon> {
    Some(match c {
        Cursor::None => return None,
        Cursor::Default => CursorIcon::Default,
        Cursor::Pointer => CursorIcon::Pointer,
        Cursor::ContextMenu => CursorIcon::ContextMenu,
        Cursor::Help => CursorIcon::Help,
        Cursor::Progress => CursorIcon::Progress,
        Cursor::Wait => CursorIcon::Wait,
        Cursor::Cell => CursorIcon::Cell,
        Cursor::Crosshair => CursorIcon::Crosshair,
        Cursor::Text => CursorIcon::Text,
        Cursor::VerticalText => CursorIcon::VerticalText,
        Cursor::Alias => CursorIcon::Alias,
        Cursor::Copy => CursorIcon::Copy,
        Cursor::Move => CursorIcon::Move,
        Cursor::NoDrop => CursorIcon::NoDrop,
        Cursor::NotAllowed => CursorIcon::NotAllowed,
        Cursor::Grab => CursorIcon::Grab,
        Cursor::Grabbing => CursorIcon::Grabbing,
        Cursor::EResize => CursorIcon::EResize,
        Cursor::NResize => CursorIcon::NResize,
        Cursor::NeResize => CursorIcon::NeResize,
        Cursor::NwResize => CursorIcon::NwResize,
        Cursor::SResize => CursorIcon::SResize,
        Cursor::SeResize => CursorIcon::SeResize,
        Cursor::SwResize => CursorIcon::SwResize,
        Cursor::WResize => CursorIcon::WResize,
        Cursor::EwResize => CursorIcon::EwResize,
        Cursor::NsResize => CursorIcon::NsResize,
        Cursor::NeswResize => CursorIcon::NeswResize,
        Cursor::NwseResize => CursorIcon::NwseResize,
        Cursor::ColResize => CursorIcon::ColResize,
        Cursor::RowResize => CursorIcon::RowResize,
        Cursor::AllScroll => CursorIcon::AllScroll,
        Cursor::ZoomIn => CursorIcon::ZoomIn,
        Cursor::ZoomOut => CursorIcon::ZoomOut,
    })
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() > n {
        format!("{}…", s.chars().take(n.saturating_sub(1)).collect::<String>())
    } else {
        s.to_string()
    }
}

impl Browser {
    fn new(event_loop: &ActiveEventLoop, proxy: EventLoopProxy<Wake>, start: Url) -> Rc<Browser> {
        let window = event_loop
            .create_window(
                Window::default_attributes()
                    .with_title("Servo Browser")
                    .with_inner_size(LogicalSize::new(1200.0, 820.0)),
            )
            .expect("Failed to create window");
        let display_handle = event_loop.display_handle().expect("No display handle");
        let window_handle = window.window_handle().expect("No window handle");
        let window_ctx = Rc::new(
            WindowRenderingContext::new(display_handle, window_handle, window.inner_size())
                .expect("Could not create an OpenGL context for the window"),
        );
        window_ctx
            .make_current()
            .expect("Could not make the GL context current");
        let offscreen = Rc::new(window_ctx.offscreen_context(window.inner_size()));
        let egui = EguiGlow::new(event_loop, window_ctx.glow_gl_api(), None, None, false);
        egui.egui_ctx.options_mut(|o| {
            o.zoom_with_keyboard = false;
            o.fallback_theme = egui::Theme::Light;
        });
        egui.egui_ctx.set_zoom_factor(1.15);
        let mut fonts = egui::FontDefinitions::default();
        let candidates = [
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "C:\\Windows\\Fonts\\seguisym.ttf",
            "C:\\Windows\\Fonts\\arial.ttf",
        ];
        if let Some(bytes) = candidates.iter().find_map(|p| std::fs::read(p).ok()) {
            fonts
                .font_data
                .insert("system-fallback".into(), Arc::new(egui::FontData::from_owned(bytes)));
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(family).or_default().push("system-fallback".into());
            }
        }
        egui.egui_ctx.set_fonts(fonts);
        let servo = ServoBuilder::default()
            .event_loop_waker(Box::new(Waker(proxy)))
            .protocol_registry(protocols())
            .build();
        servo.setup_logging();
        let browser = Rc::new_cyclic(|me| Browser {
            me: me.clone(),
            window,
            servo,
            window_ctx,
            offscreen,
            egui: RefCell::new(egui),
            tabs: RefCell::new(Vec::new()),
            active: Cell::new(0),
            ui: RefCell::new(UiState::default()),
            dialogs: RefCell::new(Vec::new()),
            modifiers: Cell::new(ModifiersState::empty()),
            cursor: Cell::new(None),
        });
        browser.new_tab(start);
        browser
    }

    fn scale(&self) -> Scale<f32, servo::DeviceIndependentPixel, servo::DevicePixel> {
        Scale::new(self.window.scale_factor() as f32)
    }

    fn active_webview(&self) -> Option<WebView> {
        self.tabs.borrow().get(self.active.get()).cloned()
    }

    fn is_active(&self, wv: &WebView) -> bool {
        self.active_webview().map_or(false, |a| a.id() == wv.id())
    }

    fn add_tab(&self, wv: WebView) {
        self.tabs.borrow_mut().push(wv);
        let n = self.tabs.borrow().len();
        self.activate(n - 1);
    }

    fn new_tab(&self, url: Url) {
        let me: Rc<dyn WebViewDelegate> = self.me.upgrade().expect("browser alive");
        let wv = WebViewBuilder::new(&self.servo, self.offscreen.clone())
            .url(url)
            .hidpi_scale_factor(self.scale())
            .delegate(me)
            .build();
        self.add_tab(wv);
    }

    fn activate(&self, index: usize) {
        let tabs = self.tabs.borrow().clone();
        if index >= tabs.len() {
            return;
        }
        for (i, t) in tabs.iter().enumerate() {
            if i == index {
                t.show();
                t.set_focused(true);
            } else {
                t.set_focused(false);
                t.hide();
            }
        }
        self.active.set(index);
        let wv = &tabs[index];
        {
            let mut ui = self.ui.borrow_mut();
            ui.location = wv.url().map(|u| display_url(&u)).unwrap_or_default();
            ui.location_dirty = false;
            ui.status = None;
        }
        self.update_title();
        self.window.request_redraw();
    }

    fn close_tab(&self, index: usize) {
        let removed = {
            let mut tabs = self.tabs.borrow_mut();
            if index >= tabs.len() {
                return;
            }
            tabs.remove(index)
        };
        drop(removed);
        let len = self.tabs.borrow().len();
        if len == 0 {
            self.new_tab(Url::parse(HOME_URL).unwrap());
            return;
        }
        let active = self.active.get();
        let next = if index < active || active >= len {
            active.saturating_sub(1).min(len - 1)
        } else {
            active.min(len - 1)
        };
        self.activate(next);
    }

    fn update_title(&self) {
        let title = self
            .active_webview()
            .and_then(|w| w.page_title())
            .filter(|t| !t.trim().is_empty());
        self.window.set_title(&match title {
            Some(t) => format!("{} - Servo Browser", t),
            None => "Servo Browser".to_string(),
        });
    }

    fn apply(&self, cmd: Command) {
        let wv = self.active_webview();
        match cmd {
            Command::Back => {
                if let Some(w) = wv {
                    w.go_back(1);
                }
            }
            Command::Forward => {
                if let Some(w) = wv {
                    w.go_forward(1);
                }
            }
            Command::Reload => {
                if let Some(w) = wv {
                    w.reload();
                }
            }
            Command::Stop => {}
            Command::Go(text) => {
                if let (Some(w), Some(u)) = (wv, parse_input(&text)) {
                    {
                        let mut ui = self.ui.borrow_mut();
                        ui.location = display_url(&u);
                        ui.location_dirty = false;
                    }
                    w.load(u);
                    w.set_focused(true);
                }
            }
            Command::NewTab => {
                self.new_tab(Url::parse(HOME_URL).unwrap());
                self.ui.borrow_mut().focus_location = true;
            }
            Command::CloseTab(i) => self.close_tab(i),
            Command::Activate(i) => self.activate(i),
        }
        self.window.request_redraw();
    }

    fn surrender_ui_focus(&self) {
        self.egui.borrow().egui_ctx.memory_mut(|m| {
            if let Some(f) = m.focused() {
                m.surrender_focus(f);
            }
        });
    }

    /// Handles browser shortcuts before the page sees the key. Returns true if consumed.
    fn handle_shortcut(&self, event: &KeyEvent, event_loop: &ActiveEventLoop) -> bool {
        if event.state != ElementState::Pressed {
            return false;
        }
        let m = self.modifiers.get();
        let cmd = if cfg!(target_os = "macos") {
            m.super_key()
        } else {
            m.control_key()
        };
        let key = &event.logical_key;
        let ch = |c: &str| matches!(key, WKey::Character(s) if s.eq_ignore_ascii_case(c));
        let named = |n: WNamed| matches!(key, WKey::Named(k) if *k == n);
        let command = if (cmd && ch("l")) || named(WNamed::F6) || (m.alt_key() && ch("d")) {
            self.ui.borrow_mut().focus_location = true;
            self.window.request_redraw();
            return true;
        } else if cmd && ch("t") {
            Command::NewTab
        } else if cmd && ch("w") {
            Command::CloseTab(self.active.get())
        } else if (cmd && ch("r")) || named(WNamed::F5) {
            Command::Reload
        } else if m.alt_key() && named(WNamed::ArrowLeft) {
            Command::Back
        } else if m.alt_key() && named(WNamed::ArrowRight) {
            Command::Forward
        } else if cmd && named(WNamed::Tab) {
            let n = self.tabs.borrow().len().max(1);
            let a = self.active.get();
            Command::Activate(if m.shift_key() { (a + n - 1) % n } else { (a + 1) % n })
        } else if cmd && (ch("=") || ch("+") || ch("-") || ch("0")) {
            if let Some(w) = self.active_webview() {
                let z = w.page_zoom();
                let nz = if ch("0") {
                    1.0
                } else if ch("-") {
                    (z - 0.1).max(0.3)
                } else {
                    (z + 0.1).min(5.0)
                };
                w.set_page_zoom(nz);
                self.ui.borrow_mut().status = Some(format!("Zoom {}%", (nz * 100.0).round()));
                self.window.request_redraw();
            }
            return true;
        } else if cmd && ch("q") {
            event_loop.exit();
            return true;
        } else {
            return false;
        };
        self.apply(command);
        true
    }

    /// Converts a window position to a point inside the web view, or `None` if it is over the UI.
    fn webview_point(&self, pos: PhysicalPosition<f64>) -> Option<DevicePoint> {
        let ui = self.ui.borrow();
        let rect = ui.webview_rect?;
        let s = ui.pixels_per_point.max(0.1);
        let x = pos.x as f32 - rect.min.x * s;
        let y = pos.y as f32 - rect.min.y * s;
        if x < 0.0 || y < 0.0 || x > rect.width() * s || y > rect.height() * s {
            return None;
        }
        Some(DevicePoint::new(x, y))
    }

    fn over_ui(&self) -> bool {
        if !self.dialogs.borrow().is_empty() {
            return true;
        }
        match self.cursor.get() {
            Some(p) => self.webview_point(p).is_none(),
            None => true,
        }
    }

    /// Routes a window event to egui or to the active web view.
    fn on_window_event(&self, event_loop: &ActiveEventLoop, event: WindowEvent) {
        match &event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                return;
            }
            WindowEvent::RedrawRequested => {
                self.redraw();
                return;
            }
            WindowEvent::Resized(size) => {
                self.window_ctx.resize(*size);
                self.window.request_redraw();
            }
            WindowEvent::ModifiersChanged(m) => self.modifiers.set(m.state()),
            WindowEvent::CursorMoved { position, .. } => self.cursor.set(Some(*position)),
            WindowEvent::KeyboardInput { event: k, .. } => {
                if self.handle_shortcut(k, event_loop) {
                    return;
                }
            }
            _ => {}
        }
        let ui_has_keyboard =
            self.egui.borrow().egui_ctx.memory(|m| m.focused().is_some()) || !self.dialogs.borrow().is_empty();
        let to_ui = match &event {
            WindowEvent::KeyboardInput { .. } | WindowEvent::Ime(_) => ui_has_keyboard,
            WindowEvent::MouseInput { .. } | WindowEvent::MouseWheel { .. } => self.over_ui(),
            _ => true,
        };
        if to_ui {
            let response = self.egui.borrow_mut().on_window_event(&self.window, &event);
            if response.repaint {
                self.window.request_redraw();
            }
            let input_event = matches!(
                event,
                WindowEvent::KeyboardInput { .. }
                    | WindowEvent::MouseInput { .. }
                    | WindowEvent::MouseWheel { .. }
                    | WindowEvent::Ime(_)
            );
            if response.consumed || input_event {
                return;
            }
        }
        let Some(wv) = self.active_webview() else { return };
        match event {
            WindowEvent::KeyboardInput { event: k, .. } => {
                wv.notify_input_event(InputEvent::Keyboard(keys::keyboard_event(&k, self.modifiers.get())));
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(p) = self.webview_point(position) {
                    wv.notify_input_event(InputEvent::MouseMove(MouseMoveEvent::new(p.into())));
                }
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let Some(p) = self.cursor.get().and_then(|c| self.webview_point(c)) else {
                    return;
                };
                if state == ElementState::Pressed {
                    self.surrender_ui_focus();
                    wv.set_focused(true);
                }
                let b = match button {
                    MouseButton::Left => ServoButton::Primary,
                    MouseButton::Right => ServoButton::Secondary,
                    MouseButton::Middle => ServoButton::Auxiliary,
                    MouseButton::Back => {
                        if state == ElementState::Pressed {
                            self.apply(Command::Back);
                        }
                        return;
                    }
                    MouseButton::Forward => {
                        if state == ElementState::Pressed {
                            self.apply(Command::Forward);
                        }
                        return;
                    }
                    MouseButton::Other(v) => ServoButton::Other(v),
                };
                let action = if state == ElementState::Pressed {
                    MouseButtonAction::Down
                } else {
                    MouseButtonAction::Up
                };
                wv.notify_input_event(InputEvent::MouseButton(MouseButtonEvent::new(action, b, p.into())));
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let Some(p) = self.cursor.get().and_then(|c| self.webview_point(c)) else {
                    return;
                };
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => ((x * LINE_HEIGHT) as f64, (y * LINE_HEIGHT) as f64),
                    MouseScrollDelta::PixelDelta(d) => (d.x, d.y),
                };
                wv.notify_input_event(InputEvent::Wheel(WheelEvent::new(
                    WheelDelta {
                        x: dx,
                        y: dy,
                        z: 0.0,
                        mode: WheelMode::DeltaPixel,
                    },
                    p.into(),
                )));
            }
            _ => {}
        }
    }

    /// Draws one frame: UI layout, web view resize and paint, then egui on top.
    fn redraw(&self) {
        if self.window_ctx.make_current().is_err() {
            return;
        }
        let mut commands = Vec::new();
        let tabs: Vec<(String, bool)> = self
            .tabs
            .borrow()
            .iter()
            .map(|w| {
                let title = w
                    .page_title()
                    .filter(|t| !t.trim().is_empty())
                    .or_else(|| w.url().map(|u| u.to_string()))
                    .unwrap_or_else(|| "New tab".into());
                let loading = !matches!(w.load_status(), LoadStatus::Complete);
                (title, loading)
            })
            .collect();
        let active = self.active_webview();
        let can_back = active.as_ref().map_or(false, |w| w.can_go_back());
        let can_forward = active.as_ref().map_or(false, |w| w.can_go_forward());
        let loading = active
            .as_ref()
            .map_or(false, |w| !matches!(w.load_status(), LoadStatus::Complete));
        let mut render_to_parent = self.offscreen.render_to_parent_callback();
        let mut dialogs = std::mem::take(&mut *self.dialogs.borrow_mut());
        let active_id = active.as_ref().map(|w| w.id());
        {
            let mut egui = self.egui.borrow_mut();
            let mut ui_state = self.ui.borrow_mut();
            let active_index = self.active.get();
            egui.run(&self.window, |ui| {
                let frame = egui::Frame::default()
                    .fill(ui.style().visuals.panel_fill)
                    .inner_margin(egui::Margin::symmetric(6, 4));
                egui::Panel::top("tabs").frame(frame).show_inside(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (i, (title, tab_loading)) in tabs.iter().enumerate() {
                            let label = format!("{}{}", if *tab_loading { "◌ " } else { "" }, truncate(title, 24));
                            let tab = ui.add(
                                egui::Button::new(label)
                                    .selected(i == active_index)
                                    .min_size(Vec2::new(150.0, 24.0)),
                            );
                            if tab.clicked() {
                                commands.push(Command::Activate(i));
                            }
                            if tab.middle_clicked() {
                                commands.push(Command::CloseTab(i));
                            }
                            if ui
                                .add(egui::Button::new("×").frame(false))
                                .on_hover_text("Close tab")
                                .clicked()
                            {
                                commands.push(Command::CloseTab(i));
                            }
                            ui.add_space(4.0);
                        }
                        if ui
                            .add(egui::Button::new(" + ").frame(false))
                            .on_hover_text("New tab (Ctrl+T)")
                            .clicked()
                        {
                            commands.push(Command::NewTab);
                        }
                    });
                });
                egui::Panel::top("toolbar").frame(frame).show_inside(ui, |ui| {
                    ui.horizontal(|ui| {
                        let nav = |t: &'static str| egui::Button::new(t).frame(false).min_size(Vec2::new(28.0, 26.0));
                        if ui
                            .add_enabled(can_back, nav("⏴"))
                            .on_hover_text("Back (Alt+Left)")
                            .clicked()
                        {
                            commands.push(Command::Back);
                        }
                        if ui
                            .add_enabled(can_forward, nav("⏵"))
                            .on_hover_text("Forward (Alt+Right)")
                            .clicked()
                        {
                            commands.push(Command::Forward);
                        }
                        if loading {
                            if ui.add(nav("✖")).on_hover_text("Stop").clicked() {
                                commands.push(Command::Stop);
                            }
                        } else if ui.add(nav("↻")).on_hover_text("Reload (Ctrl+R)").clicked() {
                            commands.push(Command::Reload);
                        }
                        if ui.add(nav("🏠")).on_hover_text("Home").clicked() {
                            commands.push(Command::Go(HOME_URL.into()));
                        }
                        let id = Id::new("location");
                        let field = ui.add_sized(
                            Vec2::new(ui.available_width(), 28.0),
                            egui::TextEdit::singleline(&mut ui_state.location)
                                .id(id)
                                .hint_text("Search or enter address")
                                .vertical_align(egui::Align::Center),
                        );
                        if field.changed() {
                            ui_state.location_dirty = true;
                        }
                        if ui_state.focus_location {
                            ui_state.focus_location = false;
                            field.request_focus();
                        }
                        if field.gained_focus() {
                            if let Some(mut st) = TextEditState::load(ui.ctx(), id) {
                                let end = ui_state.location.chars().count();
                                st.cursor
                                    .set_char_range(Some(CCursorRange::two(CCursor::new(0), CCursor::new(end))));
                                st.store(ui.ctx(), id);
                            }
                        }
                        if field.lost_focus() {
                            if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                                commands.push(Command::Go(ui_state.location.clone()));
                            } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                ui_state.location_dirty = false;
                                ui_state.location = active
                                    .as_ref()
                                    .and_then(|w| w.url())
                                    .map(|u| display_url(&u))
                                    .unwrap_or_default();
                            }
                        }
                    });
                });
                let available = ui.available_rect_before_wrap();
                ui_state.webview_rect = Some(available);
                ui_state.pixels_per_point = ui.ctx().pixels_per_point();

                if let Some(status) = &ui_state.status {
                    egui::Area::new(Id::new("status"))
                        .order(Order::Tooltip)
                        .anchor(Align2::LEFT_BOTTOM, Vec2::new(4.0, -4.0))
                        .interactable(false)
                        .show(ui.ctx(), |ui| {
                            egui::Frame::popup(ui.style()).show(ui, |ui| {
                                ui.label(truncate(status, 120));
                            });
                        });
                }

                for d in dialogs.iter_mut().filter(|d| Some(d.webview) == active_id) {
                    let modal = egui::Modal::new(Id::new(("dialog", format!("{:?}", d.id)))).show(ui.ctx(), |ui| {
                        ui.set_max_width(460.0);
                        match &mut d.kind {
                            DialogKind::Simple(slot, input) => {
                                let Some(dialog) = slot.as_ref() else { return };
                                let origin = active
                                    .as_ref()
                                    .and_then(|w| w.url())
                                    .and_then(|u| u.host_str().map(str::to_string));
                                ui.strong(match origin {
                                    Some(h) => format!("{} says", h),
                                    None => "This page says".into(),
                                });
                                ui.add_space(6.0);
                                ui.label(dialog.message().to_string());
                                ui.add_space(8.0);
                                let is_prompt = matches!(dialog, SimpleDialog::Prompt(_));
                                let has_cancel = !matches!(dialog, SimpleDialog::Alert(_));
                                if is_prompt {
                                    ui.text_edit_singleline(input).request_focus();
                                }
                                ui.horizontal(|ui| {
                                    let ok = ui.button("OK").clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter));
                                    let cancel = has_cancel
                                        && (ui.button("Cancel").clicked()
                                            || ui.input(|i| i.key_pressed(egui::Key::Escape)));
                                    if ok {
                                        if let Some(mut dlg) = slot.take() {
                                            if let SimpleDialog::Prompt(p) = &mut dlg {
                                                p.set_current_value(input);
                                            }
                                            dlg.confirm();
                                        }
                                    } else if cancel {
                                        if let Some(dlg) = slot.take() {
                                            dlg.dismiss();
                                        }
                                    }
                                });
                            }
                            DialogKind::Select(slot) => {
                                let Some(select) = slot.as_mut() else { return };
                                ui.strong("Choose an option");
                                ui.add_space(6.0);
                                let current = select.selected_options();
                                let mut picked = None;
                                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                                    for item in select.options() {
                                        match item {
                                            SelectElementOptionOrOptgroup::Option(o) => {
                                                let b = egui::Button::new(o.label.as_str())
                                                    .selected(current.contains(&o.id));
                                                if ui.add_enabled(!o.is_disabled, b).clicked() {
                                                    picked = Some(o.id);
                                                }
                                            }
                                            SelectElementOptionOrOptgroup::Optgroup { label, options } => {
                                                ui.weak(label.as_str());
                                                for o in options {
                                                    let b = egui::Button::new(format!("  {}", o.label))
                                                        .selected(current.contains(&o.id));
                                                    if ui.add_enabled(!o.is_disabled, b).clicked() {
                                                        picked = Some(o.id);
                                                    }
                                                }
                                            }
                                        }
                                    }
                                });
                                if let Some(id) = picked {
                                    if let Some(mut s) = slot.take() {
                                        s.select(vec![id]);
                                        s.submit();
                                    }
                                }
                            }
                        }
                    });
                    if modal.should_close() {
                        match &mut d.kind {
                            DialogKind::Simple(slot, _) => {
                                if let Some(dlg) = slot.take() {
                                    dlg.dismiss();
                                }
                            }
                            DialogKind::Select(slot) => {
                                slot.take();
                            }
                        }
                    }
                }

                if let Some(render) = render_to_parent.take() {
                    ui.ctx().layer_painter(LayerId::background()).add(PaintCallback {
                        rect: available,
                        callback: Arc::new(CallbackFn::new(move |info, painter| {
                            let clip = info.viewport_in_pixels();
                            let r = ERect::new(
                                Point2D::new(clip.left_px, clip.from_bottom_px),
                                Size2D::new(clip.width_px, clip.height_px),
                            );
                            render(painter.gl(), r)
                        })),
                    });
                }
            });
        }
        dialogs.retain(|d| match &d.kind {
            DialogKind::Simple(s, _) => s.is_some(),
            DialogKind::Select(s) => s.is_some(),
        });
        {
            let mut current = self.dialogs.borrow_mut();
            dialogs.extend(current.drain(..));
            *current = dialogs;
        }
        for c in commands {
            self.apply(c);
        }
        if let Some(wv) = self.active_webview() {
            let (rect, s) = {
                let ui = self.ui.borrow();
                (ui.webview_rect, ui.pixels_per_point.max(0.1))
            };
            if let Some(rect) = rect {
                let w = (rect.width() * s).max(1.0) as u32;
                let h = (rect.height() * s).max(1.0) as u32;
                let cur = wv.size();
                if cur.width as u32 != w || cur.height as u32 != h {
                    wv.resize(PhysicalSize::new(w, h));
                }
            }
            wv.paint();
        }
        let _ = self.window_ctx.make_current();
        self.window_ctx.prepare_for_rendering();
        let mut egui = self.egui.borrow_mut();
        egui.paint(&self.window);
        self.window_ctx.present();
        if egui.egui_ctx.has_requested_repaint() {
            self.window.request_redraw();
        }
    }
}

impl WebViewDelegate for Browser {
    fn notify_new_frame_ready(&self, _: WebView) {
        self.window.request_redraw();
    }

    fn notify_url_changed(&self, wv: WebView, url: Url) {
        if self.is_active(&wv) {
            let mut ui = self.ui.borrow_mut();
            if !ui.location_dirty {
                ui.location = display_url(&url);
            }
        }
        self.window.request_redraw();
    }

    fn notify_page_title_changed(&self, wv: WebView, _: Option<String>) {
        if self.is_active(&wv) {
            self.update_title();
        }
        self.window.request_redraw();
    }

    fn notify_load_status_changed(&self, _: WebView, _: LoadStatus) {
        self.window.request_redraw();
    }

    fn notify_status_text_changed(&self, wv: WebView, status: Option<String>) {
        if self.is_active(&wv) {
            self.ui.borrow_mut().status = status;
            self.window.request_redraw();
        }
    }

    fn notify_cursor_changed(&self, wv: WebView, cursor: Cursor) {
        if !self.is_active(&wv) {
            return;
        }
        match cursor_icon(cursor) {
            Some(icon) => {
                self.window.set_cursor_visible(true);
                self.window.set_cursor(icon);
            }
            None => self.window.set_cursor_visible(false),
        }
    }

    fn notify_history_changed(&self, _: WebView, _: Vec<Url>, _: usize) {
        self.window.request_redraw();
    }

    fn notify_closed(&self, wv: WebView) {
        let index = self.tabs.borrow().iter().position(|t| t.id() == wv.id());
        if let Some(i) = index {
            self.close_tab(i);
        }
    }

    fn notify_crashed(&self, wv: WebView, reason: String, _: Option<String>) {
        if self.is_active(&wv) {
            self.ui.borrow_mut().status = Some(format!("This tab crashed: {}", reason));
            self.window.request_redraw();
        }
    }

    fn request_create_new(&self, _: WebView, request: CreateNewWebViewRequest) {
        let me: Rc<dyn WebViewDelegate> = self.me.upgrade().expect("browser alive");
        let wv = request
            .builder(self.offscreen.clone())
            .hidpi_scale_factor(self.scale())
            .delegate(me)
            .build();
        self.add_tab(wv);
    }

    fn show_embedder_control(&self, wv: WebView, control: EmbedderControl) {
        let id = control.id();
        let kind = match control {
            EmbedderControl::SimpleDialog(d) => {
                let input = match &d {
                    SimpleDialog::Prompt(p) => p.current_value().to_string(),
                    _ => String::new(),
                };
                DialogKind::Simple(Some(d), input)
            }
            EmbedderControl::SelectElement(s) => DialogKind::Select(Some(s)),
            _ => return,
        };
        self.dialogs.borrow_mut().push(PendingDialog {
            id,
            webview: wv.id(),
            kind,
        });
        self.window.request_redraw();
    }

    fn hide_embedder_control(&self, _: WebView, id: EmbedderControlId) {
        self.dialogs.borrow_mut().retain(|d| d.id != id);
        self.window.request_redraw();
    }

    fn show_console_message(&self, _: WebView, level: ConsoleLogLevel, message: String) {
        eprintln!("[console.{:?}] {}", level, message);
    }
}

struct App {
    proxy: EventLoopProxy<Wake>,
    start: Url,
    browser: Option<Rc<Browser>>,
}

impl ApplicationHandler<Wake> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.browser.is_none() {
            self.browser = Some(Browser::new(event_loop, self.proxy.clone(), self.start.clone()));
        }
    }

    fn user_event(&mut self, _: &ActiveEventLoop, _: Wake) {
        if let Some(b) = &self.browser {
            b.servo.spin_event_loop();
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        if let Some(b) = self.browser.clone() {
            b.servo.spin_event_loop();
            b.on_window_event(event_loop, event);
        }
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        if let Some(b) = self.browser.take() {
            b.tabs.borrow_mut().clear();
            if b.window_ctx.make_current().is_ok() {
                b.egui.borrow_mut().destroy();
            }
        }
    }
}

/// Delegate for screenshot mode that records when the page finished loading.
struct Headless {
    complete: Cell<Option<Instant>>,
}

impl WebViewDelegate for Headless {
    fn notify_load_status_changed(&self, _: WebView, status: LoadStatus) {
        if matches!(status, LoadStatus::Complete) && self.complete.get().is_none() {
            self.complete.set(Some(Instant::now()));
        }
    }

    fn show_console_message(&self, _: WebView, level: ConsoleLogLevel, message: String) {
        eprintln!("[console.{:?}] {}", level, message);
    }
}

/// Loads a page with the software renderer, waits for it to settle and writes a PNG.
fn screenshot(url: Url, out: &str, width: u32, height: u32) -> Result<(), String> {
    let ctx = Rc::new(SoftwareRenderingContext::new(PhysicalSize::new(width, height)).map_err(|e| format!("{:?}", e))?);
    ctx.make_current().map_err(|e| format!("{:?}", e))?;
    let servo = ServoBuilder::default().protocol_registry(protocols()).build();
    let delegate = Rc::new(Headless {
        complete: Cell::new(None),
    });
    let wv = WebViewBuilder::new(&servo, ctx.clone())
        .url(url.clone())
        .delegate(delegate.clone())
        .build();
    let start = Instant::now();
    loop {
        servo.spin_event_loop();
        std::thread::sleep(Duration::from_millis(10));
        let settled = delegate
            .complete
            .get()
            .map_or(false, |t| t.elapsed() > Duration::from_millis(1500));
        if settled || start.elapsed() > Duration::from_secs(45) {
            break;
        }
    }
    for _ in 0..20 {
        wv.paint();
        servo.spin_event_loop();
        std::thread::sleep(Duration::from_millis(20));
    }
    wv.paint();
    let image = ctx
        .read_to_image(DeviceIntRect::from_size(DeviceIntSize::new(
            width as i32,
            height as i32,
        )))
        .ok_or("Nothing was rendered")?;
    let file = std::fs::File::create(out).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), image.width(), image.height());
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut w| w.write_image_data(image.as_raw()))
        .map_err(|e| e.to_string())?;
    let note = if delegate.complete.get().is_none() {
        ", page did not finish loading"
    } else {
        ""
    };
    println!("{} -> {} ({}x{}){}", url, out, width, height, note);
    Ok(())
}

fn main() {
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("Failed to install the TLS crypto provider");
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--screenshot") {
        if args.len() < 3 {
            eprintln!("usage: servobrowser --screenshot <url> <out.png> [width] [height]");
            std::process::exit(2);
        }
        let url = parse_input(&args[1]).unwrap_or_else(|| Url::parse(HOME_URL).unwrap());
        let w = args.get(3).and_then(|v| v.parse().ok()).unwrap_or(1200);
        let h = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(800);
        if let Err(e) = screenshot(url, &args[2], w, h) {
            eprintln!("Screenshot failed: {}", e);
            std::process::exit(1);
        }
        return;
    }
    let start = args
        .first()
        .and_then(|a| parse_input(a))
        .unwrap_or_else(|| Url::parse(HOME_URL).unwrap());
    let event_loop = EventLoop::<Wake>::with_user_event()
        .build()
        .expect("Failed to create the event loop");
    let mut app = App {
        proxy: event_loop.create_proxy(),
        start,
        browser: None,
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("Event loop error: {}", e);
    }
}
