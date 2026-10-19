//! User preferences, stored as `settings.json` in the profile folder.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Search engines offered in settings. `custom` uses [`Settings::custom_search_url`].
pub const SEARCH_ENGINES: &[(&str, &str, &str)] = &[
    ("duckduckgo", "DuckDuckGo", "https://duckduckgo.com/?q=%s"),
    ("google", "Google", "https://www.google.com/search?q=%s"),
    ("bing", "Bing", "https://www.bing.com/search?q=%s"),
    ("startpage", "Startpage", "https://www.startpage.com/do/search?q=%s"),
    ("ecosia", "Ecosia", "https://www.ecosia.org/search?q=%s"),
];

/// User preferences. Unknown or missing fields fall back to their defaults, so older and
/// newer settings files both load.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub homepage: String,
    pub restore_session: bool,
    pub search_engine: String,
    pub custom_search_url: String,
    pub show_bookmarks_bar: bool,
    pub default_zoom: f64,
    pub download_dir: String,
    pub suggest_history: bool,
    pub suggest_bookmarks: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            homepage: "browser://newtab".into(),
            restore_session: false,
            search_engine: "duckduckgo".into(),
            custom_search_url: String::new(),
            show_bookmarks_bar: true,
            default_zoom: 1.0,
            download_dir: String::new(),
            suggest_history: true,
            suggest_bookmarks: true,
        }
    }
}

impl Settings {
    /// Loads settings, falling back to defaults if the file is missing or unreadable.
    pub fn load(path: &Path) -> Settings {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<Settings>(&s).ok())
            .map(Settings::sanitized)
            .unwrap_or_default()
    }

    /// Writes settings atomically, so a crash mid-write never leaves a truncated file.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(self).expect("settings serialize");
        crate::paths::write_atomic(path, json.as_bytes())
    }

    /// Clamps or resets values that would break the browser.
    pub fn sanitized(mut self) -> Settings {
        let defaults = Settings::default();
        if !self.default_zoom.is_finite() {
            self.default_zoom = 1.0;
        }
        self.default_zoom = self.default_zoom.clamp(0.3, 5.0);
        let known = SEARCH_ENGINES.iter().any(|(id, _, _)| *id == self.search_engine);
        let custom_ok = self.search_engine == "custom" && valid_search_template(&self.custom_search_url);
        if !known && !custom_ok {
            self.search_engine = defaults.search_engine;
        }
        if self.homepage.trim().is_empty() {
            self.homepage = defaults.homepage;
        }
        self
    }

    /// The URL template for the selected search engine, with `%s` where the query goes.
    pub fn search_template(&self) -> &str {
        if self.search_engine == "custom" && valid_search_template(&self.custom_search_url) {
            return &self.custom_search_url;
        }
        SEARCH_ENGINES
            .iter()
            .find(|(id, _, _)| *id == self.search_engine)
            .map(|(_, _, t)| *t)
            .unwrap_or(SEARCH_ENGINES[0].2)
    }

    /// Display name of the selected search engine.
    pub fn search_engine_name(&self) -> String {
        if self.search_engine == "custom" {
            return url::Url::parse(&self.custom_search_url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string))
                .unwrap_or_else(|| "custom search".into());
        }
        SEARCH_ENGINES
            .iter()
            .find(|(id, _, _)| *id == self.search_engine)
            .map(|(_, name, _)| name.to_string())
            .unwrap_or_else(|| "DuckDuckGo".into())
    }
}

/// A custom search URL must be http(s) and contain `%s`.
pub fn valid_search_template(t: &str) -> bool {
    t.contains("%s")
        && url::Url::parse(&t.replace("%s", "q"))
            .map(|u| matches!(u.scheme(), "http" | "https"))
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_use_defaults() {
        let s: Settings = serde_json::from_str(r#"{"restoreSession": true, "futureOption": 1}"#).unwrap();
        assert!(s.restore_session);
        assert_eq!(s.search_engine, "duckduckgo");
        assert!(s.show_bookmarks_bar);
    }

    #[test]
    fn sanitizes_bad_values() {
        let s = Settings {
            default_zoom: 40.0,
            search_engine: "custom".into(),
            custom_search_url: "javascript:alert(%s)".into(),
            homepage: " ".into(),
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(s.default_zoom, 5.0);
        assert_eq!(s.search_engine, "duckduckgo");
        assert_eq!(s.homepage, "browser://newtab");
    }

    #[test]
    fn picks_search_templates() {
        let mut s = Settings {
            search_engine: "google".into(),
            ..Settings::default()
        };
        assert!(s.search_template().starts_with("https://www.google.com/"));
        s.search_engine = "custom".into();
        s.custom_search_url = "https://search.example/?q=%s".into();
        assert_eq!(s.search_template(), "https://search.example/?q=%s");
        assert_eq!(s.search_engine_name(), "search.example");
    }

    #[test]
    fn round_trips_through_a_file() {
        let dir = std::env::temp_dir().join(format!("wvb-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let s = Settings {
            restore_session: true,
            default_zoom: 1.25,
            ..Settings::default()
        };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
        std::fs::remove_dir_all(dir).ok();
    }
}
