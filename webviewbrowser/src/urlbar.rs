//! Turning address bar input into a URL, the way Firefox's address bar does.

use crate::protocol;
use crate::settings::Settings;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use std::path::Path;
use url::Url;

/// Schemes the browser loads itself. Anything else is handed to the operating system.
pub const WEB_SCHEMES: &[&str] = &[
    "http",
    "https",
    "file",
    "data",
    "blob",
    "about",
    "javascript",
    protocol::SCHEME,
];

/// Builds the search URL for a query with the user's search engine.
pub fn search_url(settings: &Settings, query: &str) -> String {
    let encoded = utf8_percent_encode(query.trim(), NON_ALPHANUMERIC).to_string();
    settings.search_template().replace("%s", &encoded)
}

/// Interprets address bar input: a URL, a host name, a local path, a `browser://` page, or a
/// search. Returns `None` for empty input.
pub fn interpret(input: &str, settings: &Settings) -> Option<String> {
    let text = input.trim();
    if text.is_empty() {
        return None;
    }
    if let Some(query) = text.strip_prefix('?') {
        return Some(search_url(settings, query));
    }
    if let Some(page) = protocol::internal_page(text) {
        return Some(protocol::page_url(&page));
    }
    if let Some(url) = local_path(text) {
        return Some(url);
    }
    if !text.contains(char::is_whitespace) {
        if let Ok(url) = Url::parse(text) {
            if url.scheme() == "about" || url.has_host() || url.scheme() == "data" || url.scheme() == "mailto" {
                return Some(url.to_string());
            }
            if url.scheme().len() > 1 && !looks_like_host(url.scheme()) && text.contains("://") {
                return Some(url.to_string());
            }
        }
        let host = text.split(['/', '?', '#']).next().unwrap_or("");
        let host_only = host
            .rsplit_once(':')
            .filter(|(_, p)| p.parse::<u16>().is_ok())
            .map_or(host, |(h, _)| h);
        if host_only.eq_ignore_ascii_case("localhost") || host_only.parse::<std::net::Ipv4Addr>().is_ok() {
            return Url::parse(&format!("http://{text}")).ok().map(|u| u.to_string());
        }
        if looks_like_host(host_only) {
            return Url::parse(&format!("https://{text}")).ok().map(|u| u.to_string());
        }
    }
    Some(search_url(settings, text))
}

fn looks_like_host(host: &str) -> bool {
    let Some((_, tld)) = host.rsplit_once('.') else {
        return false;
    };
    tld.len() >= 2
        && tld.chars().all(|c| c.is_alphabetic())
        && host
            .split('.')
            .all(|label| !label.is_empty() && label.chars().all(|c| c.is_alphanumeric() || c == '-'))
}

fn local_path(text: &str) -> Option<String> {
    let windows_drive = text.len() > 2 && text.as_bytes()[1] == b':' && matches!(text.as_bytes()[2], b'\\' | b'/');
    if !(text.starts_with('/') || text.starts_with("~/") || windows_drive) {
        return None;
    }
    let expanded = match text.strip_prefix("~/") {
        Some(rest) => dirs::home_dir()?.join(rest),
        None => Path::new(text).to_path_buf(),
    };
    let path = expanded.canonicalize().ok()?;
    Url::from_file_path(path).ok().map(|u| u.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn go(input: &str) -> String {
        interpret(input, &Settings::default()).unwrap()
    }

    #[test]
    fn keeps_full_urls() {
        assert_eq!(go("https://example.com/a?b=1"), "https://example.com/a?b=1");
        assert_eq!(go("about:blank"), "about:blank");
        assert_eq!(go("mailto:me@example.com"), "mailto:me@example.com");
    }

    #[test]
    fn adds_a_scheme_to_host_names() {
        assert_eq!(go("example.com"), "https://example.com/");
        assert_eq!(go("en.wikipedia.org/wiki/Rust"), "https://en.wikipedia.org/wiki/Rust");
        assert_eq!(go("localhost:8080/app"), "http://localhost:8080/app");
        assert_eq!(go("192.168.1.1"), "http://192.168.1.1/");
    }

    #[test]
    fn searches_everything_else() {
        assert_eq!(go("rust browser"), "https://duckduckgo.com/?q=rust%20browser");
        assert_eq!(go("hello"), "https://duckduckgo.com/?q=hello");
        assert_eq!(go("?example.com"), "https://duckduckgo.com/?q=example%2Ecom");
        assert_eq!(go("version 1.2"), "https://duckduckgo.com/?q=version%201%2E2");
        assert!(interpret("   ", &Settings::default()).is_none());
    }

    #[test]
    fn opens_browser_pages() {
        assert_eq!(go("browser://history"), protocol::page_url("history"));
    }

    #[test]
    fn opens_local_files() {
        let dir = std::env::temp_dir().canonicalize().unwrap();
        let url = go(dir.to_str().unwrap());
        assert!(url.starts_with("file://"), "{url}");
    }
}
