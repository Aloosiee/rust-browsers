//! Browser-wide state and the handling of every event: windows, tabs, toolbar messages,
//! page messages, downloads and the app menu.

use crate::actions::Action;
use crate::event::UserEvent;
use crate::ipc::{self, ChromeMessage, PageMessage};
use crate::menu::AppMenu;
use crate::paths::Profile;
use crate::session::{Session, TabState, WindowState};
use crate::settings::{SEARCH_ENGINES, Settings};
use crate::store::{Store, now_ms};
use crate::tab::{ContextSlot, Start, TabId, build_page};
use crate::window::BrowserWindow;
use crate::{paths, protocol, urlbar};
use serde_json::{Value, json};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::str::FromStr;
use tao::event_loop::{EventLoopProxy, EventLoopWindowTarget};
use tao::window::{Fullscreen, WindowId};
use wry::{NewWindowFeatures, NewWindowResponse};

/// Zoom levels Ctrl+Plus and Ctrl+Minus step through, the same as Firefox's.
const ZOOM_LEVELS: &[f64] = &[
    0.3, 0.5, 0.67, 0.8, 0.9, 1.0, 1.1, 1.2, 1.33, 1.5, 1.7, 2.0, 2.4, 3.0, 4.0, 5.0,
];
const DEFAULT_SIZE: (f64, f64) = (1280.0, 860.0);
const CLOSED_TABS_KEPT: usize = 25;

/// How the first window is opened.
pub struct Startup {
    pub urls: Vec<String>,
    pub private: bool,
}

pub struct App {
    me: Weak<RefCell<App>>,
    proxy: EventLoopProxy<UserEvent>,
    profile: Profile,
    store: Store,
    settings: Settings,
    download_dir: Rc<RefCell<Option<PathBuf>>>,
    normal: ContextSlot,
    private: Option<ContextSlot>,
    windows: Vec<BrowserWindow>,
    next_tab: TabId,
    next_prompt: u64,
    menu: AppMenu,
    menu_window: Option<WindowId>,
    downloads_active: usize,
    last_download: Option<(String, String)>,
    session_dirty: bool,
    pub quit_requested: bool,
}

/// Whether a URL belongs in history: web pages only, never browser pages.
fn recordable(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://")) && !protocol::is_internal(url)
}

fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string))
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn js_string(s: &str) -> String {
    serde_json::to_string(s).expect("string serializes")
}

impl App {
    pub fn new(proxy: EventLoopProxy<UserEvent>, profile: Profile) -> Result<Rc<RefCell<App>>, String> {
        std::fs::create_dir_all(&profile.root).map_err(|e| format!("cannot create {}: {e}", profile.root.display()))?;
        let store = Store::open(&profile.database()).map_err(|e| format!("cannot open the database: {e}"))?;
        let _ = store.fail_interrupted_downloads();
        let settings = Settings::load(&profile.settings());
        let _ = std::fs::remove_dir_all(profile.private_web_data());
        let download_dir = Rc::new(RefCell::new(Self::download_dir_setting(&settings)));
        let normal = ContextSlot::new(profile.web_data(), false, download_dir.clone());
        Ok(Rc::new_cyclic(|me| {
            RefCell::new(App {
                me: me.clone(),
                proxy,
                profile,
                store,
                settings,
                download_dir,
                normal,
                private: None,
                windows: Vec::new(),
                next_tab: 1,
                next_prompt: 1,
                menu: AppMenu::new(),
                menu_window: None,
                downloads_active: 0,
                last_download: None,
                session_dirty: false,
                quit_requested: false,
            })
        }))
    }

    fn download_dir_setting(settings: &Settings) -> Option<PathBuf> {
        let dir = settings.download_dir.trim();
        (!dir.is_empty()).then(|| PathBuf::from(dir))
    }

    pub fn has_windows(&self) -> bool {
        !self.windows.is_empty()
    }

    /// Opens the first window(s): URLs from the command line, the previous session, or the
    /// home page.
    pub fn start(&mut self, target: &EventLoopWindowTarget<UserEvent>, startup: Startup) {
        let urls: Vec<String> = startup
            .urls
            .iter()
            .filter_map(|u| urlbar::interpret(u, &self.settings))
            .collect();
        if startup.private {
            let tabs = if urls.is_empty() { vec![self.newtab_url()] } else { urls };
            self.open_window(target, true, tabs, 0, None);
            return;
        }
        if urls.is_empty() && self.settings.restore_session {
            if let Some(session) = Session::load(&self.profile.session()) {
                for w in session.windows {
                    let tabs: Vec<String> = w
                        .tabs
                        .iter()
                        .filter_map(|t| urlbar::interpret(&t.url, &self.settings))
                        .collect();
                    if tabs.is_empty() {
                        continue;
                    }
                    let size = (w.width > 200.0 && w.height > 200.0).then_some((w.width, w.height, w.maximized));
                    let active = w.active.min(tabs.len() - 1);
                    if let Some(id) = self.open_window(target, false, tabs, active, size) {
                        let titles: Vec<String> = w.tabs.into_iter().map(|t| t.title).collect();
                        if let Some(win) = self.window_mut(id) {
                            for (tab, title) in win.tabs.iter_mut().zip(titles) {
                                if tab.pending.is_some() {
                                    tab.title = title;
                                }
                            }
                        }
                        self.push_state(id);
                    }
                }
                if self.has_windows() {
                    return;
                }
            }
        }
        let tabs = if urls.is_empty() { vec![self.home_url()] } else { urls };
        self.open_window(target, false, tabs, 0, None);
    }

    fn home_url(&self) -> String {
        urlbar::interpret(&self.settings.homepage, &self.settings).unwrap_or_else(|| self.newtab_url())
    }

    fn newtab_url(&self) -> String {
        protocol::page_url("newtab")
    }

    fn window(&self, id: WindowId) -> Option<&BrowserWindow> {
        self.windows.iter().find(|w| w.id == id)
    }

