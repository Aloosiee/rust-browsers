//! Creating the webviews that host web pages and the toolbar.

use crate::event::UserEvent;
use crate::{paths, protocol, urlbar};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use tao::event_loop::EventLoopProxy;
use tao::window::{Window, WindowId};
use wry::{NewWindowFeatures, NewWindowResponse, PageLoadEvent, Rect, WebContext, WebView, WebViewBuilder};

pub type TabId = u64;

/// JavaScript run in the main frame of every page before the page's own scripts.
pub fn page_init_script() -> String {
    format!(
        "(() => {{\n{}\n{}\n}})();",
        include_str!("../ui/shortcuts.js"),
        include_str!("../ui/inject.js")
    )
}

/// State shared between a tab and the callbacks of its webview.
#[derive(Default)]
pub struct TabShared {
    /// The page's current URL. A `Mutex` because wry requires permission handlers to be `Send`.
    pub url: Arc<Mutex<String>>,
    /// Set right before the browser itself loads a `browser:` page into the tab.
    pub allow_internal: std::cell::Cell<bool>,
}

impl TabShared {
    pub fn url(&self) -> String {
        self.url.lock().map(|u| u.clone()).unwrap_or_default()
    }

    pub fn set_url(&self, url: &str) {
        if let Ok(mut u) = self.url.lock() {
            *u = url.to_string();
        }
    }
}

/// A tab: one webview plus what the toolbar shows about it.
pub struct Tab {
    pub id: TabId,
    pub webview: WebView,
    pub shared: Rc<TabShared>,
    pub title: String,
    pub url: String,
    pub favicon: String,
    pub loading: bool,
    pub zoom: f64,
    /// A restored tab's address, loaded the first time the tab is shown.
    pub pending: Option<String>,
    /// Opened by a page with `window.open`, so the page may close it again.
    pub popup: bool,
    /// The tab whose page opened this popup.
    pub opener: Option<TabId>,
}

impl Tab {
    /// Loads a URL, allowing it to be a `browser:` page.
    pub fn load(&self, url: &str) {
        if protocol::is_internal(url) {
            self.shared.allow_internal.set(true);
        }
        if let Err(e) = self.webview.load_url(url) {
            eprintln!("failed to load {url}: {e}");
        }
    }

    pub fn is_internal(&self) -> bool {
        protocol::is_internal(&self.url)
    }

    /// The tab's title, falling back to its address.
    pub fn display_title(&self) -> String {
        if !self.title.trim().is_empty() {
            return self.title.clone();
        }
        match protocol::internal_page(&self.url).as_deref() {
            Some("newtab") => "New Tab".into(),
            Some(p) => {
                let mut c = p.chars();
                c.next()
                    .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
                    .unwrap_or_default()
            }
            None => self.url.clone(),
        }
    }
}

/// A browsing context: cookies, cache and storage shared by a group of webviews.
///
/// On Linux, custom schemes and download handlers belong to the context rather than the
/// webview, and registering them twice is an error, so the first webview built in a context
/// registers them for all the others.
pub struct ContextSlot {
    pub context: WebContext,
    pub private: bool,
    download_dir: Rc<RefCell<Option<PathBuf>>>,
    registered: bool,
}

impl ContextSlot {
    /// `download_dir` is the user's chosen download folder, shared so settings changes apply
    /// to downloads started afterwards.
    pub fn new(data_dir: PathBuf, private: bool, download_dir: Rc<RefCell<Option<PathBuf>>>) -> ContextSlot {
        ContextSlot {
            context: WebContext::new(Some(data_dir)),
            private,
            download_dir,
            registered: false,
        }
    }

    /// The context-wide handlers a webview built now must register, if any.
    fn take_registration(&mut self) -> Option<Registration> {
        let first = !std::mem::replace(&mut self.registered, true);
        (first || !cfg!(target_os = "linux")).then(|| Registration {
            private: self.private,
            download_dir: self.download_dir.clone(),
        })
    }
}

/// Handlers that belong to a web context: the `browser:` scheme and downloads.
struct Registration {
    private: bool,
    download_dir: Rc<RefCell<Option<PathBuf>>>,
}

