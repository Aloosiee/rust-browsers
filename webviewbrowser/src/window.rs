//! A browser window: the native window, its toolbar webview and its tabs.

use crate::event::UserEvent;
use crate::session::TabState;
use crate::tab::{ContextSlot, Host, Tab, TabId};
use tao::dpi::LogicalSize;
use tao::event_loop::{EventLoopProxy, EventLoopWindowTarget};
use tao::window::{Window, WindowBuilder, WindowId};
use wry::dpi::{LogicalPosition, LogicalSize as WryLogicalSize};
use wry::{Rect, WebView};

/// Toolbar height before the toolbar reports its real size: tab strip plus navigation bar.
const DEFAULT_CHROME_HEIGHT: f64 = 84.0;
const BOOKMARKS_BAR_HEIGHT: f64 = 32.0;

pub struct BrowserWindow {
    pub id: WindowId,
    pub window: Window,
    #[cfg(target_os = "linux")]
    pub container: gtk::Fixed,
    pub chrome: WebView,
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub private: bool,
    pub chrome_ready: bool,
    pub chrome_height: f64,
    pub overlay_height: f64,
    pub page_fullscreen: bool,
    pub find_open: bool,
    pub find_query: String,
    /// Recently closed tabs, newest last, for Ctrl+Shift+T.
    pub closed: Vec<TabState>,
    /// External-protocol link waiting for the user to allow it: (prompt id, URL).
    pub infobar: Option<(u64, String)>,
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect {
        position: LogicalPosition::new(x, y).into(),
        size: WryLogicalSize::new(w.max(1.0), h.max(1.0)).into(),
    }
}

impl BrowserWindow {
    pub fn new(
        target: &EventLoopWindowTarget<UserEvent>,
        proxy: &EventLoopProxy<UserEvent>,
        chrome_slot: &mut ContextSlot,
        private: bool,
        size: (f64, f64),
        maximized: bool,
        bookmarks_bar: bool,
    ) -> Result<BrowserWindow, String> {
        let window = WindowBuilder::new()
            .with_title(if private {
                "Private Browsing — Web Browser"
            } else {
                "Web Browser"
            })
            .with_inner_size(LogicalSize::new(size.0, size.1))
            .with_min_inner_size(LogicalSize::new(480.0, 320.0))
            .with_maximized(maximized)
            .build(target)
            .map_err(|e| e.to_string())?;

        #[cfg(target_os = "linux")]
        let container = {
            use gtk::prelude::*;
            use tao::platform::unix::WindowExtUnix;
            let fixed = gtk::Fixed::new();
            let vbox = window.default_vbox().ok_or("window has no GTK box")?;
            vbox.pack_start(&fixed, true, true, 0);
            fixed.show_all();
            fixed
        };

        let chrome_height = DEFAULT_CHROME_HEIGHT + if bookmarks_bar { BOOKMARKS_BAR_HEIGHT } else { 0.0 };
        let logical = window.inner_size().to_logical::<f64>(window.scale_factor());
        let host = Host {
            window: &window,
            #[cfg(target_os = "linux")]
            container: &container,
            window_id: window.id(),
            proxy,
            bounds: rect(0.0, 0.0, logical.width, chrome_height),
        };
        let chrome = crate::tab::build_chrome(&host, chrome_slot).map_err(|e| e.to_string())?;

        Ok(BrowserWindow {
            id: window.id(),
            window,
            #[cfg(target_os = "linux")]
            container,
            chrome,
            tabs: Vec::new(),
            active: 0,
            private,
            chrome_ready: false,
            chrome_height,
            overlay_height: 0.0,
            page_fullscreen: false,
            find_open: false,
            find_query: String::new(),
            closed: Vec::new(),
            infobar: None,
        })
    }

    /// Where a new tab's webview goes.
    pub fn host<'a>(&'a self, proxy: &'a EventLoopProxy<UserEvent>) -> Host<'a> {
        Host {
            window: &self.window,
            #[cfg(target_os = "linux")]
            container: &self.container,
            window_id: self.id,
            proxy,
            bounds: self.content_bounds(),
        }
    }

    fn logical_size(&self) -> (f64, f64) {
        let size = self.window.inner_size().to_logical::<f64>(self.window.scale_factor());
        (size.width, size.height)
    }

    fn content_top(&self) -> f64 {
        if self.page_fullscreen {
            0.0
        } else {
            self.chrome_height + self.overlay_height
        }
    }

    pub fn content_bounds(&self) -> Rect {
        let (w, h) = self.logical_size();
        let top = self.content_top();
        rect(0.0, top, w, h - top)
    }

    /// Positions the toolbar and the active tab for the current window size.
    pub fn layout(&self) {
        let (w, _) = self.logical_size();
        let _ = self.chrome.set_visible(!self.page_fullscreen);
        let _ = self
            .chrome
            .set_bounds(rect(0.0, 0.0, w, self.chrome_height + self.overlay_height));
        if let Some(tab) = self.tabs.get(self.active) {
            let _ = tab.webview.set_bounds(self.content_bounds());
        }
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active)
    }

    pub fn tab_index(&self, id: TabId) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    /// Shows the tab at `index` and hides the others.
    pub fn activate(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        if let Some(old) = self.tabs.get(self.active).filter(|_| self.active != index) {
            let _ = old.webview.set_visible(false);
        }
        self.active = index;
        let bounds = self.content_bounds();
        let tab = &mut self.tabs[index];
        let _ = tab.webview.set_bounds(bounds);
        let _ = tab.webview.set_visible(true);
        if let Some(url) = tab.pending.take() {
            tab.load(&url);
        }
    }

    /// Runs JavaScript in the toolbar, once it has loaded.
    pub fn eval_chrome(&self, js: &str) {
        if self.chrome_ready {
            let _ = self.chrome.evaluate_script(js);
        }
    }

    /// The window's title bar text for the active tab.
    pub fn update_title(&self) {
        let suffix = if self.private {
            "Private Browsing — Web Browser"
        } else {
            "Web Browser"
        };
        let title = match self.active_tab().map(|t| t.display_title()) {
            Some(t) if !t.is_empty() => format!("{t} — {suffix}"),
            _ => suffix.to_string(),
        };
        self.window.set_title(&title);
    }

    pub fn size_for_session(&self) -> (f64, f64, bool) {
        let (w, h) = self.logical_size();
        (w, h, self.window.is_maximized())
    }
}
