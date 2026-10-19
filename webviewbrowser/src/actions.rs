//! Browser commands that can be triggered from keyboard shortcuts, the toolbar or the app menu.
//!
//! Shortcuts are detected in JavaScript (in the toolbar and in every page) and arrive here as
//! action names, so the key map lives in `ui/shortcuts.js` and this module only names actions.

use std::str::FromStr;

/// A browser command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    NewTab,
    CloseTab,
    ReopenClosedTab,
    NextTab,
    PreviousTab,
    SelectTab(usize),
    SelectLastTab,
    FocusAddressBar,
    Reload,
    Stop,
    Back,
    Forward,
    Home,
    BookmarkPage,
    ToggleBookmarksBar,
    ShowBookmarks,
    ShowHistory,
    ShowDownloads,
    ShowSettings,
    Find,
    FindNext,
    FindPrevious,
    Print,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    Fullscreen,
    DevTools,
    NewWindow,
    NewPrivateWindow,
    CloseWindow,
    Quit,
}

impl FromStr for Action {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if let Some(n) = s.strip_prefix("selectTab") {
            return match n.parse::<usize>() {
                Ok(n) if (1..=8).contains(&n) => Ok(Action::SelectTab(n - 1)),
                _ => Err(format!("unknown action {s}")),
            };
        }
        Ok(match s {
            "newTab" => Action::NewTab,
            "closeTab" => Action::CloseTab,
            "reopenClosedTab" => Action::ReopenClosedTab,
            "nextTab" => Action::NextTab,
            "previousTab" => Action::PreviousTab,
            "selectLastTab" => Action::SelectLastTab,
            "focusAddressBar" => Action::FocusAddressBar,
            "reload" => Action::Reload,
            "stop" => Action::Stop,
            "back" => Action::Back,
            "forward" => Action::Forward,
            "home" => Action::Home,
            "bookmarkPage" => Action::BookmarkPage,
            "toggleBookmarksBar" => Action::ToggleBookmarksBar,
            "showBookmarks" => Action::ShowBookmarks,
            "showHistory" => Action::ShowHistory,
            "showDownloads" => Action::ShowDownloads,
            "showSettings" => Action::ShowSettings,
            "find" => Action::Find,
            "findNext" => Action::FindNext,
            "findPrevious" => Action::FindPrevious,
            "print" => Action::Print,
            "zoomIn" => Action::ZoomIn,
            "zoomOut" => Action::ZoomOut,
            "zoomReset" => Action::ZoomReset,
            "fullscreen" => Action::Fullscreen,
            "devTools" => Action::DevTools,
            "newWindow" => Action::NewWindow,
            "newPrivateWindow" => Action::NewPrivateWindow,
            "closeWindow" => Action::CloseWindow,
            "quit" => Action::Quit,
            _ => return Err(format!("unknown action {s}")),
        })
    }
}

impl Action {
    /// Whether a web page may trigger this action through the shortcut bridge.
    ///
    /// Pages can post the same messages the shortcut listener posts, so anything a hostile page
    /// should not be able to do on its own is reserved for the toolbar and the app menu.
    pub fn allowed_from_web_page(self) -> bool {
        !matches!(self, Action::Quit | Action::CloseWindow)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_action_names() {
        assert_eq!("newTab".parse(), Ok(Action::NewTab));
        assert_eq!("selectTab1".parse(), Ok(Action::SelectTab(0)));
        assert_eq!("selectTab8".parse(), Ok(Action::SelectTab(7)));
        assert!("selectTab9".parse::<Action>().is_err());
        assert!("format".parse::<Action>().is_err());
    }

    #[test]
    fn pages_cannot_quit_or_close_the_window() {
        assert!(!Action::Quit.allowed_from_web_page());
        assert!(!Action::CloseWindow.allowed_from_web_page());
        assert!(Action::CloseTab.allowed_from_web_page());
    }
}
