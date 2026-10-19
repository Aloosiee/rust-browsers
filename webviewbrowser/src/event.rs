//! Events sent from webview callbacks to the event loop.
//!
//! Webview callbacks run inside the engine's own event dispatch, so they never touch browser
//! state directly; they post one of these and the event loop handles it in order.

use crate::tab::TabId;
use std::path::PathBuf;
use tao::window::WindowId;

#[derive(Debug)]
pub enum UserEvent {
    /// A message from a window's toolbar.
    Chrome {
        window: WindowId,
        body: String,
    },
    /// A message from a page. `sender` is the URL of the page that sent it.
    Page {
        window: WindowId,
        tab: TabId,
        sender: String,
        body: String,
    },
    Title {
        window: WindowId,
        tab: TabId,
        title: String,
    },
    Load {
        window: WindowId,
        tab: TabId,
        started: bool,
        url: String,
    },
    /// A link to a scheme the browser does not handle, such as `mailto:`.
    External {
        window: WindowId,
        url: String,
    },
    /// A popup that could not be created as a linked tab; opened as a plain tab instead.
    OpenTab {
        window: WindowId,
        url: String,
    },
    DownloadStarted {
        private: bool,
        url: String,
        path: PathBuf,
    },
    DownloadFinished {
        private: bool,
        url: String,
        path: Option<PathBuf>,
        ok: bool,
    },
    Menu(String),
}
