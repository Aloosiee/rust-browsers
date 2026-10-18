//! HTML tokenizer and tree builder.
//!
//! This is a forgiving subset of the WHATWG parsing algorithm: it handles raw-text elements,
//! character references, void elements, the common implied end tags and stray end tags, then
//! normalizes the result so every document has an `html` element with a `body`.

use crate::dom::{Node, NodeType};
use std::collections::HashMap;

/// Elements that never have children or an end tag.
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr",
    "keygen", "frame",
];
/// Elements whose content is read verbatim up to the matching end tag.
const RAW_TEXT: &[&str] = &["script", "style", "textarea", "title", "xmp", "iframe", "noembed"];
/// Start tags that implicitly close an open `<p>`.
const CLOSES_P: &[&str] = &[
    "address",
    "article",
    "aside",
    "blockquote",
    "center",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hgroup",
    "hr",
    "main",
    "menu",
    "nav",
    "ol",
    "p",
    "pre",
    "section",
    "summary",
    "table",
    "ul",
];
const INLINE_FORMATTING: &[&str] = &[
    "a", "b", "big", "code", "em", "font", "i", "s", "small", "span", "strike", "strong", "tt", "u", "abbr", "cite",
    "kbd", "mark", "q", "samp", "sub", "sup", "var", "label",
];
const HEAD_ONLY: &[&str] = &["head", "title", "meta", "link", "style", "base", "script", "noscript"];

enum Token {
    Start {
        name: String,
        attrs: HashMap<String, String>,
        self_closing: bool,
    },
    End(String),
    Text(String),
}

/// Character-level tokenizer. `raw_until` is set while inside a raw-text element.
struct Tokenizer {
    chars: Vec<char>,
    pos: usize,
    raw_until: Option<String>,
    pending: Vec<Token>,
}

impl Tokenizer {
    fn peek(&self, off: usize) -> Option<char> {
        self.chars.get(self.pos + off).copied()
    }

