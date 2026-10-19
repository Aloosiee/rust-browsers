//! Saving and restoring open windows and tabs between runs.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub windows: Vec<WindowState>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    pub tabs: Vec<TabState>,
    pub active: usize,
    pub width: f64,
    pub height: f64,
    pub maximized: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TabState {
    pub url: String,
    pub title: String,
}

impl Session {
    pub fn load(path: &Path) -> Option<Session> {
        let text = std::fs::read_to_string(path).ok()?;
        let session: Session = serde_json::from_str(&text).ok()?;
        let windows: Vec<WindowState> = session.windows.into_iter().filter(|w| !w.tabs.is_empty()).collect();
        (!windows.is_empty()).then_some(Session { windows })
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        crate::paths::write_atomic(path, serde_json::to_string(self).expect("session serialize").as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_drops_empty_windows() {
        let dir = std::env::temp_dir().join(format!("wvb-session-{}", std::process::id()));
        let path = dir.join("session.json");
        let session = Session {
            windows: vec![
                WindowState {
                    tabs: vec![TabState {
                        url: "https://example.com/".into(),
                        title: "Example".into(),
                    }],
                    active: 0,
                    width: 1200.0,
                    height: 800.0,
                    maximized: false,
                },
                WindowState::default(),
            ],
        };
        session.save(&path).unwrap();
        let loaded = Session::load(&path).unwrap();
        assert_eq!(loaded.windows.len(), 1);
        assert_eq!(loaded.windows[0], session.windows[0]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn missing_or_empty_sessions_are_none() {
        assert!(Session::load(Path::new("/nonexistent/session.json")).is_none());
    }
}
