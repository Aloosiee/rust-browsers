//! History, bookmarks, downloads and per-site zoom, stored in SQLite.
//!
//! Like Firefox's `places.sqlite`, history keeps one row per URL (`places`) with a visit count
//! for ranking address bar suggestions, plus one row per visit for the history page.

use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub type Result<T> = rusqlite::Result<T>;

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Visit {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub at: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Site {
    pub url: String,
    pub title: String,
    pub visits: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub url: String,
    pub title: String,
    pub bookmarked: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Bookmark {
    pub id: i64,
    pub url: String,
    pub title: String,
    pub toolbar: bool,
    pub added: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Download {
    pub id: i64,
    pub url: String,
    pub path: String,
    pub state: String,
    pub started: i64,
    pub finished: Option<i64>,
}

pub struct Store {
    conn: Connection,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS places (
    url TEXT PRIMARY KEY,
    title TEXT NOT NULL DEFAULT '',
    visit_count INTEGER NOT NULL DEFAULT 0,
    last_visit INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS visits (
    id INTEGER PRIMARY KEY,
    url TEXT NOT NULL,
    at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS visits_at ON visits(at);
CREATE INDEX IF NOT EXISTS visits_url ON visits(url);
CREATE TABLE IF NOT EXISTS bookmarks (
    id INTEGER PRIMARY KEY,
    url TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    toolbar INTEGER NOT NULL DEFAULT 1,
    position INTEGER NOT NULL DEFAULT 0,
    added INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS bookmarks_url ON bookmarks(url);
CREATE TABLE IF NOT EXISTS downloads (
    id INTEGER PRIMARY KEY,
    url TEXT NOT NULL,
    path TEXT NOT NULL,
    state TEXT NOT NULL,
    started INTEGER NOT NULL,
    finished INTEGER
);
CREATE TABLE IF NOT EXISTS site_zoom (
    host TEXT PRIMARY KEY,
    zoom REAL NOT NULL
);
";

/// Escapes `%`, `_` and `\` for a `LIKE ... ESCAPE '\'` pattern.
fn like_pattern(text: &str) -> String {
    let mut out = String::from("%");
    for c in text.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('%');
    out
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Store::init(Connection::open(path)?)
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Store> {
        Store::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Store> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Store { conn })
    }

    pub fn record_visit(&self, url: &str, title: &str, at: i64) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO places (url, title, visit_count, last_visit) VALUES (?1, ?2, 1, ?3)
             ON CONFLICT(url) DO UPDATE SET visit_count = visit_count + 1, last_visit = ?3,
             title = CASE WHEN ?2 = '' THEN title ELSE ?2 END",
            params![url, title, at],
        )?;
        tx.execute("INSERT INTO visits (url, at) VALUES (?1, ?2)", params![url, at])?;
        tx.commit()
    }

    pub fn update_title(&self, url: &str, title: &str) -> Result<()> {
        self.conn
            .execute("UPDATE places SET title = ?2 WHERE url = ?1", params![url, title])?;
        self.conn.execute(
            "UPDATE bookmarks SET title = ?2 WHERE url = ?1 AND title = ''",
            params![url, title],
        )?;
        Ok(())
    }

    /// Visits newest first, optionally filtered by text in the URL or title and paged with
    /// `before` (a visit time in ms).
    pub fn history(&self, text: &str, before: Option<i64>, limit: usize) -> Result<Vec<Visit>> {
        let mut stmt = self.conn.prepare(
            "SELECT v.id, v.url, COALESCE(p.title, ''), v.at FROM visits v LEFT JOIN places p ON p.url = v.url
             WHERE v.at < ?1 AND (?2 = '%%' OR v.url LIKE ?2 ESCAPE '\\' OR p.title LIKE ?2 ESCAPE '\\')
             ORDER BY v.at DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            params![before.unwrap_or(i64::MAX), like_pattern(text), limit as i64],
            |r| {
                Ok(Visit {
                    id: r.get(0)?,
                    url: r.get(1)?,
                    title: r.get(2)?,
                    at: r.get(3)?,
                })
            },
        )?;
        rows.collect()
    }

    pub fn delete_visit(&self, id: i64) -> Result<()> {
        let url: Option<String> = self
            .conn
            .query_row("SELECT url FROM visits WHERE id = ?1", [id], |r| r.get(0))
            .optional()?;
        self.conn.execute("DELETE FROM visits WHERE id = ?1", [id])?;
        if let Some(url) = url {
            self.prune_place(&url)?;
        }
        Ok(())
    }

    fn prune_place(&self, url: &str) -> Result<()> {
        let remaining: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM visits WHERE url = ?1", [url], |r| r.get(0))?;
        if remaining == 0 {
            self.conn.execute("DELETE FROM places WHERE url = ?1", [url])?;
        } else {
            self.conn.execute(
                "UPDATE places SET visit_count = ?2, last_visit = (SELECT MAX(at) FROM visits WHERE url = ?1) WHERE url = ?1",
                params![url, remaining],
            )?;
        }
        Ok(())
    }

    /// Deletes history newer than `since` (ms), or everything when `since` is `None`.
    pub fn clear_history(&self, since: Option<i64>) -> Result<()> {
        match since {
            None => self.conn.execute_batch("DELETE FROM visits; DELETE FROM places;"),
            Some(t) => {
                let urls: Vec<String> = {
                    let mut stmt = self.conn.prepare("SELECT DISTINCT url FROM visits WHERE at >= ?1")?;
                    stmt.query_map([t], |r| r.get(0))?.collect::<Result<_>>()?
                };
                self.conn.execute("DELETE FROM visits WHERE at >= ?1", [t])?;
                for url in urls {
                    self.prune_place(&url)?;
                }
                Ok(())
            }
        }
    }

    pub fn top_sites(&self, limit: usize) -> Result<Vec<Site>> {
        let mut stmt = self.conn.prepare(
            "SELECT url, title, visit_count FROM places ORDER BY visit_count DESC, last_visit DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map([limit as i64], |r| {
            Ok(Site {
                url: r.get(0)?,
                title: r.get(1)?,
                visits: r.get(2)?,
            })
        })?;
        rows.collect()
    }

    /// Address bar suggestions: bookmarks first, then history ranked by visit count weighted by
    /// how recently the page was visited.
    pub fn suggest(&self, text: &str, use_history: bool, use_bookmarks: bool, limit: usize) -> Result<Vec<Suggestion>> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(Vec::new());
        }
        let pattern = like_pattern(text);
        let mut out: Vec<Suggestion> = Vec::new();
        if use_bookmarks {
            let mut stmt = self.conn.prepare(
                "SELECT url, title FROM bookmarks WHERE url LIKE ?1 ESCAPE '\\' OR title LIKE ?1 ESCAPE '\\'
                 ORDER BY position, added LIMIT ?2",
            )?;
            for row in stmt.query_map(params![pattern, limit as i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get(1)?))
            })? {
                let (url, title) = row?;
                out.push(Suggestion {
                    url,
                    title,
                    bookmarked: true,
                });
            }
        }
        if use_history {
            let mut stmt = self.conn.prepare(
                "SELECT url, title FROM places WHERE url LIKE ?1 ESCAPE '\\' OR title LIKE ?1 ESCAPE '\\'
                 ORDER BY visit_count * 1.0 / (1.0 + (?2 - last_visit) / 604800000.0) DESC LIMIT ?3",
            )?;
            for row in stmt.query_map(params![pattern, now_ms(), limit as i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get(1)?))
            })? {
                let (url, title) = row?;
                if !out.iter().any(|s| s.url == url) {
                    out.push(Suggestion {
                        url,
                        title,
                        bookmarked: false,
                    });
                }
            }
        }
        out.truncate(limit);
        Ok(out)
    }

    pub fn bookmarks(&self) -> Result<Vec<Bookmark>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, url, title, toolbar, added FROM bookmarks ORDER BY position, added")?;
        let rows = stmt.query_map([], |r| {
            Ok(Bookmark {
                id: r.get(0)?,
                url: r.get(1)?,
                title: r.get(2)?,
                toolbar: r.get(3)?,
                added: r.get(4)?,
            })
        })?;
        rows.collect()
    }

    pub fn toolbar_bookmarks(&self) -> Result<Vec<Bookmark>> {
        Ok(self.bookmarks()?.into_iter().filter(|b| b.toolbar).collect())
    }

    pub fn bookmark_for(&self, url: &str) -> Result<Option<i64>> {
        self.conn
            .query_row("SELECT id FROM bookmarks WHERE url = ?1 LIMIT 1", [url], |r| r.get(0))
            .optional()
    }

    pub fn add_bookmark(&self, url: &str, title: &str, toolbar: bool) -> Result<i64> {
        let position: i64 = self
            .conn
            .query_row("SELECT COALESCE(MAX(position), 0) + 1 FROM bookmarks", [], |r| r.get(0))?;
        self.conn.execute(
            "INSERT INTO bookmarks (url, title, toolbar, position, added) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![url, title, toolbar, position, now_ms()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_bookmark(&self, id: i64, url: &str, title: &str, toolbar: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE bookmarks SET url = ?2, title = ?3, toolbar = ?4 WHERE id = ?1",
            params![id, url, title, toolbar],
        )?;
        Ok(())
    }

    /// Moves a bookmark to `index` in the overall order.
    pub fn move_bookmark(&self, id: i64, index: usize) -> Result<()> {
        let mut ids: Vec<i64> = self
            .bookmarks()?
            .into_iter()
            .map(|b| b.id)
            .filter(|b| *b != id)
            .collect();
        ids.insert(index.min(ids.len()), id);
        let tx = self.conn.unchecked_transaction()?;
        for (pos, bid) in ids.iter().enumerate() {
            tx.execute(
                "UPDATE bookmarks SET position = ?2 WHERE id = ?1",
                params![bid, pos as i64 + 1],
            )?;
        }
        tx.commit()
    }

    pub fn delete_bookmark(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM bookmarks WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn add_download(&self, url: &str, path: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO downloads (url, path, state, started) VALUES (?1, ?2, 'downloading', ?3)",
            params![url, path, now_ms()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Marks the newest unfinished download of `url` as finished and returns its id.
    pub fn finish_download(&self, url: &str, path: Option<&str>, ok: bool) -> Result<Option<i64>> {
        let id: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM downloads WHERE url = ?1 AND state = 'downloading' ORDER BY started DESC LIMIT 1",
                [url],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = id {
            let state = if ok { "completed" } else { "failed" };
            self.conn.execute(
                "UPDATE downloads SET state = ?2, finished = ?3 WHERE id = ?1",
                params![id, state, now_ms()],
            )?;
            if let Some(p) = path.filter(|p| !p.is_empty()) {
                self.conn
                    .execute("UPDATE downloads SET path = ?2 WHERE id = ?1", params![id, p])?;
            }
        }
        Ok(id)
    }

    pub fn downloads(&self) -> Result<Vec<Download>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, url, path, state, started, finished FROM downloads ORDER BY started DESC LIMIT 500")?;
        let rows = stmt.query_map([], |r| {
            Ok(Download {
                id: r.get(0)?,
                url: r.get(1)?,
                path: r.get(2)?,
                state: r.get(3)?,
                started: r.get(4)?,
                finished: r.get(5)?,
            })
        })?;
        rows.collect()
    }

    pub fn download(&self, id: i64) -> Result<Option<Download>> {
        Ok(self.downloads()?.into_iter().find(|d| d.id == id))
    }

    pub fn remove_download(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM downloads WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Removes finished downloads from the list. Files on disk are left alone.
    pub fn clear_downloads(&self) -> Result<()> {
        self.conn
            .execute("DELETE FROM downloads WHERE state != 'downloading'", [])?;
        Ok(())
    }

    /// Marks downloads left over from a previous run as failed.
    pub fn fail_interrupted_downloads(&self) -> Result<()> {
        self.conn
            .execute("UPDATE downloads SET state = 'failed' WHERE state = 'downloading'", [])?;
        Ok(())
    }

    pub fn site_zoom(&self, host: &str) -> Result<Option<f64>> {
        self.conn
            .query_row("SELECT zoom FROM site_zoom WHERE host = ?1", [host], |r| r.get(0))
            .optional()
    }

    /// Remembers a zoom level for a host; a level equal to `default` forgets it.
    pub fn set_site_zoom(&self, host: &str, zoom: f64, default: f64) -> Result<()> {
        if (zoom - default).abs() < 1e-6 {
            self.conn.execute("DELETE FROM site_zoom WHERE host = ?1", [host])?;
        } else {
            self.conn.execute(
                "INSERT INTO site_zoom (host, zoom) VALUES (?1, ?2) ON CONFLICT(host) DO UPDATE SET zoom = ?2",
                params![host, zoom],
            )?;
        }
        Ok(())
    }

    pub fn clear_site_zoom(&self) -> Result<()> {
        self.conn.execute("DELETE FROM site_zoom", [])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_ranks_history() {
        let s = Store::in_memory().unwrap();
        s.record_visit("https://a.example/", "Alpha", 1000).unwrap();
        s.record_visit("https://b.example/", "Beta", 2000).unwrap();
        s.record_visit("https://a.example/", "", 3000).unwrap();
        let visits = s.history("", None, 10).unwrap();
        assert_eq!(visits.len(), 3);
        assert_eq!(visits[0].at, 3000);
        assert_eq!(visits[0].title, "Alpha");
        assert_eq!(s.top_sites(1).unwrap()[0].url, "https://a.example/");
        assert_eq!(s.history("beta", None, 10).unwrap().len(), 1);
        assert_eq!(s.history("", Some(3000), 10).unwrap().len(), 2);
    }

    #[test]
    fn like_patterns_are_escaped() {
        let s = Store::in_memory().unwrap();
        s.record_visit("https://a.example/100%", "", 1).unwrap();
        s.record_visit("https://b.example/", "", 2).unwrap();
        assert_eq!(s.history("100%", None, 10).unwrap().len(), 1);
        assert_eq!(s.history("_", None, 10).unwrap().len(), 0);
    }

    #[test]
    fn deleting_visits_prunes_places() {
        let s = Store::in_memory().unwrap();
        s.record_visit("https://a.example/", "A", 1).unwrap();
        let id = s.history("", None, 1).unwrap()[0].id;
        s.delete_visit(id).unwrap();
        assert!(s.top_sites(5).unwrap().is_empty());
        s.record_visit("https://a.example/", "A", 10).unwrap();
        s.record_visit("https://b.example/", "B", 20).unwrap();
        s.clear_history(Some(15)).unwrap();
        assert_eq!(s.top_sites(5).unwrap().len(), 1);
        s.clear_history(None).unwrap();
        assert!(s.history("", None, 5).unwrap().is_empty());
    }

    #[test]
    fn suggests_bookmarks_before_history() {
        let s = Store::in_memory().unwrap();
        s.record_visit("https://doc.rust-lang.org/", "Rust Documentation", now_ms())
            .unwrap();
        s.record_visit("https://docs.rs/", "Docs.rs", now_ms()).unwrap();
        s.add_bookmark("https://docs.rs/", "Docs", true).unwrap();
        let out = s.suggest("doc", true, true, 5).unwrap();
        assert_eq!(
            out[0],
            Suggestion {
                url: "https://docs.rs/".into(),
                title: "Docs".into(),
                bookmarked: true
            }
        );
        assert_eq!(out.len(), 2);
        assert!(s.suggest("doc", false, false, 5).unwrap().is_empty());
    }

    #[test]
    fn manages_bookmarks() {
        let s = Store::in_memory().unwrap();
        let a = s.add_bookmark("https://a.example/", "A", true).unwrap();
        let b = s.add_bookmark("https://b.example/", "B", false).unwrap();
        assert_eq!(s.bookmark_for("https://b.example/").unwrap(), Some(b));
        assert_eq!(s.toolbar_bookmarks().unwrap().len(), 1);
        s.move_bookmark(b, 0).unwrap();
        assert_eq!(s.bookmarks().unwrap()[0].id, b);
        s.update_bookmark(a, "https://a2.example/", "A2", false).unwrap();
        assert!(s.toolbar_bookmarks().unwrap().is_empty());
        s.delete_bookmark(a).unwrap();
        assert_eq!(s.bookmarks().unwrap().len(), 1);
    }

    #[test]
    fn tracks_downloads() {
        let s = Store::in_memory().unwrap();
        let id = s.add_download("https://x.example/f.zip", "/tmp/f.zip").unwrap();
        assert_eq!(
            s.finish_download("https://x.example/f.zip", Some("/tmp/f (1).zip"), true)
                .unwrap(),
            Some(id)
        );
        let d = s.download(id).unwrap().unwrap();
        assert_eq!(d.state, "completed");
        assert_eq!(d.path, "/tmp/f (1).zip");
        s.add_download("https://x.example/g.zip", "/tmp/g.zip").unwrap();
        s.fail_interrupted_downloads().unwrap();
        s.clear_downloads().unwrap();
        assert!(s.downloads().unwrap().is_empty());
    }

    #[test]
    fn remembers_site_zoom() {
        let s = Store::in_memory().unwrap();
        s.set_site_zoom("example.com", 1.5, 1.0).unwrap();
        assert_eq!(s.site_zoom("example.com").unwrap(), Some(1.5));
        s.set_site_zoom("example.com", 1.0, 1.0).unwrap();
        assert_eq!(s.site_zoom("example.com").unwrap(), None);
    }
}
