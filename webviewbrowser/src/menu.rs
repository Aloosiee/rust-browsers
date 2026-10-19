//! The application menu opened from the toolbar's ☰ button, as a native popup menu.
//!
//! Menu item ids are action names (see [`crate::actions`]), so choosing an item runs the same
//! code as the matching keyboard shortcut.

use muda::{ContextMenu, Menu, MenuItem, PredefinedMenuItem};
use tao::window::Window;

pub struct AppMenu {
    menu: Menu,
}

const PRIMARY: &str = if cfg!(target_os = "macos") { "Cmd" } else { "Ctrl" };

fn item(id: &str, label: &str, keys: &str) -> MenuItem {
    // Windows right-aligns text after a tab as the shortcut column. GTK shows the tab as
    // spaces, so on Linux the shortcut is set on the item's accelerator label instead.
    let label = if keys.is_empty() || cfg!(target_os = "linux") {
        label.to_string()
    } else {
        format!("{label}\t{}", keys.replace("Ctrl", PRIMARY))
    };
    MenuItem::with_id(id, label, true, None)
}

/// Converts shortcut text like `Ctrl+Shift+P` or `Ctrl++` to GTK's `<Control><Shift>p`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn gtk_accelerator(keys: &str) -> String {
    let (mods, key) = match keys.strip_suffix("++") {
        Some(mods) => (mods, "plus"),
        None => keys.rsplit_once('+').unwrap_or(("", keys)),
    };
    let key = match key {
        "-" => "minus".to_string(),
        k if k.len() == 1 => k.to_ascii_lowercase(),
        k => k.to_string(),
    };
    let mods: String = mods
        .split('+')
        .filter(|m| !m.is_empty())
        .map(|m| format!("<{}>", if m == "Ctrl" { "Control" } else { m }))
        .collect();
    format!("{mods}{key}")
}

/// Shortcut text for each item, by menu item id.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const SHORTCUTS: &[(&str, &str)] = &[
    ("newTab", "Ctrl+T"),
    ("newWindow", "Ctrl+N"),
    ("newPrivateWindow", "Ctrl+Shift+P"),
    ("showBookmarks", "Ctrl+Shift+O"),
    ("showHistory", "Ctrl+H"),
    ("showDownloads", "Ctrl+J"),
    ("find", "Ctrl+F"),
    ("print", "Ctrl+P"),
    ("zoomIn", "Ctrl++"),
    ("zoomOut", "Ctrl+-"),
    ("zoomReset", "Ctrl+0"),
    ("fullscreen", "F11"),
    ("devTools", "F12"),
    ("quit", "Ctrl+Q"),
];

/// Puts each item's shortcut in the right-hand column of GTK's accelerator label. The
/// accelerators are for display only; the shortcuts themselves are handled in the pages.
#[cfg(target_os = "linux")]
fn show_shortcuts_gtk(menu: &Menu) {
    use gtk::prelude::*;
    let gtk_menu = menu.gtk_context_menu();
    let items: Vec<gtk::MenuItem> = gtk_menu
        .children()
        .into_iter()
        .filter_map(|w| w.downcast::<gtk::MenuItem>().ok())
        .filter(|w| !w.is::<gtk::SeparatorMenuItem>())
        .collect();
    let ids = menu
        .items()
        .into_iter()
        .filter_map(|i| i.as_menuitem().map(|m| m.id().0.clone()));
    for (gtk_item, id) in items.iter().zip(ids) {
        let Some((_, keys)) = SHORTCUTS.iter().find(|(i, _)| *i == id) else {
            continue;
        };
        let Some(label) = gtk_item.child().and_then(|c| c.downcast::<gtk::AccelLabel>().ok()) else {
            continue;
        };
        let (key, mods) = gtk::accelerator_parse(&gtk_accelerator(keys));
        if key != 0 {
            label.set_accel(key, mods);
        }
    }
}

impl AppMenu {
    pub fn new() -> AppMenu {
        let menu = Menu::new();
        let sep = || PredefinedMenuItem::separator();
        let items: Vec<Box<dyn muda::IsMenuItem>> = vec![
            Box::new(item("newTab", "New tab", "Ctrl+T")),
            Box::new(item("newWindow", "New window", "Ctrl+N")),
            Box::new(item("newPrivateWindow", "New private window", "Ctrl+Shift+P")),
            Box::new(sep()),
            Box::new(item("showBookmarks", "Bookmarks", "Ctrl+Shift+O")),
            Box::new(item("showHistory", "History", "Ctrl+H")),
            Box::new(item("showDownloads", "Downloads", "Ctrl+J")),
            Box::new(sep()),
            Box::new(item("find", "Find in page…", "Ctrl+F")),
            Box::new(item("print", "Print…", "Ctrl+P")),
            Box::new(item("zoomIn", "Zoom in", "Ctrl++")),
            Box::new(item("zoomOut", "Zoom out", "Ctrl+-")),
            Box::new(item("zoomReset", "Reset zoom", "Ctrl+0")),
            Box::new(item("fullscreen", "Full screen", "F11")),
            Box::new(sep()),
            Box::new(item("devTools", "Web developer tools", "F12")),
            Box::new(item("showSettings", "Settings", "")),
            Box::new(sep()),
            Box::new(item(
                "quit",
                "Quit",
                if cfg!(windows) { "Ctrl+Shift+Q" } else { "Ctrl+Q" },
            )),
        ];
        for i in &items {
            let _ = menu.append(i.as_ref());
        }
        #[cfg(target_os = "linux")]
        show_shortcuts_gtk(&menu);
        AppMenu { menu }
    }

    /// Opens the menu at `(x, y)` in the window's logical coordinates.
    pub fn show(&self, window: &Window, x: f64, y: f64) {
        let position = Some(muda::dpi::Position::Logical(muda::dpi::LogicalPosition::new(x, y)));
        #[cfg(target_os = "linux")]
        {
            use gtk::prelude::Cast;
            use tao::platform::unix::WindowExtUnix;
            let gtk_window: &gtk::Window = window.gtk_window().upcast_ref();
            self.menu.show_context_menu_for_gtk_window(gtk_window, position);
        }
        #[cfg(windows)]
        unsafe {
            use tao::platform::windows::WindowExtWindows;
            self.menu.show_context_menu_for_hwnd(window.hwnd(), position);
        }
        #[cfg(target_os = "macos")]
        unsafe {
            use tao::platform::macos::WindowExtMacOS;
            self.menu.show_context_menu_for_nsview(window.ns_view() as _, position);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_shortcuts_for_gtk() {
        assert_eq!(gtk_accelerator("Ctrl+Shift+P"), "<Control><Shift>p");
        assert_eq!(gtk_accelerator("Ctrl++"), "<Control>plus");
        assert_eq!(gtk_accelerator("Ctrl+-"), "<Control>minus");
        assert_eq!(gtk_accelerator("F11"), "F11");
    }
}