    fn window_mut(&mut self, id: WindowId) -> Option<&mut BrowserWindow> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    /// Opens a window with tabs for `urls`; only the active tab loads straight away.
    pub fn open_window(
        &mut self,
        target: &EventLoopWindowTarget<UserEvent>,
        private: bool,
        urls: Vec<String>,
        active: usize,
        size: Option<(f64, f64, bool)>,
    ) -> Option<WindowId> {
        let (w, h, maximized) = size.unwrap_or((DEFAULT_SIZE.0, DEFAULT_SIZE.1, false));
        let bar = self.settings.show_bookmarks_bar;
        let window = match BrowserWindow::new(target, &self.proxy, &mut self.normal, private, (w, h), maximized, bar) {
            Ok(win) => win,
            Err(e) => {
                eprintln!("could not open a window: {e}");
                return None;
            }
        };
        let id = window.id;
        self.windows.push(window);
        for (i, url) in urls.iter().enumerate() {
            if i == active {
                self.add_tab(id, Start::Url(url.clone()), None, false);
            } else {
                let tab = self.add_tab(id, Start::Url("about:blank".into()), None, false);
                if let (Some(tab), Some(win)) = (tab, self.window_mut(id)) {
                    if let Some(t) = win.tabs.iter_mut().find(|t| t.id == tab) {
                        t.pending = Some(url.clone());
                        t.url = url.clone();
                        t.loading = false;
                    }
                }
            }
        }
        if let Some(win) = self.window_mut(id) {
            win.activate(active.min(win.tabs.len().saturating_sub(1)));
            win.layout();
            win.update_title();
        }
        self.session_dirty = true;
        Some(id)
    }

    /// Creates a tab in a window. `position` defaults to the end of the tab strip.
    fn add_tab(&mut self, window: WindowId, start: Start, position: Option<usize>, activate: bool) -> Option<TabId> {
        let id = self.next_tab;
        self.next_tab += 1;
        let private = self.window(window)?.private;
        let popup_handler = self.popup_handler(window, id);
        let extensions = (!private).then(|| self.profile.extensions());
        if let Some(dir) = &extensions {
            let _ = std::fs::create_dir_all(dir);
        }
        let default_zoom = self.settings.default_zoom;
        let proxy = self.proxy.clone();
        let mut slot = if private { self.private.take() } else { None };
        let built = {
            let slot_ref = match slot.as_mut() {
                Some(s) => s,
                None if private => {
                    slot = Some(ContextSlot::new(
                        self.profile.private_web_data(),
                        true,
                        self.download_dir.clone(),
                    ));
                    slot.as_mut().expect("just set")
                }
                None => &mut self.normal,
            };
            let win = self.windows.iter().find(|w| w.id == window)?;
            let host = win.host(&proxy);
            let visible = activate || win.tabs.is_empty();
            build_page(
                &host,
                slot_ref,
                id,
                start,
                visible,
                extensions.as_deref(),
                popup_handler,
            )
        };
        if private {
            self.private = slot;
        }
        let mut tab = match built {
            Ok(tab) => tab,
            Err(e) => {
                eprintln!("could not create a tab: {e}");
                return None;
            }
        };
        tab.zoom = default_zoom;
        if (default_zoom - 1.0).abs() > f64::EPSILON {
            let _ = tab.webview.zoom(default_zoom);
        }
        let win = self.window_mut(window)?;
        let index = position.unwrap_or(win.tabs.len()).min(win.tabs.len());
        if index <= win.active && !win.tabs.is_empty() {
            win.active += 1;
        }
        win.tabs.insert(index, tab);
        if activate || win.tabs.len() == 1 {
            win.activate(index);
        }
        self.session_dirty = true;
        Some(id)
    }

    /// Handles `window.open` and links with `target=_blank` by opening a linked tab, so
    /// sign-in popups can still talk to the page that opened them.
    fn popup_handler(
        &self,
        window: WindowId,
        opener: TabId,
    ) -> Box<dyn Fn(String, NewWindowFeatures) -> NewWindowResponse> {
        let me = self.me.clone();
        let proxy = self.proxy.clone();
        Box::new(move |url, features| {
            if let Some(app) = me.upgrade() {
                if let Ok(mut app) = app.try_borrow_mut() {
                    if let Some(response) = app.create_popup(window, opener, &url, features) {
                        return response;
                    }
                    let _ = proxy.send_event(UserEvent::OpenTab { window, url });
                    return NewWindowResponse::Deny;
                }
            }
            let _ = proxy.send_event(UserEvent::OpenTab { window, url });
            NewWindowResponse::Deny
        })
    }

    #[allow(unused_variables)]
    fn create_popup(
        &mut self,
        window: WindowId,
        opener: TabId,
        url: &str,
        features: NewWindowFeatures,
    ) -> Option<NewWindowResponse> {
        #[cfg(any(target_os = "linux", windows))]
        {
            let win = self.window(window)?;
            if win.tabs.iter().find(|t| t.id == opener)?.is_internal() {
                return None;
            }
            let position = win.tab_index(opener).map(|i| i + 1);
            let id = self.add_tab(window, Start::Popup(Box::new(features)), position, true)?;
            let win = self.window_mut(window)?;
            let index = win.tab_index(id)?;
            let tab = &mut win.tabs[index];
            tab.url = url.to_string();
            tab.opener = Some(opener);
            tab.shared.set_url(url);
            #[cfg(target_os = "linux")]
            let webview = {
                use wry::WebViewExtUnix;
                tab.webview.webview()
            };
            #[cfg(windows)]
            let webview = {
                use wry::WebViewExtWindows;
                tab.webview.webview()
            };
            self.push_state(window);
            return Some(NewWindowResponse::Create { webview });
        }
        #[allow(unreachable_code)]
        None
    }

    /// Opens a new tab next to the active one, as Firefox does for links.
    fn open_tab(&mut self, window: WindowId, url: &str, activate: bool) {
        let position = self.window(window).map(|w| w.active + 1);
        self.add_tab(window, Start::Url(url.to_string()), position, activate);
        self.push_state(window);
    }

