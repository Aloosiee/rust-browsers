//! A loaded document: its DOM, stylesheets, script runtime and latest layout.
//!
//! [`Page`] owns the pipeline from bytes to display list and implements the browser's default
//! actions for clicks, key presses and form submission after scripts have had a chance to
//! cancel them.

use crate::css::{Color, Stylesheet, parse_stylesheet};
use crate::dom::{Node, NodeType};
use crate::font::FontStore;
use crate::js::{EventInit, JsRuntime, Requests};
use crate::jsdom::Dom;
use crate::layout::{Rect, build_layout_tree};
use crate::net::{self, escape};
use crate::paint::{Cmd, LinkArea, display_list};
use crate::style::{Origin, style_tree};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use url::Url;

pub const UA_CSS: &str = include_str!("ua.css");
/// Set to false by `--no-js`.
pub static JS_ENABLED: AtomicBool = AtomicBool::new(true);

/// A loaded document and everything derived from it.
pub struct Page {
    pub url: Url,
    pub base: Url,
    pub title: String,
    dom: Rc<RefCell<Dom>>,
    sheets: Vec<(Stylesheet, Origin)>,
    js: Option<JsRuntime>,
    laid_version: u64,
    pub cmds: Vec<Cmd>,
    pub links: Vec<LinkArea>,
    hits: Vec<(Rect, usize)>,
    pub anchors: HashMap<String, f32>,
    pub height: f32,
    pub bg: Color,
    pub focused: Option<usize>,
    viewport: (f32, f32),
}

/// What the browser should do after an input event.
pub enum Action {
    None,
    Navigate(Url),
}

enum SheetSource {
    Inline(String, Option<String>),
    Linked(Url, Option<String>),
}

fn collect_sheets(node: &Node, base: &Url, out: &mut Vec<SheetSource>) {
    if let NodeType::Element(e) = &node.node_type {
        let media = e
            .attr("media")
            .map(|m| m.trim().to_ascii_lowercase())
            .filter(|m| !m.is_empty() && m != "all" && m != "screen");
        match e.tag.as_str() {
            "style" => {
                out.push(SheetSource::Inline(node.text_content(), media));
                return;
            }
            "link" => {
                let rel = e.attr("rel").unwrap_or("").to_ascii_lowercase();
                if rel.split_whitespace().any(|r| r == "stylesheet") && !rel.contains("alternate") {
                    if let Some(u) = e.attr("href").and_then(|h| base.join(h.trim()).ok()) {
                        out.push(SheetSource::Linked(u, media));
                    }
                }
                return;
            }
            "svg" | "template" | "noscript" => return,
            _ => {}
        }
    }
    for c in &node.children {
        collect_sheets(c, base, out);
    }
}

fn with_media(mut sheet: Stylesheet, media: Option<String>) -> Stylesheet {
    if let Some(m) = media {
        for rule in &mut sheet.rules {
            rule.media = Some(match &rule.media {
                Some(inner) => format!("{} and {}", m, inner),
                None => m.clone(),
            });
        }
    }
    sheet
}

pub fn error_html(title: &str, detail: &str) -> String {
    format!(
        "<html><head><title>{t}</title><style>body{{font-family:sans-serif;margin:48px;color:#333}}h1{{color:#a33a12;font-size:26px}}p.d{{background:#f3f1ec;padding:10px 14px;font-family:monospace}}</style></head>\
         <body><h1>{t}</h1><p class=\"d\">{d}</p><p><a href=\"about:home\">Go to the start page</a></p></body></html>",
        t = escape(title),
        d = escape(detail)
    )
}

