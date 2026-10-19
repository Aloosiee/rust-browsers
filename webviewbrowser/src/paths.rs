//! Locations of the profile folder and its files, plus atomic file writes.

use std::io::Write;
use std::path::{Path, PathBuf};

/// Files and folders that make up a browser profile.
pub struct Profile {
    pub root: PathBuf,
}

impl Profile {
    /// The default profile under the platform's local data folder, or `override_dir` if given.
    pub fn locate(override_dir: Option<PathBuf>) -> Profile {
        let root = override_dir.unwrap_or_else(|| {
            dirs::data_local_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("rust-browsers")
                .join("webviewbrowser")
        });
        Profile { root }
    }

    pub fn database(&self) -> PathBuf {
        self.root.join("browser.sqlite")
    }

    pub fn settings(&self) -> PathBuf {
        self.root.join("settings.json")
    }

    pub fn session(&self) -> PathBuf {
        self.root.join("session.json")
    }

    /// Cookies, cache and site storage for normal windows.
    pub fn web_data(&self) -> PathBuf {
        self.root.join("webdata")
    }

    /// Throwaway site data shared by private windows; deleted on exit and on the next start.
    pub fn private_web_data(&self) -> PathBuf {
        self.root.join("private-webdata")
    }

    /// Unpacked browser extensions (WebView2 on Windows only).
    pub fn extensions(&self) -> PathBuf {
        self.root.join("extensions")
    }
}

/// Writes a file by writing a sibling temp file and renaming it over the target.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

/// Where downloads go when the user hasn't picked a folder: the system Downloads folder, or a
/// `Downloads` folder in the home directory when the system doesn't define one.
pub fn default_download_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
        .unwrap_or_else(std::env::temp_dir)
}