    fn close_tab(&mut self, window: WindowId, index: usize) {
        let Some(win) = self.window_mut(window) else { return };
        if index >= win.tabs.len() {
            return;
        }
        let tab = win.tabs.remove(index);
        let url = tab.pending.clone().unwrap_or_else(|| tab.url.clone());
        if !url.is_empty() && url != "about:blank" {
            win.closed.push(TabState {
                url: protocol::display_url(&url),
                title: tab.title.clone(),
            });
            if win.closed.len() > CLOSED_TABS_KEPT {
                win.closed.remove(0);
            }
        }
        drop(tab);
        if win.tabs.is_empty() {
            self.close_window(window);
            return;
        }
        let was_active = index == win.active;
        if index < win.active || win.active >= win.tabs.len() {
            win.active = win.active.saturating_sub(1);
        }
        if was_active {
            let next = index.min(win.tabs.len() - 1);
            win.active = next;
            win.activate(next);
            win.layout();
        }
        win.update_title();
        self.session_dirty = true;
        self.push_state(window);
    }

    pub fn close_window(&mut self, window: WindowId) {
        let Some(index) = self.windows.iter().position(|w| w.id == window) else {
            return;
        };
        let last_normal = !self.windows[index].private && self.windows.iter().filter(|w| !w.private).count() == 1;
        if last_normal || self.quit_requested {
            self.save_session();
        }
        let win = self.windows.remove(index);
        let was_private = win.private;
        drop(win);
        if was_private && !self.windows.iter().any(|w| w.private) {
            self.private = None;
            let _ = std::fs::remove_dir_all(self.profile.private_web_data());
        }
        if !last_normal {
            self.session_dirty = true;
        }
    }

    /// Saves open normal windows so the next start can restore them.
    pub fn save_session(&mut self) {
        let windows: Vec<WindowState> = self
            .windows
            .iter()
            .filter(|w| !w.private)
            .map(|w| {
                let (width, height, maximized) = w.size_for_session();
                WindowState {
                    tabs: w
                        .tabs
                        .iter()
                        .map(|t| TabState {
                            url: protocol::display_url(t.pending.as_deref().unwrap_or(&t.url)),
                            title: t.title.clone(),
                        })
                        .filter(|t| !t.url.is_empty() && t.url != "about:blank")
                        .collect(),
                    active: w.active,
                    width,
                    height,
                    maximized,
                }
            })
            .collect();
        if windows.is_empty() {
            return;
        }
        if let Err(e) = (Session { windows }).save(&self.profile.session()) {
            eprintln!("could not save the session: {e}");
        }
        self.session_dirty = false;
    }

    pub fn save_session_if_dirty(&mut self) {
        if self.session_dirty {
            self.save_session();
        }
    }

    /// Called on exit: saves state and removes private browsing data.
    pub fn shutdown(&mut self) {
        self.save_session();
        self.windows.clear();
        self.private = None;
        let _ = std::fs::remove_dir_all(self.profile.private_web_data());
    }

    pub fn layout(&mut self, window: WindowId) {
        if let Some(win) = self.window(window) {
            win.layout();
        }
    }

    /// Sends the toolbar everything it displays.
    fn push_state(&mut self, window: WindowId) {
        let Some(win) = self.window(window) else { return };
        if !win.chrome_ready {
            return;
        }
        let tabs: Vec<Value> = win
            .tabs
            .iter()
            .enumerate()
            .map(|(i, t)| {
                json!({
                    "id": t.id,
                    "title": t.display_title(),
                    "url": protocol::display_url(t.pending.as_deref().unwrap_or(&t.url)),
                    "favicon": t.favicon,
                    "loading": t.loading && t.pending.is_none(),
                    "active": i == win.active,
                    "internal": t.is_internal(),
                })
            })
            .collect();
        let active = win.active_tab();
        let url = active.map(|t| t.url.clone()).unwrap_or_default();
        let display = if protocol::internal_page(&url).as_deref() == Some("newtab") || url == "about:blank" {
            String::new()
        } else {
            protocol::display_url(&url)
        };
        let bookmarked = recordable(&url) && self.store.bookmark_for(&url).ok().flatten().is_some();
        let bookmarks: Vec<Value> = self
            .store
            .toolbar_bookmarks()
            .unwrap_or_default()
            .into_iter()
            .map(|b| json!({ "title": b.title, "url": b.url }))
            .collect();
        let recent = self
            .last_download
            .as_ref()
            .map(|(name, state)| json!({ "name": name, "state": state }));
        let state = json!({
            "private": win.private,
            "tabs": tabs,
            "url": display,
            "internal": protocol::is_internal(&url),
            "canBack": active.map(|t| t.webview.can_go_back().unwrap_or(false)).unwrap_or(false),
            "canForward": active.map(|t| t.webview.can_go_forward().unwrap_or(false)).unwrap_or(false),
            "loading": active.map(|t| t.loading).unwrap_or(false),
            "bookmarked": bookmarked,
            "zoom": active.map(|t| t.zoom).unwrap_or(1.0),
            "defaultZoom": self.settings.default_zoom,
            "bookmarks": bookmarks,
            "showBookmarksBar": self.settings.show_bookmarks_bar,
            "downloads": { "active": self.downloads_active, "recent": recent },
            "searchEngine": self.settings.search_engine_name(),
        });
        win.eval_chrome(&format!("ui.state({state})"));
    }

    fn push_all(&mut self) {
        let ids: Vec<WindowId> = self.windows.iter().map(|w| w.id).collect();
        for id in ids {
            self.push_state(id);
        }
    }

    /// Tells open browser pages that some data changed, so they can reload it.
    fn broadcast(&self, topic: &str) {
        let js = format!(
            "window.dispatchEvent(new CustomEvent('browser:changed', {{ detail: {} }}))",
            js_string(topic)
        );
        for win in &self.windows {
            for tab in win.tabs.iter().filter(|t| t.is_internal()) {
                let _ = tab.webview.evaluate_script(&js);
            }
        }
    }

    /* Events */