impl Page {
    /// Fetches and builds a page. Network failures produce an error page instead of an error.
    pub fn load(url: Url) -> Page {
        match net::fetch(&url) {
            Err(e) => Page::from_html(url, &error_html("Couldn't load this page", &e)),
            Ok(resp) => {
                let ct = resp.content_type.to_ascii_lowercase();
                let text = net::decode(&resp.body, &resp.content_type);
                if ct.starts_with("text/html")
                    || ct.contains("xhtml")
                    || ct.contains("xml") && text.trim_start().starts_with('<')
                {
                    let mut page = Page::from_html(resp.url, &text);
                    if resp.status >= 400 && page.title.is_empty() {
                        page.title = format!("HTTP {}", resp.status);
                    }
                    page
                } else if ct.starts_with("text/") || ct.contains("json") || ct.contains("javascript") {
                    let html = format!(
                        "<html><body><pre style=\"white-space: pre-wrap\">{}</pre></body></html>",
                        escape(&text)
                    );
                    Page::static_html(resp.url, &html)
                } else {
                    let detail = format!("{} ({} bytes)", resp.content_type, resp.body.len());
                    Page::static_html(resp.url, &error_html("This file type can't be displayed", &detail))
                }
            }
        }
    }

    pub fn static_html(url: Url, html: &str) -> Page {
        Page::build(url, html, false)
    }

    pub fn from_html(url: Url, html: &str) -> Page {
        Page::build(url, html, JS_ENABLED.load(Ordering::Relaxed))
    }

    fn build(url: Url, html: &str, scripts: bool) -> Page {
        let tree = crate::html::parse(html);
        let base = tree
            .find_first("base")
            .and_then(|b| b.as_element()?.attr("href"))
            .and_then(|h| url.join(h).ok())
            .unwrap_or_else(|| url.clone());
        let mut sources = Vec::new();
        collect_sheets(&tree, &base, &mut sources);
        let mut sheets = vec![(parse_stylesheet(UA_CSS), Origin::UserAgent)];
        let mut fetched = 0;
        for src in sources {
            let (css, media) = match src {
                SheetSource::Inline(css, m) => (css, m),
                SheetSource::Linked(u, m) => {
                    if fetched >= 12 || m.as_deref().map_or(false, |m| m.contains("print")) {
                        continue;
                    }
                    fetched += 1;
                    match net::fetch(&u) {
                        Ok(r) if r.status < 400 => (net::decode(&r.body, &r.content_type), m),
                        _ => continue,
                    }
                }
            };
            sheets.push((with_media(parse_stylesheet(&css), media), Origin::Author));
        }
        let dom = Rc::new(RefCell::new(Dom::from_tree(&tree)));
        let mut page = Page {
            url: url.clone(),
            base: base.clone(),
            title: String::new(),
            dom: dom.clone(),
            sheets,
            js: None,
            laid_version: 0,
            cmds: Vec::new(),
            links: Vec::new(),
            hits: Vec::new(),
            anchors: HashMap::new(),
            height: 0.0,
            bg: Color::WHITE,
            focused: None,
            viewport: (1024.0, 768.0),
        };
        if scripts {
            let mut js = JsRuntime::new(dom, base);
            js.run_page_scripts();
            page.js = Some(js);
        }
        page.refresh_title();
        page
    }

    fn refresh_title(&mut self) {
        let d = self.dom.borrow();
        self.title = d
            .first_tag("title")
            .map(|t| d.text_content(t).split_whitespace().collect::<Vec<_>>().join(" "))
            .unwrap_or_default();
    }

    /// True when the DOM changed since the last layout.
    pub fn needs_layout(&self) -> bool {
        self.dom.borrow().version != self.laid_version
    }

    /// Restyles and lays out the current DOM at the given viewport size.
    pub fn layout(&mut self, width: f32, viewport_h: f32, fonts: &FontStore) {
        let tree: Node = {
            let d = self.dom.borrow();
            self.laid_version = d.version;
            d.to_tree()
        };
        self.viewport = (width, viewport_h);
        let Some(styled) = style_tree(&tree, &self.sheets, width, viewport_h) else {
            return;
        };
        let body_bg = styled
            .children
            .iter()
            .find(|c| c.node.tag() == Some("body"))
            .and_then(|b| b.style.background);
        self.bg = styled.style.background.or(body_bg).unwrap_or(Color::WHITE);
        let mut root = build_layout_tree(&styled, self.focused);
        drop(styled);
        let mut anchors = Vec::new();
        let (mt, _) = root.vertical_margins(width);
        root.layout(0.0, width, mt, fonts, &mut anchors);
        let mb = root.dims.margin_box();
        self.height = mb.y + mb.h;
        self.anchors.clear();
        for (name, y) in anchors {
            self.anchors.entry(name).or_insert(y);
        }
        let painted = display_list(&root, fonts);
        self.cmds = painted.cmds;
        self.links = painted.links;
        self.hits = painted.hits;
        if let Some(js) = &self.js {
            js.set_layout_info(self.viewport, 0.0, painted.rects);
        }
        self.refresh_title();
    }

