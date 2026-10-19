//! Messages that arrive over the webview IPC bridge.
//!
//! There are two channels. The toolbar (`chrome`) is the browser's own UI and is trusted.
//! Every page, including web pages, can post [`PageMessage`]s; requests that read or change
//! browser data are only honoured when the sending page is one of the `browser:` pages.

use serde::Deserialize;
use serde_json::Value;

/// A message from the toolbar.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ChromeMessage {
    Ready,
    Layout {
        height: f64,
    },
    Overlay {
        height: f64,
    },
    Navigate {
        input: String,
        #[serde(default)]
        new_tab: bool,
    },
    OpenUrl {
        url: String,
        #[serde(default)]
        new_tab: bool,
    },
    Action {
        name: String,
    },
    SelectTab {
        id: u64,
    },
    CloseTab {
        id: u64,
    },
    MoveTab {
        id: u64,
        index: usize,
    },
    Suggest {
        text: String,
    },
    Find {
        query: String,
        backwards: bool,
        #[serde(default)]
        again: bool,
    },
    FindClose,
    Menu {
        x: f64,
        y: f64,
    },
    Infobar {
        id: u64,
        accept: bool,
    },
    FocusPage,
}

/// A message from a page in a tab.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum PageMessage {
    Shortcut {
        action: String,
    },
    Location,
    Favicon {
        url: String,
    },
    FindResult {
        current: u32,
        total: u32,
    },
    Fullscreen {
        on: bool,
    },
    Close,
    Request {
        id: u64,
        method: String,
        #[serde(default)]
        params: Value,
    },
}

/// A page message plus the address of the page that sent it.
#[derive(Debug, Deserialize)]
pub struct PageEnvelope {
    #[serde(default)]
    pub href: Option<String>,
    #[serde(flatten)]
    pub message: PageMessage,
}

impl PageMessage {
    /// Whether a page with the given origin may send this message.
    pub fn allowed_from(&self, sender_is_internal: bool) -> bool {
        match self {
            PageMessage::Request { .. } => sender_is_internal,
            _ => true,
        }
    }
}

/// Favicon URLs are shown in the toolbar, so only web and inline image URLs are accepted.
pub fn safe_favicon(url: &str) -> Option<String> {
    let ok = url.len() <= 100_000
        && (url.starts_with("https://") || url.starts_with("http://") || url.starts_with("data:image/"));
    ok.then(|| url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_toolbar_messages() {
        let m: ChromeMessage = serde_json::from_str(r#"{"kind":"navigate","input":"rust","newTab":true}"#).unwrap();
        assert_eq!(
            m,
            ChromeMessage::Navigate {
                input: "rust".into(),
                new_tab: true
            }
        );
        let m: ChromeMessage = serde_json::from_str(r#"{"kind":"moveTab","id":3,"index":0}"#).unwrap();
        assert_eq!(m, ChromeMessage::MoveTab { id: 3, index: 0 });
        assert!(serde_json::from_str::<ChromeMessage>(r#"{"kind":"rm -rf"}"#).is_err());
    }

    #[test]
    fn requests_need_an_internal_sender() {
        let m: PageMessage = serde_json::from_str(r#"{"kind":"request","id":1,"method":"history.clear"}"#).unwrap();
        assert!(!m.allowed_from(false));
        assert!(m.allowed_from(true));
        let m: PageMessage = serde_json::from_str(r#"{"kind":"findResult","current":1,"total":4}"#).unwrap();
        assert!(m.allowed_from(false));
    }

    #[test]
    fn envelopes_carry_the_sender_address() {
        let e: PageEnvelope = serde_json::from_str(r#"{"kind":"close","href":"https://a.example/x"}"#).unwrap();
        assert_eq!(e.message, PageMessage::Close);
        assert_eq!(e.href.as_deref(), Some("https://a.example/x"));
        let e: PageEnvelope = serde_json::from_str(r#"{"kind":"shortcut","action":"newTab"}"#).unwrap();
        assert_eq!(
            e.message,
            PageMessage::Shortcut {
                action: "newTab".into()
            }
        );
        assert_eq!(e.href, None);
    }

    #[test]
    fn filters_favicon_urls() {
        assert!(safe_favicon("https://example.com/favicon.ico").is_some());
        assert!(safe_favicon("data:image/png;base64,AAAA").is_some());
        assert!(safe_favicon("javascript:alert(1)").is_none());
        assert!(safe_favicon("file:///etc/passwd").is_none());
    }
}