    pub fn handle_user_event(&mut self, target: &EventLoopWindowTarget<UserEvent>, event: UserEvent) {
        match event {
            UserEvent::Chrome { window, body } => match serde_json::from_str::<ChromeMessage>(&body) {
                Ok(msg) => self.on_chrome(target, window, msg),
                Err(e) => eprintln!("bad toolbar message: {e}"),
            },
            UserEvent::Page {
                window,
                tab,
                sender,
                body,
            } => {
                // Pages can post anything; malformed messages are dropped.
                if let Ok(envelope) = serde_json::from_str::<ipc::PageEnvelope>(&body) {
                    let tab = self.route(window, tab, envelope.href.as_deref());
                    self.on_page(target, window, tab, &sender, envelope.message)
                }
            }
            UserEvent::Title { window, tab, title } => self.on_title(window, tab, title),
            UserEvent::Load {
                window,
                tab,
                started,
                url,
            } => self.on_load(window, tab, started, &url),
            UserEvent::External { window, url } => self.on_external(window, url),
            UserEvent::OpenTab { window, url } => self.open_tab(window, &url, true),
            UserEvent::DownloadStarted { private, url, path } => {
                if !private {
                    let _ = self.store.add_download(&url, &path.to_string_lossy());
                }
                self.downloads_active += 1;
                self.last_download = Some((file_name(&path), "downloading".into()));
                self.push_all();
                self.broadcast("downloads");
            }
            UserEvent::DownloadFinished { private, url, path, ok } => {
                if !private {
                    let p = path.as_ref().map(|p| p.to_string_lossy().into_owned());
                    let _ = self.store.finish_download(&url, p.as_deref(), ok);
                }
                self.downloads_active = self.downloads_active.saturating_sub(1);
                let name = path
                    .as_deref()
                    .map(file_name)
                    .or_else(|| self.last_download.as_ref().map(|d| d.0.clone()));
                self.last_download = Some((name.unwrap_or_default(), if ok { "completed" } else { "failed" }.into()));
                self.push_all();
                self.broadcast("downloads");
            }
            UserEvent::Menu(id) => {
                let window = self.menu_window.or_else(|| self.windows.first().map(|w| w.id));
                if let (Some(window), Ok(action)) = (window, Action::from_str(&id)) {
                    self.run_action(target, window, action);
                }
            }
        }
    }

    fn on_chrome(&mut self, target: &EventLoopWindowTarget<UserEvent>, window: WindowId, msg: ChromeMessage) {
        match msg {
            ChromeMessage::Ready => {
                let Some(win) = self.window_mut(window) else { return };
                win.chrome_ready = true;
                let blank = win
                    .active_tab()
                    .map(|t| protocol::internal_page(&t.url).as_deref() == Some("newtab"));
                self.push_state(window);
                if blank == Some(true) {
                    self.focus_address_bar(window);
                }
            }
            ChromeMessage::Layout { height } => {
                if let Some(win) = self.window_mut(window) {
                    win.chrome_height = height.clamp(40.0, 400.0);
                    win.layout();
                }
            }
            ChromeMessage::Overlay { height } => {
                if let Some(win) = self.window_mut(window) {
                    win.overlay_height = height.clamp(0.0, 600.0);
                    win.layout();
                }
            }
            ChromeMessage::Navigate { input, new_tab } => {
                let Some(url) = urlbar::interpret(&input, &self.settings) else {
                    return;
                };
                if new_tab {
                    self.open_tab(window, &url, true);
                } else {
                    self.load_in_active(window, &url);
                }
                self.focus_page(window);
            }
            ChromeMessage::OpenUrl { url, new_tab } => {
                let Some(url) = urlbar::interpret(&url, &self.settings) else {
                    return;
                };
                if new_tab {
                    self.open_tab(window, &url, false);
                } else {
                    self.load_in_active(window, &url);
                    self.focus_page(window);
                }
            }
            ChromeMessage::Action { name } => match Action::from_str(&name) {
                Ok(action) => self.run_action(target, window, action),
                Err(e) => eprintln!("{e}"),
            },
            ChromeMessage::SelectTab { id } => {
                if let Some(win) = self.window_mut(window) {
                    if let Some(i) = win.tab_index(id) {
                        self.select_tab(window, i);
                    }
                }
            }
            ChromeMessage::CloseTab { id } => {
                if let Some(i) = self.window(window).and_then(|w| w.tab_index(id)) {
                    self.close_tab(window, i);
                }
            }
            ChromeMessage::MoveTab { id, index } => {
                if let Some(win) = self.window_mut(window) {
                    if let Some(from) = win.tab_index(id) {
                        let active_id = win.active_tab().map(|t| t.id);
                        let tab = win.tabs.remove(from);
                        let to = index.min(win.tabs.len());
                        win.tabs.insert(to, tab);
                        if let Some(a) = active_id.and_then(|a| win.tab_index(a)) {
                            win.active = a;
                        }
                        self.session_dirty = true;
                        self.push_state(window);
                    }
                }
            }
            ChromeMessage::Suggest { text } => self.suggest(window, &text),
            ChromeMessage::Find { query, backwards, .. } => {
                if let Some(win) = self.window_mut(window) {
                    win.find_query = query.clone();
                    if let Some(tab) = win.active_tab() {
                        let js = format!(
                            "window.__browserFind && window.__browserFind({}, {backwards})",
                            js_string(&query)
                        );
                        let _ = tab.webview.evaluate_script(&js);
                    }
                }
            }
            ChromeMessage::FindClose => self.close_find(window),
            ChromeMessage::Menu { x, y } => {
                self.menu_window = Some(window);
                if let Some(win) = self.window(window) {
                    self.menu.show(&win.window, x, y);
                }
            }
            ChromeMessage::Infobar { id, accept } => {
                let Some(win) = self.window_mut(window) else { return };
                if let Some((pending, url)) = win.infobar.take() {
                    if pending == id && accept {
                        if let Err(e) = open::that_detached(&url) {
                            eprintln!("could not open {url}: {e}");
                        }
                    }
                }
                win.eval_chrome("ui.infobar(null)");
            }
            ChromeMessage::FocusPage => self.focus_page(window),
        }
    }

    /// Finds the tab a page message really came from.
    ///
    /// On Linux a popup created as a linked view shares its opener's message handler, so its
    /// messages arrive labelled with the opener's tab. Pages include their own address, which
    /// identifies the popup among the opener's children.
    fn route(&self, window: WindowId, tab: TabId, href: Option<&str>) -> TabId {
        let (Some(win), Some(href)) = (self.window(window), href) else {
            return tab;
        };
        let same = |a: &str, b: &str| a.split('#').next() == b.split('#').next();
        if win.tabs.iter().any(|t| t.id == tab && same(&t.url, href)) {
            return tab;
        }
        win.tabs
            .iter()
            .rev()
            .find(|t| t.opener == Some(tab) && same(&t.url, href))
            .map(|t| t.id)
            .unwrap_or(tab)
    }