    pub fn link_at(&self, x: f32, y: f32) -> Option<&str> {
        self.links
            .iter()
            .rev()
            .find(|l| l.rect.contains(x, y))
            .map(|l| &*l.href)
    }

    /// The topmost element painted at a point in page coordinates.
    pub fn element_at(&self, x: f32, y: f32) -> Option<usize> {
        self.hits
            .iter()
            .rev()
            .find(|(r, _)| r.contains(x, y))
            .map(|(_, id)| *id)
    }

    pub fn resolve(&self, href: &str) -> Option<Url> {
        self.base.join(href).ok()
    }

    pub fn tick(&mut self) -> bool {
        self.js.as_mut().map_or(false, |js| js.tick())
    }

    pub fn take_requests(&mut self) -> Requests {
        self.js.as_ref().map(|js| js.take_requests()).unwrap_or_default()
    }

    pub fn set_scroll(&self, y: f32) {
        if let Some(js) = &self.js {
            js.set_scroll(y);
        }
    }

    pub fn set_focus(&mut self, id: Option<usize>) {
        if self.focused != id {
            self.focused = id;
            self.laid_version = 0;
            if let Some(js) = &self.js {
                js.set_focus(id);
            }
        }
    }

    pub fn run_js_url(&mut self, code: &str) {
        if let Some(js) = &mut self.js {
            let code = url::form_urlencoded::parse(format!("x={}", code.replace('+', "%2B")).as_bytes())
                .next()
                .map(|(_, v)| v.into_owned())
                .unwrap_or_default();
            js.exec(&code, "javascript: URL");
        }
    }

    fn dispatch(&mut self, target: Option<usize>, ty: &str, init: EventInit) -> bool {
        match &mut self.js {
            Some(js) => js.dispatch(target, ty, init),
            None => false,
        }
    }

    fn element_tag(&self, id: usize) -> Option<String> {
        self.dom.borrow().element(id).map(|e| e.tag.clone())
    }

    fn attr(&self, id: usize, name: &str) -> Option<String> {
        self.dom
            .borrow()
            .element(id)
            .and_then(|e| e.attr(name).map(str::to_string))
    }

    fn set_attr(&self, id: usize, name: &str, value: Option<String>) {
        let mut d = self.dom.borrow_mut();
        if let Some(e) = d.element_mut(id) {
            match value {
                Some(v) => {
                    e.attrs.insert(name.to_string(), v);
                }
                None => {
                    e.attrs.remove(name);
                }
            }
        }
    }

    fn is_text_field(&self, id: usize) -> bool {
        match self.element_tag(id).as_deref() {
            Some("textarea") => true,
            Some("input") => !matches!(
                self.attr(id, "type").unwrap_or_default().to_ascii_lowercase().as_str(),
                "checkbox" | "radio" | "submit" | "button" | "reset" | "image" | "hidden" | "file" | "color" | "range"
            ),
            _ => false,
        }
    }

