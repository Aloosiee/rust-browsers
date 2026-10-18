//! Resource loading for `http`, `https`, `file` and `about:` URLs, plus text decoding.

use crate::html::cp1252;
use std::sync::OnceLock;
use std::time::Duration;
use ureq::tls::{PemItem, RootCerts, TlsConfig, parse_pem};
use ureq::{Agent, ResponseExt};
use url::Url;

pub const HOME_HTML: &str = include_str!("home.html");

/// A fetched resource. `url` is the final URL after redirects.
pub struct Response {
    pub url: Url,
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
}

fn agent() -> &'static Agent {
    static AGENT: OnceLock<Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        let mut tls = TlsConfig::builder();
        if let Some(pem) = std::env::var_os("SSL_CERT_FILE").and_then(|p| std::fs::read(p).ok()) {
            let certs: Vec<_> = parse_pem(&pem)
                .filter_map(|item| match item {
                    Ok(PemItem::Certificate(c)) => Some(c.to_owned()),
                    _ => None,
                })
                .collect();
            if !certs.is_empty() {
                tls = tls.root_certs(RootCerts::new_with_certs(&certs));
            }
        }
        Agent::config_builder()
            .tls_config(tls.build())
            .timeout_global(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .user_agent("Mozilla/5.0 (compatible; RustBrowser/0.1; toy engine)")
            .build()
            .into()
    })
}

/// Fetches a URL. HTTP error statuses are returned as responses, not errors.
pub fn fetch(url: &Url) -> Result<Response, String> {
    match url.scheme() {
        "about" => {
            let body = match url.path() {
                "home" | "" => HOME_HTML.as_bytes().to_vec(),
                "blank" => Vec::new(),
                other => return Err(format!("Unknown page about:{}", other)),
            };
            Ok(Response {
                url: url.clone(),
                status: 200,
                content_type: "text/html; charset=utf-8".into(),
                body,
            })
        }
        "file" => {
            let path = url.to_file_path().map_err(|_| "Invalid file path".to_string())?;
            if path.is_dir() {
                return Ok(Response {
                    url: url.clone(),
                    status: 200,
                    content_type: "text/html".into(),
                    body: dir_listing(&path).into_bytes(),
                });
            }
            let body = std::fs::read(&path).map_err(|e| format!("{}: {}", path.display(), e))?;
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            let content_type = match ext.as_str() {
                "html" | "htm" | "xhtml" => "text/html",
                "css" => "text/css",
                "png" | "jpg" | "jpeg" | "gif" | "webp" => "image/other",
                "pdf" => "application/pdf",
                _ if body.iter().take(512).any(|&b| b == 0) => "application/octet-stream",
                _ if body.trim_ascii_start().starts_with(b"<") => "text/html",
                _ => "text/plain",
            };
            Ok(Response {
                url: url.clone(),
                status: 200,
                content_type: content_type.into(),
                body,
            })
        }
        "http" | "https" => {
            let mut resp = agent()
                .get(url.as_str())
                .header("Accept", "text/html,application/xhtml+xml,text/plain;q=0.9,*/*;q=0.5")
                .header("Accept-Language", "en,fr;q=0.8")
                .call()
                .map_err(|e| e.to_string())?;
            let final_url = Url::parse(&resp.get_uri().to_string()).unwrap_or_else(|_| url.clone());
            let status = resp.status().as_u16();
            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("text/html")
                .to_string();
            let body = resp
                .body_mut()
                .with_config()
                .limit(15 * 1024 * 1024)
                .read_to_vec()
                .map_err(|e| e.to_string())?;
            Ok(Response {
                url: final_url,
                status,
                content_type,
                body,
            })
        }
        other => Err(format!("The \"{}:\" scheme is not supported", other)),
    }
}

fn dir_listing(path: &std::path::Path) -> String {
    let mut entries: Vec<(String, bool)> = std::fs::read_dir(path)
        .map(|rd| {
            rd.flatten()
                .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path().is_dir()))
                .collect()
        })
        .unwrap_or_default();
    entries.sort();
    let mut html = format!(
        "<title>{0}</title><h1>Index of {0}</h1><ul>",
        escape(&path.display().to_string())
    );
    html += "<li><a href=\"../\">../</a></li>";
    for (name, dir) in entries {
        let slash = if dir { "/" } else { "" };
        html += &format!("<li><a href=\"{0}{1}\">{0}{1}</a></li>", escape(&name), slash);
    }
    html + "</ul>"
}

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Decodes a body as UTF-8 or Windows-1252, using the `Content-Type` charset or a `<meta charset>` in the first 2 KB.
pub fn decode(body: &[u8], content_type: &str) -> String {
    let body = body.strip_prefix(b"\xef\xbb\xbf").unwrap_or(body);
    let mut charset = content_type.to_ascii_lowercase().split(';').find_map(|p| {
        p.trim()
            .strip_prefix("charset=")
            .map(|c| c.trim_matches('"').to_string())
    });
    if charset.is_none() {
        let head = String::from_utf8_lossy(&body[..body.len().min(2048)]).to_ascii_lowercase();
        if let Some(i) = head.find("charset=") {
            let rest = &head[i + 8..];
            let rest = rest.trim_start_matches(|c| c == '"' || c == '\'');
            let end = rest
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
                .unwrap_or(rest.len());
            charset = Some(rest[..end].to_string());
        }
    }
    match charset.as_deref() {
        Some("iso-8859-1" | "latin1" | "latin-1" | "windows-1252" | "cp1252" | "us-ascii" | "ascii") => {
            body.iter().map(|&b| cp1252(b)).collect()
        }
        _ => String::from_utf8_lossy(body).into_owned(),
    }
}