    fn on_page(
        &mut self,
        target: &EventLoopWindowTarget<UserEvent>,
        window: WindowId,
        tab: TabId,
        sender: &str,
        msg: PageMessage,
    ) {
        let internal = protocol::is_internal(sender);
        if !msg.allowed_from(internal) {
            eprintln!("ignored a privileged request from {sender}");
            return;
        }
        match msg {
            PageMessage::Shortcut { action } => {
                if let Ok(action) = Action::from_str(&action) {
                    if internal || action.allowed_from_web_page() {
                        self.run_action(target, window, action);
                    }
                }
            }
            PageMessage::Location => self.refresh_url(window, tab, true),
            PageMessage::Favicon { url } => {
                if let (Some(url), Some(win)) = (ipc::safe_favicon(&url), self.window_mut(window)) {
                    if let Some(i) = win.tab_index(tab) {
                        win.tabs[i].favicon = url;
                        self.push_state(window);
                    }
                }
            }
            PageMessage::FindResult { current, total } => {
                if let Some(win) = self.window(window) {
                    if win.active_tab().map(|t| t.id) == Some(tab) {
                        win.eval_chrome(&format!("ui.findResult({current}, {total})"));
                    }
                }
            }
            PageMessage::Fullscreen { on } => {
                if let Some(win) = self.window_mut(window) {
                    if win.active_tab().map(|t| t.id) == Some(tab) {
                        win.page_fullscreen = on;
                        win.window.set_fullscreen(on.then_some(Fullscreen::Borderless(None)));
                        win.layout();
                    }
                }
            }
            PageMessage::Close => {
                if let Some(i) = self.window(window).and_then(|w| w.tab_index(tab)) {
                    if self.window(window).map(|w| w.tabs[i].popup).unwrap_or(false) {
                        self.close_tab(window, i);
                    }
                }
            }
            PageMessage::Request { id, method, params } => {
                let result = self.on_request(target, window, tab, &method, &params);
                let (ok, data) = match result {
                    Ok(v) => (true, v),
                    Err(e) => (false, Value::String(e)),
                };
                if let Some(t) = self.window(window).and_then(|w| w.tabs.iter().find(|t| t.id == tab)) {
                    let _ = t
                        .webview
                        .evaluate_script(&format!("window.__browserResolve({id}, {ok}, {data})"));
                }
            }
        }
    }

    fn on_title(&mut self, window: WindowId, tab: TabId, title: String) {
        let Some(win) = self.window_mut(window) else { return };
        let Some(i) = win.tab_index(tab) else { return };
        let private = win.private;
        win.tabs[i].title = title.clone();
        let url = win.tabs[i].url.clone();
        if i == win.active {
            win.update_title();
        }
        if !private && recordable(&url) && !title.is_empty() {
            let _ = self.store.update_title(&url, &title);
        }
        self.session_dirty = true;
        self.push_state(window);
    }

    fn on_load(&mut self, window: WindowId, tab: TabId, started: bool, url: &str) {
        let zoom = self.zoom_for(url);
        let Some(win) = self.window_mut(window) else { return };
        let Some(i) = win.tab_index(tab) else { return };
        let t = &mut win.tabs[i];
        if t.pending.is_some() {
            return;
        }
        t.loading = started;
        if started {
            let old_host = host_of(&t.url);
            if !url.is_empty() {
                t.url = url.to_string();
                t.shared.set_url(url);
            }
            if host_of(&t.url) != old_host || protocol::is_internal(&t.url) {
                t.favicon.clear();
            }
            if (zoom - t.zoom).abs() > f64::EPSILON {
                t.zoom = zoom;
                let _ = t.webview.zoom(zoom);
            }
            self.push_state(window);
        } else {
            self.refresh_url(window, tab, false);
            if let Some(win) = self.window(window) {
                if win.find_open && win.active_tab().map(|t| t.id) == Some(tab) && !win.find_query.is_empty() {
                    let js = format!("window.__browserFind({}, false)", js_string(&win.find_query));
                    if let Some(t) = win.active_tab() {
                        let _ = t.webview.evaluate_script(&js);
                    }
                }
            }
        }
    }

    /// The zoom level for a page: the level remembered for its site, or the default.
    fn zoom_for(&self, url: &str) -> f64 {
        host_of(url)
            .filter(|_| recordable(url))
            .and_then(|h| self.store.site_zoom(&h).ok().flatten())
            .unwrap_or(self.settings.default_zoom)
    }

    /// Reads the tab's real URL from the engine and records a visit when it changed, or when a
    /// page load finished.
    fn refresh_url(&mut self, window: WindowId, tab: TabId, spa: bool) {
        let Some(win) = self.window_mut(window) else { return };
        let private = win.private;
        let Some(i) = win.tab_index(tab) else { return };
        let t = &mut win.tabs[i];
        let Ok(url) = t.webview.url() else { return };
        let changed = url != t.url;
        if url.is_empty() {
            return;
        }
        t.url = url.clone();
        t.shared.set_url(&url);
        let title = t.title.clone();
        if !private && recordable(&url) && (!spa || changed) {
            let _ = self.store.record_visit(&url, &title, now_ms());
        }
        self.session_dirty = true;
        self.push_state(window);
    }

    fn on_external(&mut self, window: WindowId, url: String) {
        let id = self.next_prompt;
        self.next_prompt += 1;
        let scheme = url.split(':').next().unwrap_or("").to_string();
        let Some(win) = self.window_mut(window) else { return };
        win.infobar = Some((id, url));
        let info = json!({
            "id": id,
            "message": format!("This page wants to open a “{scheme}:” link in another application."),
            "accept": "Open link",
            "cancel": "Cancel",
        });
        win.eval_chrome(&format!("ui.infobar({info})"));
    }