    /// Dispatches mouse events for a click and, unless a script cancels them, performs the default action of the clicked element.
    pub fn click(&mut self, x: f32, y: f32, client: (f32, f32), mods: (bool, bool, bool)) -> Action {
        let Some(target) = self.element_at(x, y) else {
            self.set_focus(None);
            return Action::None;
        };
        let init = |kind| EventInit {
            kind,
            bubbles: true,
            client: Some(client),
            key: None,
            ctrl: mods.0,
            shift: mods.1,
            alt: mods.2,
        };
        self.dispatch(Some(target), "mousedown", init("mouse"));
        self.dispatch(Some(target), "mouseup", init("mouse"));
        let prevented = self.dispatch(Some(target), "click", init("mouse"));
        if prevented {
            return Action::None;
        }
        let chain: Vec<usize> = {
            let d = self.dom.borrow();
            std::iter::once(target).chain(d.ancestors(target)).collect()
        };
        for id in chain {
            let Some(tag) = self.element_tag(id) else { continue };
            let kind = self.attr(id, "type").unwrap_or_default().to_ascii_lowercase();
            match tag.as_str() {
                "input" if kind == "checkbox" => {
                    let on = self.attr(id, "checked").is_none();
                    self.set_attr(id, "checked", on.then(String::new));
                    self.dispatch(Some(id), "input", EventInit::simple(true));
                    self.dispatch(Some(id), "change", EventInit::simple(true));
                    return Action::None;
                }
                "input" if kind == "radio" => {
                    let name = self.attr(id, "name");
                    let group: Vec<usize> = self.dom.borrow().find(0, |e| {
                        e.tag == "input"
                            && e.attr("type") == Some("radio")
                            && e.attr("name").map(str::to_string) == name
                    });
                    for g in group {
                        self.set_attr(g, "checked", None);
                    }
                    self.set_attr(id, "checked", Some(String::new()));
                    self.dispatch(Some(id), "change", EventInit::simple(true));
                    return Action::None;
                }
                "input" | "button"
                    if matches!(kind.as_str(), "submit" | "image") || (tag == "button" && kind.is_empty()) =>
                {
                    self.set_focus(None);
                    let form = self.dom.borrow().closest(id, |e| e.tag == "form");
                    if let Some(form) = form {
                        return self.submit(form, Some(id));
                    }
                    return Action::None;
                }
                "select" => {
                    let opts = self.dom.borrow().find(id, |e| e.tag == "option");
                    if !opts.is_empty() {
                        let cur = opts
                            .iter()
                            .position(|&o| self.attr(o, "selected").is_some())
                            .unwrap_or(0);
                        for &o in &opts {
                            self.set_attr(o, "selected", None);
                        }
                        self.set_attr(opts[(cur + 1) % opts.len()], "selected", Some(String::new()));
                        self.dispatch(Some(id), "change", EventInit::simple(true));
                    }
                    return Action::None;
                }
                "a" => {
                    if let Some(href) = self.attr(id, "href") {
                        self.set_focus(None);
                        let trimmed = href.trim();
                        if let Some(code) = trimmed.strip_prefix("javascript:") {
                            self.run_js_url(code);
                            return Action::None;
                        }
                        if let Some(u) = self.resolve(trimmed) {
                            return Action::Navigate(u);
                        }
                    }
                }
                _ if self.is_text_field(id) => {
                    self.set_focus(Some(id));
                    self.dispatch(Some(id), "focus", EventInit::simple(false));
                    return Action::None;
                }
                _ => {}
            }
        }
        if let Some(prev) = self.focused {
            self.dispatch(Some(prev), "blur", EventInit::simple(false));
        }
        self.set_focus(None);
        Action::None
    }