impl Registration {
    fn apply<'a>(self, builder: WebViewBuilder<'a>, proxy: &EventLoopProxy<UserEvent>) -> WebViewBuilder<'a> {
        let Registration { private, download_dir } = self;
        let p = proxy.clone();
        let started = move |url: String, path: &mut PathBuf| {
            if let Some(name) = path.file_name() {
                let dir = download_dir
                    .borrow()
                    .clone()
                    .unwrap_or_else(paths::default_download_dir);
                let _ = std::fs::create_dir_all(&dir);
                *path = unique_path(&dir, &name.to_string_lossy());
            }
            let _ = p.send_event(UserEvent::DownloadStarted {
                private,
                url,
                path: path.clone(),
            });
            true
        };
        let p = proxy.clone();
        let finished = move |url, path, ok| {
            let _ = p.send_event(UserEvent::DownloadFinished { private, url, path, ok });
        };
        builder
            .with_custom_protocol(protocol::SCHEME.into(), |_, req| protocol::respond(req.uri().path()))
            .with_download_started_handler(started)
            .with_download_completed_handler(finished)
    }
}

/// Where a new webview goes and what it reports to.
pub struct Host<'a> {
    pub window: &'a Window,
    #[cfg(target_os = "linux")]
    pub container: &'a gtk::Fixed,
    pub window_id: WindowId,
    pub proxy: &'a EventLoopProxy<UserEvent>,
    pub bounds: Rect,
}

impl Host<'_> {
    fn build(&self, builder: WebViewBuilder) -> wry::Result<WebView> {
        #[cfg(target_os = "linux")]
        {
            use wry::WebViewBuilderExtUnix;
            let _ = self.window;
            builder.build_gtk(self.container)
        }
        #[cfg(not(target_os = "linux"))]
        {
            builder.build_as_child(self.window)
        }
    }
}

type PopupHandler = Box<dyn Fn(String, NewWindowFeatures) -> NewWindowResponse>;

/// What to load into a new page webview.
pub enum Start {
    Url(String),
    /// A popup requested by a page; the engine loads it and keeps `window.opener` working.
    Popup(Box<NewWindowFeatures>),
}