    fn starts_with_ci(&self, s: &str) -> bool {
        s.chars()
            .enumerate()
            .all(|(i, c)| self.peek(i).map(|p| p.to_ascii_lowercase() == c).unwrap_or(false))
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek(0) {
            if c.is_ascii_whitespace() {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn skip_past(&mut self, pat: &str) {
        while self.pos < self.chars.len() && !self.starts_with_ci(pat) {
            self.pos += 1;
        }
        self.pos = (self.pos + pat.chars().count()).min(self.chars.len());
    }

    fn next(&mut self) -> Option<Token> {
        if self.pos >= self.chars.len() {
            return None;
        }
        if let Some(tag) = self.raw_until.take() {
            let close = format!("</{}", tag);
            let start = self.pos;
            while self.pos < self.chars.len() && !self.starts_with_ci(&close) {
                self.pos += 1;
            }
            let text: String = self.chars[start..self.pos].iter().collect();
            let text = if tag == "textarea" || tag == "title" {
                decode_entities(&text)
            } else {
                text
            };
            if self.pos < self.chars.len() {
                self.skip_past(">");
            }
            if text.is_empty() {
                return Some(Token::End(tag));
            }
            self.pending.push(Token::End(tag));
            return Some(Token::Text(text));
        }
        if self.peek(0) == Some('<') {
            if self.starts_with_ci("<!--") {
                self.pos += 4;
                self.skip_past("-->");
                return self.next();
            }
            if matches!(self.peek(1), Some('!') | Some('?')) {
                self.skip_past(">");
                return self.next();
            }
            if self.peek(1) == Some('/') && self.peek(2).map_or(false, |c| c.is_ascii_alphabetic()) {
                self.pos += 2;
                let name = self.read_name();
                self.skip_past(">");
                return Some(Token::End(name));
            }
            if self.peek(1).map_or(false, |c| c.is_ascii_alphabetic()) {
                self.pos += 1;
                return Some(self.read_start_tag());
            }
        }
        let start = self.pos;
        self.pos += 1;
        while self.pos < self.chars.len() && self.chars[self.pos] != '<' {
            self.pos += 1;
        }
        let raw: String = self.chars[start..self.pos].iter().collect();
        Some(Token::Text(decode_entities(&raw)))
    }

    fn read_name(&mut self) -> String {
        let start = self.pos;
        while let Some(c) = self.peek(0) {
            if c.is_ascii_whitespace() || c == '>' || c == '/' {
                break;
            }
            self.pos += 1;
        }
        self.chars[start..self.pos]
            .iter()
            .collect::<String>()
            .to_ascii_lowercase()
    }

    fn read_start_tag(&mut self) -> Token {
        let name = self.read_name();
        let mut attrs = HashMap::new();
        let mut self_closing = false;
        loop {
            self.skip_ws();
            match self.peek(0) {
                None => break,
                Some('>') => {
                    self.pos += 1;
                    break;
                }
                Some('/') => {
                    self.pos += 1;
                    if self.peek(0) == Some('>') {
                        self_closing = true;
                    }
                }
                Some(_) => {
                    let start = self.pos;
                    while let Some(c) = self.peek(0) {
                        if c.is_ascii_whitespace() || c == '=' || c == '>' || c == '/' {
                            break;
                        }
                        self.pos += 1;
                    }
                    if self.pos == start {
                        self.pos += 1;
                        continue;
                    }
                    let key: String = self.chars[start..self.pos]
                        .iter()
                        .collect::<String>()
                        .to_ascii_lowercase();
                    self.skip_ws();
                    let mut value = String::new();
                    if self.peek(0) == Some('=') {
                        self.pos += 1;
                        self.skip_ws();
                        match self.peek(0) {
                            Some(q @ ('"' | '\'')) => {
                                self.pos += 1;
                                let vs = self.pos;
                                while self.peek(0).map_or(false, |c| c != q) {
                                    self.pos += 1;
                                }
                                value = self.chars[vs..self.pos].iter().collect();
                                self.pos = (self.pos + 1).min(self.chars.len());
                            }
                            _ => {
                                let vs = self.pos;
                                while let Some(c) = self.peek(0) {
                                    if c.is_ascii_whitespace() || c == '>' {
                                        break;
                                    }
                                    self.pos += 1;
                                }
                                value = self.chars[vs..self.pos].iter().collect();
                            }
                        }
                    }
                    attrs.entry(key).or_insert_with(|| decode_entities(&value));
                }
            }
        }
        if RAW_TEXT.contains(&name.as_str()) && !self_closing {
            self.raw_until = Some(name.clone());
        }
        Token::Start {
            name,
            attrs,
            self_closing,
        }
    }
}

impl Tokenizer {
    fn pending_pop(&mut self) -> Option<Token> {
        if self.pending.is_empty() {
            None
        } else {
            Some(self.pending.remove(0))
        }
    }
}

struct Lexer {
    tok: Tokenizer,
}

impl Iterator for Lexer {
    type Item = Token;
    fn next(&mut self) -> Option<Token> {
        if !self.tok.pending.is_empty() {
            return self.tok.pending_pop();
        }
        self.tok.next()
    }
}

/// Builds the tree with an explicit stack of open elements.
struct TreeBuilder {
    stack: Vec<Node>,
    skip_newline: bool,
}

impl TreeBuilder {
    fn top_tag(&self) -> Option<&str> {
        self.stack.last().and_then(|n| n.tag())
    }

    fn close_top(&mut self) {
        if self.stack.len() > 1 {
            let node = self.stack.pop().unwrap();
            self.stack.last_mut().unwrap().children.push(node);
        }
    }

    fn close_to(&mut self, idx: usize) {
        while self.stack.len() > idx.max(1) {
            self.close_top();
        }
    }

    fn find(&self, tag: &str, boundaries: &[&str]) -> Option<usize> {
        for i in (1..self.stack.len()).rev() {
            let t = self.stack[i].tag().unwrap_or("");
            if t == tag {
                return Some(i);
            }
            if boundaries.contains(&t) {
                return None;
            }
        }
        None
    }

    /// Closes a `<p>` if only inline formatting elements sit above it on the stack.
    fn close_open_p(&mut self) {
        for i in (1..self.stack.len()).rev() {
            let t = self.stack[i].tag().unwrap_or("");
            if t == "p" {
                self.close_to(i);
                return;
            }
            if !INLINE_FORMATTING.contains(&t) {
                return;
            }
        }
    }

    fn push_text(&mut self, mut text: String) {
        if std::mem::take(&mut self.skip_newline) {
            if let Some(rest) = text.strip_prefix("\r\n").or_else(|| text.strip_prefix('\n')) {
                text = rest.to_string();
            }
            if text.is_empty() {
                return;
            }
        }
        let parent = self.stack.last_mut().unwrap();
        if let Some(Node {
            node_type: NodeType::Text(prev),
            ..
        }) = parent.children.last_mut()
        {
            prev.push_str(&text);
        } else {
            parent.children.push(Node::text(text));
        }
    }

    /// Handles a start tag, applying the implied end tag rules first.
    fn start(&mut self, name: String, attrs: HashMap<String, String>, self_closing: bool) {
        let n = name.as_str();
        if CLOSES_P.contains(&n) {
            self.close_open_p();
        }
        match n {
            "li" => {
                if let Some(i) = self.find("li", &["ul", "ol", "menu"]) {
                    self.close_to(i);
                }
            }
            "dt" | "dd" => {
                if let Some(i) = self.find("dd", &["dl"]).or_else(|| self.find("dt", &["dl"])) {
                    self.close_to(i);
                }
            }
            "option" => {
                if self.top_tag() == Some("option") {
                    self.close_top();
                }
            }
            "tr" => {
                if let Some(i) = self.find("tr", &["table", "tbody", "thead", "tfoot"]) {
                    self.close_to(i);
                }
            }
            "td" | "th" => {
                if let Some(i) = self
                    .find("td", &["tr", "table"])
                    .or_else(|| self.find("th", &["tr", "table"]))
                {
                    self.close_to(i);
                }
            }
            "thead" | "tbody" | "tfoot" => {
                for t in ["thead", "tbody", "tfoot"] {
                    if let Some(i) = self.find(t, &["table"]) {
                        self.close_to(i);
                    }
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                if matches!(self.top_tag(), Some("h1" | "h2" | "h3" | "h4" | "h5" | "h6")) {
                    self.close_top();
                }
            }
            "a" => {
                if let Some(i) = self.find("a", &[]) {
                    self.close_to(i);
                }
            }
            "html" | "body" | "head" => {
                if let Some(i) = self.find(n, &[]) {
                    for (k, v) in attrs {
                        if let NodeType::Element(e) = &mut self.stack[i].node_type {
                            e.attrs.entry(k).or_insert(v);
                        }
                    }
                    return;
                }
            }
            _ => {}
        }
        self.skip_newline = matches!(n, "pre" | "listing" | "textarea") && !self_closing;
        let node = Node::element(n, attrs);
        if VOID.contains(&n) || self_closing {
            self.stack.last_mut().unwrap().children.push(node);
        } else {
            self.stack.push(node);
        }
    }

    fn end(&mut self, name: String) {
        if name == "br" {
            self.start(name, HashMap::new(), true);
            return;
        }
        if let Some(i) = self.find(&name, &[]) {
            self.close_to(i);
        }
    }
}

/// Parses an HTML document. Never fails: malformed markup is repaired, not rejected.
pub fn parse(source: &str) -> Node {
    let tok = Tokenizer {
        chars: source.chars().collect(),
        pos: 0,
        raw_until: None,
        pending: Vec::new(),
    };
    let mut builder = TreeBuilder {
        stack: vec![Node {
            node_type: NodeType::Document,
            children: Vec::new(),
            id: 0,
        }],
        skip_newline: false,
    };
    for token in (Lexer { tok }) {
        if !matches!(token, Token::Text(_)) {
            builder.skip_newline = false;
        }
        match token {
            Token::Start {
                name,
                attrs,
                self_closing,
            } => builder.start(name, attrs, self_closing),
            Token::End(name) => builder.end(name),
            Token::Text(t) => builder.push_text(t),
        }
    }
    builder.close_to(1);
    normalize(builder.stack.pop().unwrap())
}

/// Ensures the document has an `html` root containing a `body`.
fn normalize(mut doc: Node) -> Node {
    let html_idx = doc.children.iter().position(|c| c.tag() == Some("html"));
    let mut html = match html_idx {
        Some(i) => {
            let mut html = doc.children.remove(i);
            let rest: Vec<Node> = doc.children.drain(..).filter(|c| c.as_element().is_some()).collect();
            html.children.extend(rest);
            html
        }
        None => {
            let mut html = Node::element("html", HashMap::new());
            html.children = doc.children.drain(..).collect();
            html
        }
    };
    if !html.children.iter().any(|c| c.tag() == Some("body")) {
        let mut body = Node::element("body", HashMap::new());
        let mut kept = Vec::new();
        for child in html.children.drain(..) {
            match child.tag() {
                Some(t) if HEAD_ONLY.contains(&t) => kept.push(child),
                _ => body.children.push(child),
            }
        }
        kept.push(body);
        html.children = kept;
    }
    doc.children = vec![html];
    doc
}

/// Decodes named and numeric character references. Unknown references are left as written.
pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let bytes: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != '&' {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < bytes.len() && j - i < 32 && (bytes[j].is_ascii_alphanumeric() || bytes[j] == '#') {
            j += 1;
        }
        let name: String = bytes[i + 1..j].iter().collect();
        let has_semi = bytes.get(j) == Some(&';');
        let decoded = if let Some(num) = name.strip_prefix('#') {
            let code = if let Some(hex) = num.strip_prefix('x').or_else(|| num.strip_prefix('X')) {
                u32::from_str_radix(hex, 16).ok()
            } else {
                num.parse::<u32>().ok()
            };
            code.map(|c| match c {
                0x80..=0x9f => cp1252(c as u8),
                _ => char::from_u32(c).unwrap_or('\u{fffd}'),
            })
            .map(|c| c.to_string())
        } else if has_semi || matches!(name.as_str(), "amp" | "lt" | "gt" | "quot" | "nbsp" | "copy") {
            named_entity(&name).map(|s| s.to_string())
        } else {
            None
        };
        match decoded {
            Some(d) => {
                out.push_str(&d);
                i = if has_semi { j + 1 } else { j };
            }
            None => {
                out.push('&');
                i += 1;
            }
        }
    }
    out
}

fn named_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => "\u{a0}",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "hellip" => "…",
        "mdash" => "—",
        "ndash" => "–",
        "lsquo" => "‘",
        "rsquo" => "’",
        "sbquo" => "‚",
        "ldquo" => "“",
        "rdquo" => "”",
        "bdquo" => "„",
        "bull" => "•",
        "middot" => "·",
        "laquo" => "«",
        "raquo" => "»",
        "lsaquo" => "‹",
        "rsaquo" => "›",
        "times" => "×",
        "divide" => "÷",
        "deg" => "°",
        "plusmn" => "±",
        "euro" => "€",
        "pound" => "£",
        "yen" => "¥",
        "cent" => "¢",
        "sect" => "§",
        "para" => "¶",
        "shy" => "",
        "zwj" | "zwnj" => "",
        "ensp" | "emsp" | "thinsp" => " ",
        "larr" => "←",
        "rarr" => "→",
        "uarr" => "↑",
        "darr" => "↓",
        "harr" => "↔",
        "hearts" => "♥",
        "eacute" => "é",
        "egrave" => "è",
        "ecirc" => "ê",
        "euml" => "ë",
        "aacute" => "á",
        "agrave" => "à",
        "acirc" => "â",
        "auml" => "ä",
        "aring" => "å",
        "ccedil" => "ç",
        "iacute" => "í",
        "icirc" => "î",
        "iuml" => "ï",
        "oacute" => "ó",
        "ocirc" => "ô",
        "ouml" => "ö",
        "uacute" => "ú",
        "ugrave" => "ù",
        "ucirc" => "û",
        "uuml" => "ü",
        "ntilde" => "ñ",
        "szlig" => "ß",
        "Eacute" => "É",
        "Agrave" => "À",
        "Ccedil" => "Ç",
        "iexcl" => "¡",
        "iquest" => "¿",
        "frac12" => "½",
        "frac14" => "¼",
        "frac34" => "¾",
        "micro" => "µ",
        "alpha" => "α",
        "beta" => "β",
        "pi" => "π",
        "lambda" => "λ",
        "infin" => "∞",
        "ne" => "≠",
        "le" => "≤",
        "ge" => "≥",
        "minus" => "−",
        _ => return None,
    })
}