    /// Dispatches key events and edits the focused text field. Returns whether the page consumed the key.
    pub fn key(&mut self, key: &str, mods: (bool, bool, bool)) -> (bool, Action) {
        let target = self.focused.or_else(|| self.dom.borrow().first_tag("body"));
        let init = |kind| EventInit {
            kind,
            bubbles: true,
            client: None,
            key: Some(key.to_string()),
            ctrl: mods.0,
            shift: mods.1,
            alt: mods.2,
        };
        let prevented = self.dispatch(target, "keydown", init("key"));
        if key.chars().count() == 1 {
            self.dispatch(target, "keypress", init("key"));
        }
        self.dispatch(target, "keyup", init("key"));
        if prevented {
            return (true, Action::None);
        }
        let Some(f) = self.focused.filter(|&f| self.is_text_field(f)) else {
            return (false, Action::None);
        };
        let is_textarea = self.element_tag(f).as_deref() == Some("textarea");
        let mut value = {
            let d = self.dom.borrow();
            let e = d.element(f).unwrap();
            e.attr("value")
                .map(str::to_string)
                .unwrap_or_else(|| if is_textarea { d.text_content(f) } else { String::new() })
        };
        match key {
            "Backspace" => {
                value.pop();
            }
            "Enter" if is_textarea => value.push('\n'),
            "Enter" => {
                self.dispatch(Some(f), "change", EventInit::simple(true));
                let form = self.dom.borrow().closest(f, |e| e.tag == "form");
                return (true, form.map(|form| self.submit(form, None)).unwrap_or(Action::None));
            }
            "Escape" => {
                self.set_focus(None);
                return (true, Action::None);
            }
            k if k.chars().count() == 1 && !mods.0 && !mods.2 => value.push_str(k),
            _ => return (true, Action::None),
        }
        self.set_attr(f, "value", Some(value));
        self.dispatch(
            Some(f),
            "input",
            EventInit {
                kind: "input",
                ..EventInit::simple(true)
            },
        );
        (true, Action::None)
    }

    /// Fires `submit` and, if not cancelled, builds the GET URL for the form.
    fn submit(&mut self, form: usize, submitter: Option<usize>) -> Action {
        let prevented = self.dispatch(Some(form), "submit", EventInit::simple(true));
        if prevented {
            return Action::None;
        }
        let method = self.attr(form, "method").unwrap_or_default().to_ascii_lowercase();
        if method == "post" {
            eprintln!("[form] POST submission is not supported");
            return Action::None;
        }
        let action = self.attr(form, "action").unwrap_or_default();
        let Some(mut url) = self.resolve(if action.trim().is_empty() {
            self.url.as_str()
        } else {
            action.trim()
        }) else {
            return Action::None;
        };
        let fields = self.dom.borrow().find(form, |e| {
            matches!(e.tag.as_str(), "input" | "select" | "textarea" | "button")
        });
        let mut pairs: Vec<(String, String)> = Vec::new();
        for id in fields {
            let Some(name) = self.attr(id, "name").filter(|n| !n.is_empty()) else {
                continue;
            };
            if self.attr(id, "disabled").is_some() {
                continue;
            }
            let tag = self.element_tag(id).unwrap_or_default();
            let kind = self.attr(id, "type").unwrap_or_default().to_ascii_lowercase();
            let value = match tag.as_str() {
                "select" => {
                    let d = self.dom.borrow();
                    let opts = d.find(id, |e| e.tag == "option");
                    let chosen = opts
                        .iter()
                        .copied()
                        .find(|&o| d.element(o).map_or(false, |e| e.attrs.contains_key("selected")))
                        .or(opts.first().copied());
                    chosen.map(|o| {
                        d.element(o)
                            .and_then(|e| e.attr("value").map(str::to_string))
                            .unwrap_or_else(|| d.text_content(o).trim().to_string())
                    })
                }
                "textarea" => Some(
                    self.attr(id, "value")
                        .unwrap_or_else(|| self.dom.borrow().text_content(id)),
                ),
                "button" | "input"
                    if matches!(kind.as_str(), "submit" | "image" | "button" | "reset") || tag == "button" =>
                {
                    if Some(id) == submitter {
                        Some(self.attr(id, "value").unwrap_or_default())
                    } else {
                        None
                    }
                }
                "input" if kind == "checkbox" || kind == "radio" => self
                    .attr(id, "checked")
                    .map(|_| self.attr(id, "value").unwrap_or_else(|| "on".into())),
                _ => Some(self.attr(id, "value").unwrap_or_default()),
            };
            if let Some(v) = value {
                pairs.push((name, v));
            }
        }
        url.set_fragment(None);
        url.query_pairs_mut().clear().extend_pairs(pairs);
        Action::Navigate(url)
    }
}