    fn suggest(&mut self, window: WindowId, text: &str) {
        let mut items: Vec<Value> = Vec::new();
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            let target = urlbar::interpret(trimmed, &self.settings).unwrap_or_default();
            let is_search = target == urlbar::search_url(&self.settings, trimmed.trim_start_matches('?'));
            if is_search {
                items.push(json!({
                    "type": "search",
                    "title": format!("{trimmed} — Search with {}", self.settings.search_engine_name()),
                    "input": trimmed,
                    "fill": trimmed,
                }));
            } else {
                items.push(json!({ "type": "url", "title": protocol::display_url(&target), "url": "", "input": trimmed, "fill": trimmed }));
            }
            let found = self
                .store
                .suggest(
                    trimmed,
                    self.settings.suggest_history,
                    self.settings.suggest_bookmarks,
                    8,
                )
                .unwrap_or_default();
            for s in found {
                items.push(json!({
                    "type": if s.bookmarked { "bookmark" } else { "history" },
                    "title": if s.title.is_empty() { s.url.clone() } else { s.title },
                    "url": s.url,
                    "input": s.url,
                    "fill": s.url,
                }));
            }
        }
        if let Some(win) = self.window(window) {
            win.eval_chrome(&format!("ui.suggestions({}, {})", js_string(text), Value::Array(items)));
        }
    }

    fn load_in_active(&mut self, window: WindowId, url: &str) {
        let Some(win) = self.window_mut(window) else { return };
        match win.active_tab_mut() {
            Some(tab) => {
                tab.pending = None;
                tab.url = url.to_string();
                tab.load(url);
            }
            None => {
                self.open_tab(window, url, true);
                return;
            }
        }
        self.push_state(window);
    }

    fn select_tab(&mut self, window: WindowId, index: usize) {
        let Some(win) = self.window_mut(window) else { return };
        if index >= win.tabs.len() {
            return;
        }
        win.activate(index);
        win.layout();
        win.update_title();
        let newtab = protocol::internal_page(&win.tabs[index].url).as_deref() == Some("newtab");
        if win.find_open {
            win.find_open = false;
            win.eval_chrome("ui.closeFind()");
        }
        self.session_dirty = true;
        self.push_state(window);
        if newtab {
            self.focus_address_bar(window);
        } else {
            self.focus_page(window);
        }
    }

    fn focus_page(&self, window: WindowId) {
        if let Some(tab) = self.window(window).and_then(|w| w.active_tab()) {
            let _ = tab.webview.focus();
        }
    }

    fn focus_address_bar(&self, window: WindowId) {
        if let Some(win) = self.window(window) {
            let _ = win.chrome.focus();
            win.eval_chrome("ui.focusAddressBar()");
        }
    }

    fn close_find(&mut self, window: WindowId) {
        let Some(win) = self.window_mut(window) else { return };
        win.find_open = false;
        win.eval_chrome("ui.closeFind()");
        if let Some(tab) = win.active_tab() {
            let _ = tab
                .webview
                .evaluate_script("window.__browserFind && window.__browserFind(null)");
        }
        self.focus_page(window);
    }

    /// Opens a browser page, switching to it if it is already open in this window.
    fn show_page(&mut self, window: WindowId, page: &str) {
        let existing = self.window(window).and_then(|w| {
            w.tabs
                .iter()
                .position(|t| protocol::internal_page(&t.url).as_deref() == Some(page))
        });
        match existing {
            Some(i) => self.select_tab(window, i),
            None => self.open_tab(window, &protocol::page_url(page), true),
        }
    }

    fn set_zoom(&mut self, window: WindowId, change: i32) {
        let default = self.settings.default_zoom;
        let Some(win) = self.window_mut(window) else { return };
        let private = win.private;
        let Some(tab) = win.active_tab_mut() else { return };
        let zoom = match change {
            0 => default,
            _ => {
                let current = ZOOM_LEVELS
                    .iter()
                    .position(|z| (z - tab.zoom).abs() < 0.01)
                    .map(|i| i as i32);
                let index = match current {
                    Some(i) => (i + change).clamp(0, ZOOM_LEVELS.len() as i32 - 1),
                    None => {
                        ZOOM_LEVELS
                            .iter()
                            .position(|z| *z > tab.zoom)
                            .unwrap_or(ZOOM_LEVELS.len() - 1) as i32
                            - if change < 0 { 1 } else { 0 }
                    }
                };
                ZOOM_LEVELS[index.clamp(0, ZOOM_LEVELS.len() as i32 - 1) as usize]
            }
        };
        tab.zoom = zoom;
        let _ = tab.webview.zoom(zoom);
        let url = tab.url.clone();
        if !private && recordable(&url) {
            if let Some(host) = host_of(&url) {
                let _ = self.store.set_site_zoom(&host, zoom, default);
            }
        }
        self.push_state(window);
    }

    fn toggle_bookmark(&mut self, window: WindowId) {
        let Some(tab) = self.window(window).and_then(|w| w.active_tab()) else {
            return;
        };
        let url = tab.url.clone();
        if !recordable(&url) && !url.starts_with("file:") {
            return;
        }
        let title = tab.display_title();
        match self.store.bookmark_for(&url) {
            Ok(Some(id)) => {
                let _ = self.store.delete_bookmark(id);
            }
            Ok(None) => {
                let _ = self.store.add_bookmark(&url, &title, true);
            }
            Err(e) => eprintln!("bookmark error: {e}"),
        }
        self.push_all();
        self.broadcast("bookmarks");
    }

    pub fn run_action(&mut self, target: &EventLoopWindowTarget<UserEvent>, window: WindowId, action: Action) {
        let tab_count = self.window(window).map(|w| w.tabs.len()).unwrap_or(0);
        let active = self.window(window).map(|w| w.active).unwrap_or(0);
        match action {
            Action::NewTab => {
                let url = self.newtab_url();
                self.add_tab(window, Start::Url(url), None, true);
                if let Some(win) = self.window(window) {
                    win.layout();
                    win.update_title();
                }
                self.push_state(window);
                self.focus_address_bar(window);
            }
            Action::CloseTab => self.close_tab(window, active),
            Action::ReopenClosedTab => {
                let closed = self.window_mut(window).and_then(|w| w.closed.pop());
                if let Some(t) = closed {
                    if let Some(url) = urlbar::interpret(&t.url, &self.settings) {
                        self.add_tab(window, Start::Url(url), None, true);
                        self.layout(window);
                        self.push_state(window);
                    }
                }
            }
            Action::NextTab if tab_count > 0 => self.select_tab(window, (active + 1) % tab_count),
            Action::PreviousTab if tab_count > 0 => self.select_tab(window, (active + tab_count - 1) % tab_count),
            Action::SelectTab(n) if n < tab_count => self.select_tab(window, n),
            Action::SelectLastTab if tab_count > 0 => self.select_tab(window, tab_count - 1),
            Action::FocusAddressBar => self.focus_address_bar(window),
            Action::Reload => {
                if let Some(tab) = self.window(window).and_then(|w| w.active_tab()) {
                    let _ = tab.webview.reload();
                }
            }
            Action::Stop => {
                if let Some(tab) = self.window(window).and_then(|w| w.active_tab()).filter(|t| t.loading) {
                    let _ = tab.webview.evaluate_script("window.stop()");
                }
            }
            Action::Back => {
                if let Some(tab) = self.window(window).and_then(|w| w.active_tab()) {
                    let _ = tab.webview.go_back();
                }
            }
            Action::Forward => {
                if let Some(tab) = self.window(window).and_then(|w| w.active_tab()) {
                    let _ = tab.webview.go_forward();
                }
            }
            Action::Home => {
                let url = self.home_url();
                self.load_in_active(window, &url);
            }
            Action::BookmarkPage => self.toggle_bookmark(window),
            Action::ToggleBookmarksBar => {
                self.settings.show_bookmarks_bar = !self.settings.show_bookmarks_bar;
                let _ = self.settings.save(&self.profile.settings());
                self.push_all();
            }
            Action::ShowBookmarks => self.show_page(window, "bookmarks"),
            Action::ShowHistory => self.show_page(window, "history"),
            Action::ShowDownloads => self.show_page(window, "downloads"),
            Action::ShowSettings => self.show_page(window, "settings"),
            Action::Find => {
                if let Some(win) = self.window_mut(window) {
                    win.find_open = true;
                    let _ = win.chrome.focus();
                    win.eval_chrome("ui.openFind()");
                }
            }
            Action::FindNext | Action::FindPrevious => {
                let backwards = action == Action::FindPrevious;
                let Some(win) = self.window(window) else { return };
                if win.find_query.is_empty() {
                    self.run_action(target, window, Action::Find);
                } else if let Some(tab) = win.active_tab() {
                    let js = format!("window.__browserFind({}, {backwards})", js_string(&win.find_query));
                    let _ = tab.webview.evaluate_script(&js);
                }
            }
            Action::Print => {
                if let Some(tab) = self.window(window).and_then(|w| w.active_tab()) {
                    let _ = tab.webview.print();
                }
            }
            Action::ZoomIn => self.set_zoom(window, 1),
            Action::ZoomOut => self.set_zoom(window, -1),
            Action::ZoomReset => self.set_zoom(window, 0),
            Action::Fullscreen => {
                if let Some(win) = self.window(window) {
                    let on = win.window.fullscreen().is_none();
                    win.window.set_fullscreen(on.then_some(Fullscreen::Borderless(None)));
                }
            }
            Action::DevTools => {
                if let Some(tab) = self.window(window).and_then(|w| w.active_tab()) {
                    if tab.webview.is_devtools_open() {
                        tab.webview.close_devtools();
                    } else {
                        tab.webview.open_devtools();
                    }
                }
            }
            Action::NewWindow | Action::NewPrivateWindow => {
                let private = action == Action::NewPrivateWindow;
                let url = if private { self.newtab_url() } else { self.home_url() };
                self.open_window(target, private, vec![url], 0, None);
            }
            Action::CloseWindow => self.close_window(window),
            Action::Quit => self.quit_requested = true,
            Action::NextTab | Action::PreviousTab | Action::SelectTab(_) | Action::SelectLastTab => {}
        }
    }

    /* Requests from browser pages */

    fn on_request(
        &mut self,
        target: &EventLoopWindowTarget<UserEvent>,
        window: WindowId,
        tab: TabId,
        method: &str,
        p: &Value,
    ) -> Result<Value, String> {
        let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let i = |k: &str| p.get(k).and_then(Value::as_i64);
        let b = |k: &str| p.get(k).and_then(Value::as_bool).unwrap_or(false);
        let db = |e: rusqlite::Error| e.to_string();
        let _ = target;
        Ok(match method {
            "navigate" => {
                let url = urlbar::interpret(&s("input"), &self.settings).ok_or("empty input")?;
                self.load_in_tab(window, tab, &url);
                Value::Null
            }
            "open" => {
                let url = urlbar::interpret(&s("url"), &self.settings).ok_or("empty url")?;
                if b("newTab") {
                    self.open_tab(window, &url, false);
                } else {
                    self.load_in_tab(window, tab, &url);
                }
                Value::Null
            }
            "newtab.data" => {
                let sites: Vec<_> = self
                    .store
                    .top_sites(16)
                    .map_err(db)?
                    .into_iter()
                    .filter(|s| recordable(&s.url))
                    .take(8)
                    .collect();
                json!({ "engine": self.settings.search_engine_name(), "sites": sites })
            }
            "history.query" => {
                let limit = i("limit").unwrap_or(100).clamp(1, 1000) as usize;
                json!(self.store.history(&s("text"), i("before"), limit).map_err(db)?)
            }
            "history.delete" => {
                self.store.delete_visit(i("id").ok_or("missing id")?).map_err(db)?;
                Value::Null
            }
            "history.clear" => {
                let since = match s("range").as_str() {
                    "hour" => Some(now_ms() - 3_600_000),
                    "day" => Some(now_ms() - 86_400_000),
                    "week" => Some(now_ms() - 7 * 86_400_000),
                    _ => None,
                };
                self.store.clear_history(since).map_err(db)?;
                Value::Null
            }
            "bookmarks.list" => json!(self.store.bookmarks().map_err(db)?),
            "bookmarks.add" => {
                let url = urlbar::interpret(&s("url"), &self.settings).ok_or("empty url")?;
                let id = self.store.add_bookmark(&url, &s("title"), b("toolbar")).map_err(db)?;
                self.after_bookmarks_change();
                json!(id)
            }
            "bookmarks.update" => {
                let url = urlbar::interpret(&s("url"), &self.settings).ok_or("empty url")?;
                self.store
                    .update_bookmark(i("id").ok_or("missing id")?, &url, &s("title"), b("toolbar"))
                    .map_err(db)?;
                self.after_bookmarks_change();
                Value::Null
            }
            "bookmarks.move" => {
                let index = i("index").unwrap_or(0).max(0) as usize;
                self.store
                    .move_bookmark(i("id").ok_or("missing id")?, index)
                    .map_err(db)?;
                self.after_bookmarks_change();
                Value::Null
            }
            "bookmarks.delete" => {
                self.store.delete_bookmark(i("id").ok_or("missing id")?).map_err(db)?;
                self.after_bookmarks_change();
                Value::Null
            }
            "downloads.list" => {
                let downloads: Vec<Value> = self
                    .store
                    .downloads()
                    .map_err(db)?
                    .into_iter()
                    .map(|d| {
                        let meta = std::fs::metadata(&d.path).ok();
                        let mut v = json!(d);
                        v["size"] = json!(meta.as_ref().map(|m| m.len()));
                        v["missing"] = json!(d.state == "completed" && meta.is_none());
                        v
                    })
                    .collect();
                json!({ "downloads": downloads, "folder": self.effective_download_dir().to_string_lossy() })
            }
            "downloads.open" | "downloads.showInFolder" => {
                let d = self
                    .store
                    .download(i("id").ok_or("missing id")?)
                    .map_err(db)?
                    .ok_or("unknown download")?;
                if method == "downloads.open" {
                    open::that_detached(&d.path).map_err(|e| e.to_string())?;
                } else {
                    show_in_folder(std::path::Path::new(&d.path))?;
                }
                Value::Null
            }
            "downloads.openFolder" => {
                open::that_detached(self.effective_download_dir()).map_err(|e| e.to_string())?;
                Value::Null
            }
            "downloads.remove" => {
                self.store.remove_download(i("id").ok_or("missing id")?).map_err(db)?;
                Value::Null
            }
            "downloads.clear" => {
                self.store.clear_downloads().map_err(db)?;
                self.last_download = None;
                self.push_all();
                Value::Null
            }
            "settings.get" => {
                let engines: Vec<Value> = SEARCH_ENGINES.iter().map(|(id, name, _)| json!([id, name])).collect();
                json!({
                    "settings": self.settings,
                    "engines": engines,
                    "engine": wry::webview_version().map(|v| engine_label(&v)).unwrap_or_else(|_| "unknown".into()),
                    "profile": self.profile.root.to_string_lossy(),
                    "defaultDownloadDir": paths::default_download_dir().to_string_lossy(),
                })
            }
            "settings.set" => {
                let next: Settings = serde_json::from_value(p.get("settings").cloned().unwrap_or_default())
                    .map_err(|e| e.to_string())?;
                let next = next.sanitized();
                let zoom_changed = (next.default_zoom - self.settings.default_zoom).abs() > f64::EPSILON;
                self.settings = next;
                self.settings
                    .save(&self.profile.settings())
                    .map_err(|e| e.to_string())?;
                *self.download_dir.borrow_mut() = Self::download_dir_setting(&self.settings);
                if zoom_changed {
                    self.reapply_zoom();
                }
                self.push_all();
                json!(self.settings)
            }
            "data.clear" => {
                match s("what").as_str() {
                    "siteData" => {
                        let webview = self.windows.iter().find(|w| !w.private).map(|w| &w.chrome);
                        if let Some(wv) = webview {
                            wv.clear_all_browsing_data().map_err(|e| e.to_string())?;
                        }
                    }
                    "history" => {
                        self.store.clear_history(None).map_err(db)?;
                        self.broadcast("history");
                    }
                    "zoom" => {
                        self.store.clear_site_zoom().map_err(db)?;
                        self.reapply_zoom();
                    }
                    other => return Err(format!("unknown data kind {other}")),
                }
                Value::Null
            }
            other => return Err(format!("unknown method {other}")),
        })
    }

    fn load_in_tab(&mut self, window: WindowId, tab: TabId, url: &str) {
        if let Some(win) = self.window_mut(window) {
            if let Some(i) = win.tab_index(tab) {
                win.tabs[i].url = url.to_string();
                win.tabs[i].load(url);
            }
        }
        self.push_state(window);
    }

    fn after_bookmarks_change(&mut self) {
        self.push_all();
        self.broadcast("bookmarks");
    }

    fn reapply_zoom(&mut self) {
        let levels: Vec<Vec<f64>> = self
            .windows
            .iter()
            .map(|w| w.tabs.iter().map(|t| self.zoom_for(&t.url)).collect())
            .collect();
        for (win, zooms) in self.windows.iter_mut().zip(levels) {
            for (tab, zoom) in win.tabs.iter_mut().zip(zooms) {
                tab.zoom = zoom;
                let _ = tab.webview.zoom(zoom);
            }
        }
    }

    fn effective_download_dir(&self) -> PathBuf {
        self.download_dir
            .borrow()
            .clone()
            .unwrap_or_else(paths::default_download_dir)
    }
}

/// A readable name for the engine version reported by wry.
fn engine_label(version: &str) -> String {
    if cfg!(windows) {
        format!("Microsoft Edge WebView2 {version}")
    } else if cfg!(target_os = "macos") {
        format!("WebKit (WKWebView) {version}")
    } else {
        format!("WebKitGTK {version}")
    }
}

/// Opens the folder containing `path`, selecting the file where the platform allows it.
fn show_in_folder(path: &std::path::Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", path.display()))
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let folder = path.parent().ok_or("no parent folder")?;
        open::that_detached(folder).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_pages_are_recorded() {
        assert!(recordable("https://example.com/"));
        assert!(!recordable("about:blank"));
        assert!(!recordable("http://browser.app/history"));
        assert!(!recordable("file:///home/me/a.html"));
    }

    #[test]
    fn zoom_levels_include_the_default() {
        assert!(ZOOM_LEVELS.contains(&1.0));
        assert!(ZOOM_LEVELS.windows(2).all(|w| w[0] < w[1]));
    }
}