/// Builds a webview for a tab.
#[allow(clippy::too_many_arguments)]
pub fn build_page(
    host: &Host,
    slot: &mut ContextSlot,
    id: TabId,
    start: Start,
    visible: bool,
    extensions_dir: Option<&Path>,
    popup_handler: PopupHandler,
) -> wry::Result<Tab> {
    let shared = Rc::new(TabShared::default());
    let register = slot.take_registration();
    let proxy = host.proxy.clone();
    let window = host.window_id;

    let mut builder = WebViewBuilder::new_with_web_context(&mut slot.context)
        .with_bounds(host.bounds)
        .with_visible(visible)
        .with_devtools(true)
        .with_clipboard(true)
        .with_hotkeys_zoom(false)
        .with_initialization_script_for_main_only(page_init_script(), true);

    builder = match &start {
        Start::Url(url) => {
            if protocol::is_internal(url) {
                shared.allow_internal.set(true);
            }
            shared.set_url(url);
            builder.with_url(url)
        }
        Start::Popup(_) => builder,
    };

    let p = proxy.clone();
    builder = builder.with_ipc_handler(move |req| {
        let sender = req.uri().to_string();
        let _ = p.send_event(UserEvent::Page {
            window,
            tab: id,
            sender,
            body: req.into_body(),
        });
    });

    let p = proxy.clone();
    builder = builder.with_document_title_changed_handler(move |title| {
        let _ = p.send_event(UserEvent::Title { window, tab: id, title });
    });

    let p = proxy.clone();
    builder = builder.with_on_page_load_handler(move |event, url| {
        let started = matches!(event, PageLoadEvent::Started);
        let _ = p.send_event(UserEvent::Load {
            window,
            tab: id,
            started,
            url,
        });
    });

    let p = proxy.clone();
    let nav_shared = shared.clone();
    builder = builder.with_navigation_handler(move |url| {
        if protocol::is_internal(&url) {
            return nav_shared.allow_internal.replace(false) || protocol::is_internal(&nav_shared.url());
        }
        let scheme = url.split(':').next().unwrap_or("").to_ascii_lowercase();
        if urlbar::WEB_SCHEMES.contains(&scheme.as_str()) {
            return true;
        }
        let _ = p.send_event(UserEvent::External { window, url });
        false
    });

    builder = builder.with_new_window_req_handler(popup_handler);

    if let Some(registration) = register {
        builder = registration.apply(builder, &proxy);
    }

    #[cfg(target_os = "linux")]
    {
        let url = shared.url.clone();
        builder = builder.with_permission_handler(move |kind| {
            let origin = url.lock().map(|u| u.clone()).unwrap_or_default();
            crate::permissions::ask(kind, &origin)
        });
    }

    #[cfg(windows)]
    {
        use wry::WebViewBuilderExtWindows;
        builder = builder.with_browser_accelerator_keys(false);
        if let Some(dir) = extensions_dir {
            builder = builder.with_browser_extensions_enabled(true).with_extensions_path(dir);
        }
    }
    #[cfg(not(windows))]
    let _ = extensions_dir;

    if let Start::Popup(features) = &start {
        #[cfg(target_os = "linux")]
        {
            use wry::WebViewBuilderExtUnix;
            builder = builder.with_related_view(features.opener.webview.clone());
        }
        #[cfg(windows)]
        {
            use wry::WebViewBuilderExtWindows;
            builder = builder.with_environment(features.opener.environment.clone());
        }
        #[cfg(target_os = "macos")]
        {
            use wry::WebViewBuilderExtMacos;
            builder = builder.with_webview_configuration(features.opener.target_configuration.clone());
        }
    }

    let webview = host.build(builder)?;
    let url = shared.url();
    let popup = matches!(start, Start::Popup(_));
    Ok(Tab {
        id,
        webview,
        shared,
        title: String::new(),
        url,
        favicon: String::new(),
        loading: true,
        zoom: 1.0,
        pending: None,
        popup,
        opener: None,
    })
}

/// Builds the toolbar webview.
pub fn build_chrome(host: &Host, slot: &mut ContextSlot) -> wry::Result<WebView> {
    let register = slot.take_registration();
    let proxy = host.proxy.clone();
    let window = host.window_id;
    let mut builder = WebViewBuilder::new_with_web_context(&mut slot.context)
        .with_bounds(host.bounds)
        .with_url(protocol::page_url("chrome"))
        .with_hotkeys_zoom(false)
        .with_devtools(cfg!(debug_assertions))
        .with_navigation_handler(|url| protocol::internal_page(&url).as_deref() == Some("chrome"))
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
        .with_ipc_handler(move |req| {
            let _ = proxy.send_event(UserEvent::Chrome {
                window,
                body: req.into_body(),
            });
        });
    if let Some(registration) = register {
        builder = registration.apply(builder, host.proxy);
    }
    #[cfg(windows)]
    {
        use wry::WebViewBuilderExtWindows;
        builder = builder
            .with_browser_accelerator_keys(false)
            .with_default_context_menus(false);
    }
    host.build(builder)
}

/// A path in `dir` for `name` that does not exist yet: `name`, then `name (1)`, `name (2)`...
pub fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    (1..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .expect("unbounded search")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_duplicate_downloads() {
        let dir = std::env::temp_dir().join(format!("wvb-unique-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_path(&dir, "a.zip"), dir.join("a.zip"));
        std::fs::write(dir.join("a.zip"), b"").unwrap();
        assert_eq!(unique_path(&dir, "a.zip"), dir.join("a (1).zip"));
        std::fs::write(dir.join("a (1).zip"), b"").unwrap();
        assert_eq!(unique_path(&dir, "a.zip"), dir.join("a (2).zip"));
        std::fs::write(dir.join("README"), b"").unwrap();
        assert_eq!(unique_path(&dir, "README"), dir.join("README (1)"));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn init_script_keeps_helpers_out_of_the_global_scope() {
        let script = page_init_script();
        assert!(script.starts_with("(() => {"));
        assert!(script.trim_end().ends_with("})();"));
    }
}
