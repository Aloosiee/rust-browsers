//! The `browser:` scheme: built-in pages such as `browser://history`.
//!
//! wry serves custom schemes differently per platform. On Linux and macOS a page lives at
//! `browser://app/<page>`; on Windows WebView2 only allows custom schemes as
//! `http://browser.app/<page>`. The address bar always shows the short `browser://<page>` form,
//! and this module converts between the three.

use std::borrow::Cow;
use wry::http::{Response, StatusCode, header};

/// The scheme name registered with every webview.
pub const SCHEME: &str = "browser";
const HOST: &str = "app";

struct Asset {
    path: &'static str,
    mime: &'static str,
    body: &'static str,
}

const ASSETS: &[Asset] = &[
    asset("chrome", "text/html", include_str!("../ui/chrome.html")),
    asset("chrome.css", "text/css", include_str!("../ui/chrome.css")),
    asset("chrome.js", "text/javascript", include_str!("../ui/chrome.js")),
    asset("shortcuts.js", "text/javascript", include_str!("../ui/shortcuts.js")),
    asset("pages.css", "text/css", include_str!("../ui/pages/pages.css")),
    asset("pages.js", "text/javascript", include_str!("../ui/pages/pages.js")),
    asset("newtab", "text/html", include_str!("../ui/pages/newtab.html")),
    asset("history", "text/html", include_str!("../ui/pages/history.html")),
    asset("bookmarks", "text/html", include_str!("../ui/pages/bookmarks.html")),
    asset("downloads", "text/html", include_str!("../ui/pages/downloads.html")),
    asset("settings", "text/html", include_str!("../ui/pages/settings.html")),
    asset("icon.svg", "image/svg+xml", include_str!("../ui/icon.svg")),
];

const fn asset(path: &'static str, mime: &'static str, body: &'static str) -> Asset {
    Asset { path, mime, body }
}

/// The URL a webview should load for an internal page or asset on this platform.
pub fn page_url(path: &str) -> String {
    if cfg!(any(windows, target_os = "android")) {
        format!("http://{SCHEME}.{HOST}/{path}")
    } else {
        format!("{SCHEME}://{HOST}/{path}")
    }
}

/// The internal page a URL points at, accepting the loaded form on any platform and the
/// short form typed by users (`browser://settings`).
pub fn internal_page(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix(&format!("{SCHEME}://{HOST}/"))
        .or_else(|| url.strip_prefix(&format!("http://{SCHEME}.{HOST}/")))
        .or_else(|| url.strip_prefix(&format!("https://{SCHEME}.{HOST}/")))
        .or_else(|| url.strip_prefix(&format!("{SCHEME}://")))?;
    let page = rest.split(['?', '#']).next().unwrap_or("").trim_end_matches('/');
    let page = if page.is_empty() { "newtab" } else { page };
    ASSETS.iter().any(|a| a.path == page).then(|| page.to_string())
}

/// Whether a URL belongs to the browser itself rather than the web.
pub fn is_internal(url: &str) -> bool {
    internal_page(url).is_some()
}

/// The URL to show in the address bar: `browser://<page>` for internal pages.
pub fn display_url(url: &str) -> String {
    match internal_page(url) {
        Some(page) => format!("{SCHEME}://{page}"),
        None => url.to_string(),
    }
}

/// Serves a request for the `browser:` scheme.
///
/// Every response forbids framing, so a web page cannot embed a browser page and trick the
/// user into clicking inside it.
pub fn respond(path: &str) -> Response<Cow<'static, [u8]>> {
    let name = path.trim_start_matches('/').split(['?', '#']).next().unwrap_or("");
    let name = if name.is_empty() { "newtab" } else { name };
    let builder = Response::builder()
        .header("X-Frame-Options", "DENY")
        .header(
            header::CONTENT_SECURITY_POLICY,
            "frame-ancestors 'none'; default-src 'self' 'unsafe-inline'; img-src * data:",
        )
        .header(header::CACHE_CONTROL, "no-store");
    match ASSETS.iter().find(|a| a.path == name) {
        Some(a) => builder
            .header(header::CONTENT_TYPE, format!("{}; charset=utf-8", a.mime))
            .body(Cow::Borrowed(a.body.as_bytes()))
            .expect("static response"),
        None => builder
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Cow::Borrowed(b"Not found".as_slice()))
            .expect("static response"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_every_url_form() {
        for url in [
            "browser://app/history",
            "http://browser.app/history",
            "https://browser.app/history?x=1",
            "browser://history",
            "browser://history/",
        ] {
            assert_eq!(internal_page(url).as_deref(), Some("history"), "{url}");
        }
        assert_eq!(internal_page("browser://").as_deref(), Some("newtab"));
        assert_eq!(internal_page("browser://nope"), None);
        assert_eq!(internal_page("https://example.com/history"), None);
        assert_eq!(internal_page("http://browser.app.evil.com/history"), None);
    }

    #[test]
    fn displays_the_short_form() {
        assert_eq!(display_url(&page_url("settings")), "browser://settings");
        assert_eq!(display_url("https://example.com/"), "https://example.com/");
    }

    #[test]
    fn serves_assets_with_framing_blocked() {
        let r = respond("/settings");
        assert_eq!(r.status(), StatusCode::OK);
        assert!(
            r.headers()[header::CONTENT_TYPE]
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
        assert_eq!(r.headers()["X-Frame-Options"], "DENY");
        assert_eq!(respond("/missing").status(), StatusCode::NOT_FOUND);
        assert_eq!(respond("/").status(), StatusCode::OK);
    }
}