/// Maps a Windows-1252 byte to its character; used for legacy pages and `&#128;`-style references.
pub fn cp1252(b: u8) -> char {
    const TABLE: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘',
        '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    if (0x80..0xa0).contains(&b) {
        TABLE[(b - 0x80) as usize]
    } else {
        b as char
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(doc: &Node) -> &Node {
        doc.find_first("body").expect("body")
    }

    fn tags(node: &Node) -> Vec<String> {
        node.children
            .iter()
            .filter_map(|c| c.tag().map(str::to_string))
            .collect()
    }

    #[test]
    fn inserts_html_and_body() {
        let doc = parse("<p>hi</p>");
        assert_eq!(doc.children[0].tag(), Some("html"));
        assert_eq!(tags(body(&doc)), ["p"]);
    }

    #[test]
    fn block_start_tag_closes_open_paragraph() {
        let doc = parse("<p>one<div>two</div>");
        assert_eq!(tags(body(&doc)), ["p", "div"]);
    }

    #[test]
    fn list_items_close_each_other() {
        let doc = parse("<ul><li>a<li>b<li>c</ul>");
        let ul = body(&doc).find_first("ul").unwrap();
        assert_eq!(tags(ul), ["li", "li", "li"]);
    }

    #[test]
    fn script_content_is_raw_text() {
        let doc = parse("<script>if (a < b && c > d) {}</script>");
        let script = doc.find_first("script").unwrap();
        assert_eq!(script.text_content(), "if (a < b && c > d) {}");
    }

    #[test]
    fn decodes_character_references() {
        assert_eq!(decode_entities("&amp; &lt;b&gt; &#233; &#x41; &copy"), "& <b> é A ©");
        assert_eq!(decode_entities("AT&T &unknown;"), "AT&T &unknown;");
    }

    #[test]
    fn drops_newline_after_pre_start_tag() {
        let doc = parse("<pre>\nline</pre>");
        assert_eq!(doc.find_first("pre").unwrap().text_content(), "line");
    }

    #[test]
    fn stray_end_tags_are_ignored() {
        let doc = parse("<div>a</span>b</div>");
        assert_eq!(body(&doc).find_first("div").unwrap().text_content(), "ab");
    }
}
