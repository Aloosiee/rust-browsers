//! CSS parser.
//!
//! Parses stylesheets into rules, selectors and declarations. Supported selectors are type,
//! class, ID, attribute, `:root` and `:link` compounds joined by descendant or child
//! combinators; anything else is dropped so one unsupported selector never discards a rule.

/// An sRGB color with 8-bit alpha.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color { r, g, b, a: 255 }
    }
    pub const BLACK: Color = Color::rgb(0, 0, 0);
    pub const WHITE: Color = Color::rgb(255, 255, 255);
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Unit {
    Px,
    Em,
    Rem,
    Percent,
    Pt,
    Vw,
    Vh,
    Ex,
    Ch,
    Cm,
    Mm,
    In,
}

/// One component of a declaration value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Keyword(String),
    Length(f32, Unit),
    Number(f32),
    Color(Color),
    Str(String),
    Comma,
    Slash,
}

/// A `property: value` pair, with `!important` recorded separately.
#[derive(Clone, Debug)]
pub struct Declaration {
    pub name: String,
    pub value: Vec<Value>,
    pub important: bool,
}

#[derive(Clone, Debug)]
pub enum AttrOp {
    Exists,
    Eq(String),
    Word(String),
    Prefix(String),
    Suffix(String),
    Contains(String),
    Dash(String),
}

#[derive(Clone, Debug, Default)]
pub struct Compound {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub attrs: Vec<(String, AttrOp)>,
    pub root: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Combinator {
    Descendant,
    Child,
}

/// A complex selector. `combinators[i]` joins `compounds[i]` and `compounds[i + 1]`.
#[derive(Clone, Debug)]
pub struct Selector {
    pub compounds: Vec<Compound>,
    pub combinators: Vec<Combinator>,
    pub specificity: u32,
}

/// A style rule. `media` holds the query of any enclosing `@media` blocks.
#[derive(Clone, Debug)]
pub struct Rule {
    pub selectors: Vec<Selector>,
    pub declarations: Vec<Declaration>,
    pub media: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Stylesheet {
    pub rules: Vec<Rule>,
}

/// Parses a stylesheet. Malformed rules are skipped rather than reported.
pub fn parse_stylesheet(src: &str) -> Stylesheet {
    let clean = strip_comments(src);
    let chars: Vec<char> = clean.chars().collect();
    let mut rules = Vec::new();
    parse_rules(&chars, None, &mut rules);
    Stylesheet { rules }
}

fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut rest = src;
    while let Some(i) = rest.find("/*") {
        out.push_str(&rest[..i]);
        match rest[i + 2..].find("*/") {
            Some(j) => rest = &rest[i + 2 + j + 2..],
            None => return out,
        }
    }
    out.push_str(rest);
    out
}

fn find_block_end(chars: &[char], open: usize) -> usize {
    let mut depth = 0;
    let mut i = open;
    let mut quote: Option<char> = None;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            if c == '\\' {
                i += 1;
            } else if c == q {
                quote = None;
            }
        } else if c == '"' || c == '\'' {
            quote = Some(c);
        } else if c == '{' {
            depth += 1;
        } else if c == '}' {
            depth -= 1;
            if depth == 0 {
                return i;
            }
        }
        i += 1;
    }
    chars.len()
}

fn find_top_level(chars: &[char], start: usize, targets: &[char]) -> usize {
    let mut i = start;
    let mut quote: Option<char> = None;
    let mut paren = 0;
    while i < chars.len() {
        let c = chars[i];
        if let Some(q) = quote {
            if c == '\\' {
                i += 1;
            } else if c == q {
                quote = None;
            }
        } else if c == '"' || c == '\'' {
            quote = Some(c);
        } else if c == '(' || c == '[' {
            paren += 1;
        } else if c == ')' || c == ']' {
            paren -= 1;
        } else if paren <= 0 && targets.contains(&c) {
            return i;
        }
        i += 1;
    }
    chars.len()
}

/// Parses a rule list, recursing into `@media`, `@supports` and similar grouping rules.
fn parse_rules(chars: &[char], media: Option<String>, rules: &mut Vec<Rule>) {
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && (chars[i].is_whitespace() || chars[i] == ';' || chars[i] == '}') {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        if chars[i] == '@' {
            let stop = find_top_level(chars, i, &['{', ';']);
            let prelude: String = chars[i + 1..stop.min(chars.len())].iter().collect();
            let name = prelude
                .split(|c: char| c.is_whitespace() || c == '(')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if stop >= chars.len() || chars[stop] == ';' {
                i = stop + 1;
                continue;
            }
            let end = find_block_end(chars, stop);
            let inner = &chars[(stop + 1).min(chars.len())..end.min(chars.len())];
            match name.as_str() {
                "media" => {
                    let q = prelude[5..].trim().to_ascii_lowercase();
                    let combined = match &media {
                        Some(m) => format!("{} and {}", m, q),
                        None => q,
                    };
                    parse_rules(inner, Some(combined), rules);
                }
                "supports" | "layer" | "document" | "-moz-document" | "container" | "scope" => {
                    parse_rules(inner, media.clone(), rules);
                }
                _ => {}
            }
            i = end + 1;
            continue;
        }
        let open = find_top_level(chars, i, &['{']);
        if open >= chars.len() {
            break;
        }
        let selector_text: String = chars[i..open].iter().collect();
        let end = find_block_end(chars, open);
        let body: String = chars[open + 1..end.min(chars.len())].iter().collect();
        i = end + 1;
        let selectors: Vec<Selector> = split_top_level(&selector_text, ',')
            .iter()
            .filter_map(|s| parse_selector(s.trim()))
            .collect();
        if selectors.is_empty() {
            continue;
        }
        let declarations = parse_declarations(&body);
        if !declarations.is_empty() {
            rules.push(Rule {
                selectors,
                declarations,
                media: media.clone(),
            });
        }
    }
}

fn split_top_level(s: &str, sep: char) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut start = 0;
    loop {
        let idx = find_top_level(&chars, start, &[sep]);
        out.push(chars[start..idx.min(chars.len())].iter().collect());
        if idx >= chars.len() {
            break;
        }
        start = idx + 1;
    }
    out
}

/// Parses the inside of a declaration block or a `style` attribute.
pub fn parse_declarations(body: &str) -> Vec<Declaration> {
    let mut out = Vec::new();
    for part in split_top_level(body, ';') {
        let Some(colon) = part.find(':') else { continue };
        let name = part[..colon].trim().to_ascii_lowercase();
        if name.is_empty() || name.starts_with("--") || name.contains('{') {
            continue;
        }
        let mut raw = part[colon + 1..].trim().to_string();
        let mut important = false;
        if let Some(idx) = raw.to_ascii_lowercase().rfind("!important") {
            important = true;
            raw.truncate(idx);
        }
        let value = parse_value(raw.trim());
        if !value.is_empty() {
            out.push(Declaration { name, value, important });
        }
    }
    out
}

/// Tokenizes a declaration value into lengths, numbers, colors, strings and keywords.
pub fn parse_value(s: &str) -> Vec<Value> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == ',' {
            out.push(Value::Comma);
            i += 1;
        } else if c == '/' {
            out.push(Value::Slash);
            i += 1;
        } else if c == '"' || c == '\'' {
            let start = i + 1;
            i += 1;
            while i < chars.len() && chars[i] != c {
                i += 1;
            }
            out.push(Value::Str(chars[start..i.min(chars.len())].iter().collect()));
            i += 1;
        } else if c == '#' {
            let start = i + 1;
            i += 1;
            while i < chars.len() && chars[i].is_ascii_alphanumeric() {
                i += 1;
            }
            let hex: String = chars[start..i].iter().collect();
            match parse_hex(&hex) {
                Some(col) => out.push(Value::Color(col)),
                None => out.push(Value::Keyword(format!("#{}", hex))),
            }
        } else if c.is_ascii_digit()
            || (c == '.' && chars.get(i + 1).map_or(false, |d| d.is_ascii_digit()))
            || ((c == '-' || c == '+') && chars.get(i + 1).map_or(false, |d| d.is_ascii_digit() || *d == '.'))
        {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let num: f32 = chars[start..i].iter().collect::<String>().parse().unwrap_or(0.0);
            let us = i;
            while i < chars.len() && (chars[i].is_ascii_alphabetic() || chars[i] == '%') {
                i += 1;
            }
            let unit: String = chars[us..i].iter().collect::<String>().to_ascii_lowercase();
            out.push(match unit.as_str() {
                "" => Value::Number(num),
                "px" => Value::Length(num, Unit::Px),
                "em" => Value::Length(num, Unit::Em),
                "rem" => Value::Length(num, Unit::Rem),
                "%" => Value::Length(num, Unit::Percent),
                "pt" => Value::Length(num, Unit::Pt),
                "vw" => Value::Length(num, Unit::Vw),
                "vh" => Value::Length(num, Unit::Vh),
                "ex" => Value::Length(num, Unit::Ex),
                "ch" => Value::Length(num, Unit::Ch),
                "cm" => Value::Length(num, Unit::Cm),
                "mm" => Value::Length(num, Unit::Mm),
                "in" => Value::Length(num, Unit::In),
                other => Value::Keyword(format!("{}{}", num, other)),
            });
        } else if c.is_alphanumeric() || c == '-' || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '-' || chars[i] == '_') {
                i += 1;
            }
            let ident: String = chars[start..i].iter().collect::<String>().to_ascii_lowercase();
            if chars.get(i) == Some(&'(') {
                let mut depth = 0;
                let args_start = i + 1;
                while i < chars.len() {
                    if chars[i] == '(' {
                        depth += 1;
                    } else if chars[i] == ')' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    i += 1;
                }
                let args: String = chars[args_start..i.min(chars.len())].iter().collect();
                i += 1;
                match parse_color_fn(&ident, &args) {
                    Some(col) => out.push(Value::Color(col)),
                    None => out.push(Value::Keyword(format!("{}()", ident))),
                }
            } else if let Some(col) = named_color(&ident) {
                out.push(Value::Color(col));
            } else {
                out.push(Value::Keyword(ident));
            }
        } else {
            i += 1;
        }
    }
    out
}

fn parse_hex(hex: &str) -> Option<Color> {
    let digits: Vec<u8> = hex
        .chars()
        .map(|c| c.to_digit(16).map(|d| d as u8))
        .collect::<Option<_>>()?;
    match digits.len() {
        3 | 4 => {
            let a = if digits.len() == 4 { digits[3] * 17 } else { 255 };
            Some(Color {
                r: digits[0] * 17,
                g: digits[1] * 17,
                b: digits[2] * 17,
                a,
            })
        }
        6 | 8 => {
            let a = if digits.len() == 8 {
                digits[6] * 16 + digits[7]
            } else {
                255
            };
            Some(Color {
                r: digits[0] * 16 + digits[1],
                g: digits[2] * 16 + digits[3],
                b: digits[4] * 16 + digits[5],
                a,
            })
        }
        _ => None,
    }
}

fn parse_color_fn(name: &str, args: &str) -> Option<Color> {
    let parts: Vec<&str> = args
        .split(|c: char| c == ',' || c == '/' || c.is_whitespace())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() < 3 {
        return None;
    }
    let num = |p: &str, scale: f32| -> Option<f32> {
        if let Some(pct) = p.strip_suffix('%') {
            pct.parse::<f32>().ok().map(|v| v / 100.0 * scale)
        } else {
            p.trim_end_matches("deg").parse::<f32>().ok()
        }
    };
    let alpha = match parts.get(3) {
        Some(p) => (num(p, 1.0)?.clamp(0.0, 1.0) * 255.0) as u8,
        None => 255,
    };
    match name {
        "rgb" | "rgba" => Some(Color {
            r: num(parts[0], 255.0)?.clamp(0.0, 255.0) as u8,
            g: num(parts[1], 255.0)?.clamp(0.0, 255.0) as u8,
            b: num(parts[2], 255.0)?.clamp(0.0, 255.0) as u8,
            a: alpha,
        }),
        "hsl" | "hsla" => {
            let h = num(parts[0], 360.0)?.rem_euclid(360.0) / 360.0;
            let s = num(parts[1], 1.0)?.clamp(0.0, 1.0);
            let l = num(parts[2], 1.0)?.clamp(0.0, 1.0);
            let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
            let p = 2.0 * l - q;
            let hue = |mut t: f32| {
                t = t.rem_euclid(1.0);
                let v = if t < 1.0 / 6.0 {
                    p + (q - p) * 6.0 * t
                } else if t < 0.5 {
                    q
                } else if t < 2.0 / 3.0 {
                    p + (q - p) * (2.0 / 3.0 - t) * 6.0
                } else {
                    p
                };
                (v * 255.0).round() as u8
            };
            Some(Color {
                r: hue(h + 1.0 / 3.0),
                g: hue(h),
                b: hue(h - 1.0 / 3.0),
                a: alpha,
            })
        }
        _ => None,
    }
}

pub fn named_color(name: &str) -> Option<Color> {
    let rgb = |v: u32| Color::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
    Some(match name {
        "transparent" => Color { r: 0, g: 0, b: 0, a: 0 },
        "black" => rgb(0x000000),
        "white" => rgb(0xffffff),
        "red" => rgb(0xff0000),
        "green" => rgb(0x008000),
        "blue" => rgb(0x0000ff),
        "yellow" => rgb(0xffff00),
        "gray" | "grey" => rgb(0x808080),
        "silver" => rgb(0xc0c0c0),
        "maroon" => rgb(0x800000),
        "purple" => rgb(0x800080),
        "fuchsia" | "magenta" => rgb(0xff00ff),
        "lime" => rgb(0x00ff00),
        "olive" => rgb(0x808000),
        "navy" => rgb(0x000080),
        "teal" => rgb(0x008080),
        "aqua" | "cyan" => rgb(0x00ffff),
        "orange" => rgb(0xffa500),
        "pink" => rgb(0xffc0cb),
        "brown" => rgb(0xa52a2a),
        "darkgray" | "darkgrey" => rgb(0xa9a9a9),
        "lightgray" | "lightgrey" => rgb(0xd3d3d3),
        "dimgray" | "dimgrey" => rgb(0x696969),
        "gainsboro" => rgb(0xdcdcdc),
        "whitesmoke" => rgb(0xf5f5f5),
        "beige" => rgb(0xf5f5dc),
        "ivory" => rgb(0xfffff0),
        "linen" => rgb(0xfaf0e6),
        "lavender" => rgb(0xe6e6fa),
        "gold" => rgb(0xffd700),
        "indigo" => rgb(0x4b0082),
        "violet" => rgb(0xee82ee),
        "coral" => rgb(0xff7f50),
        "salmon" => rgb(0xfa8072),
        "tomato" => rgb(0xff6347),
        "crimson" => rgb(0xdc143c),
        "darkred" => rgb(0x8b0000),
        "darkgreen" => rgb(0x006400),
        "darkblue" => rgb(0x00008b),
        "darkorange" => rgb(0xff8c00),
        "lightblue" => rgb(0xadd8e6),
        "lightgreen" => rgb(0x90ee90),
        "lightyellow" => rgb(0xffffe0),
        "skyblue" => rgb(0x87ceeb),
        "steelblue" => rgb(0x4682b4),
        "royalblue" => rgb(0x4169e1),
        "dodgerblue" => rgb(0x1e90ff),
        "slategray" | "slategrey" => rgb(0x708090),
        "seagreen" => rgb(0x2e8b57),
        "forestgreen" => rgb(0x228b22),
        "firebrick" => rgb(0xb22222),
        "chocolate" => rgb(0xd2691e),
        "tan" => rgb(0xd2b48c),
        "khaki" => rgb(0xf0e68c),
        "aliceblue" => rgb(0xf0f8ff),
        "ghostwhite" => rgb(0xf8f8ff),
        "honeydew" => rgb(0xf0fff0),
        "mintcream" => rgb(0xf5fffa),
        "snow" => rgb(0xfffafa),
        "wheat" => rgb(0xf5deb3),
        _ => return None,
    })
}

/// Parses a comma-separated selector list. Returns `None` if any selector is unsupported.
pub fn parse_selector_list(s: &str) -> Option<Vec<Selector>> {
    let parts = split_top_level(s, ',');
    let sels: Option<Vec<Selector>> = parts.iter().map(|p| parse_selector(p.trim())).collect();
    sels.filter(|v| !v.is_empty())
}

/// Parses one complex selector and computes its specificity as `ids * 10000 + classes * 100 + types`.
fn parse_selector(s: &str) -> Option<Selector> {
    if s.is_empty() {
        return None;
    }
    let chars: Vec<char> = s.chars().collect();
    let mut compounds = Vec::new();
    let mut combinators = Vec::new();
    let mut i = 0;
    let mut pending: Option<Combinator> = None;
    let (mut a, mut b, mut c) = (0u32, 0u32, 0u32);
    let ident_char = |ch: char| ch.is_alphanumeric() || ch == '-' || ch == '_' || ch == '\\';
    loop {
        let mut saw_ws = false;
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
            saw_ws = true;
        }
        if i >= chars.len() {
            break;
        }
        match chars[i] {
            '>' => {
                pending = Some(Combinator::Child);
                i += 1;
                continue;
            }
            '+' | '~' => return None,
            _ => {}
        }
        if !compounds.is_empty() {
            combinators.push(
                pending
                    .take()
                    .unwrap_or(if saw_ws { Combinator::Descendant } else { return None }),
            );
        }
        let mut comp = Compound::default();
        let start = i;
        while i < chars.len() && !chars[i].is_whitespace() && !matches!(chars[i], '>' | '+' | '~') {
            let ch = chars[i];
            let read_ident = |i: &mut usize| -> String {
                let st = *i;
                while *i < chars.len() && ident_char(chars[*i]) {
                    *i += 1;
                }
                chars[st..*i].iter().collect::<String>().replace('\\', "")
            };
            if ch == '*' {
                i += 1;
            } else if ch == '#' {
                i += 1;
                comp.id = Some(read_ident(&mut i));
                a += 1;
            } else if ch == '.' {
                i += 1;
                comp.classes.push(read_ident(&mut i));
                b += 1;
            } else if ch == '[' {
                let end = chars[i..].iter().position(|&x| x == ']').map(|p| p + i)?;
                let inner: String = chars[i + 1..end].iter().collect();
                i = end + 1;
                comp.attrs.push(parse_attr_selector(&inner)?);
                b += 1;
            } else if ch == ':' {
                i += 1;
                if chars.get(i) == Some(&':') {
                    return None;
                }
                let name = read_ident(&mut i).to_ascii_lowercase();
                if chars.get(i) == Some(&'(') {
                    return None;
                }
                match name.as_str() {
                    "root" => comp.root = true,
                    "link" | "any-link" => comp.attrs.push(("href".into(), AttrOp::Exists)),
                    _ => return None,
                }
                b += 1;
            } else if ident_char(ch) && i == start {
                comp.tag = Some(read_ident(&mut i).to_ascii_lowercase());
                c += 1;
            } else {
                return None;
            }
        }
        compounds.push(comp);
    }
    if compounds.is_empty() || pending.is_some() {
        return None;
    }
    Some(Selector {
        compounds,
        combinators,
        specificity: a * 10000 + b * 100 + c,
    })
}

fn parse_attr_selector(inner: &str) -> Option<(String, AttrOp)> {
    let inner = inner.trim();
    let ops = ["~=", "^=", "$=", "*=", "|=", "="];
    for op in ops {
        if let Some(idx) = inner.find(op) {
            let name = inner[..idx].trim().to_ascii_lowercase();
            let mut val = inner[idx + op.len()..].trim();
            if let Some(stripped) = val.strip_suffix(" i").or_else(|| val.strip_suffix(" s")) {
                val = stripped.trim();
            }
            let val = val.trim_matches(|c| c == '"' || c == '\'').to_string();
            let op = match op {
                "~=" => AttrOp::Word(val),
                "^=" => AttrOp::Prefix(val),
                "$=" => AttrOp::Suffix(val),
                "*=" => AttrOp::Contains(val),
                "|=" => AttrOp::Dash(val),
                _ => AttrOp::Eq(val),
            };
            return Some((name, op));
        }
    }
    Some((inner.to_ascii_lowercase(), AttrOp::Exists))
}

/// Evaluates a media query list against a screen of the given width.
pub fn media_matches(query: &str, viewport_w: f32) -> bool {
    query.split(',').any(|q| media_query_matches(q.trim(), viewport_w))
}

fn media_query_matches(q: &str, vw: f32) -> bool {
    let (negate, q) = match q.strip_prefix("not ") {
        Some(rest) => (true, rest),
        None => (false, q.strip_prefix("only ").unwrap_or(q)),
    };
    let mut ok = true;
    for word in q.split(|c: char| c == '(' || c == ')').map(str::trim) {
        let w = word.trim_start_matches("and ").trim_end_matches(" and").trim();
        if w.is_empty() || w == "and" {
            continue;
        }
        if matches!(w, "print" | "speech" | "tv" | "handheld" | "projection") {
            ok = false;
        } else if let Some((name, value)) = w.split_once(':') {
            let name = name.trim();
            let value = value.trim();
            let px = length_px(value);
            ok &= match name {
                "min-width" => px.map_or(true, |p| vw >= p),
                "max-width" => px.map_or(true, |p| vw <= p),
                "prefers-color-scheme" => value == "light",
                "prefers-reduced-motion" => value == "no-preference",
                "prefers-contrast" | "forced-colors" | "inverted-colors" => value == "no-preference" || value == "none",
                "hover" | "any-hover" => value == "hover",
                "pointer" | "any-pointer" => value == "fine",
                "orientation" => value == "landscape",
                _ => true,
            };
        } else if w.contains("width") && (w.contains('<') || w.contains('>')) {
            ok &= range_matches(w, vw);
        }
    }
    ok != negate
}

fn range_matches(expr: &str, vw: f32) -> bool {
    let cleaned = expr.replace(' ', "");
    for (op, f) in [
        (">=", (|a: f32, b: f32| a >= b) as fn(f32, f32) -> bool),
        ("<=", |a, b| a <= b),
        (">", |a, b| a > b),
        ("<", |a, b| a < b),
    ] {
        if let Some((l, r)) = cleaned.split_once(op) {
            if l == "width" {
                return length_px(r).map_or(true, |p| f(vw, p));
            }
            if r == "width" {
                return length_px(l).map_or(true, |p| f(p, vw));
            }
        }
    }
    true
}

fn length_px(v: &str) -> Option<f32> {
    match parse_value(v).first()? {
        Value::Length(n, Unit::Px) => Some(*n),
        Value::Length(n, Unit::Em) | Value::Length(n, Unit::Rem) => Some(n * 16.0),
        Value::Number(n) => Some(*n),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_specificity() {
        let sel = &parse_selector_list("div#main .item a").unwrap()[0];
        assert_eq!(sel.specificity, 10000 + 100 + 2);
        assert_eq!(sel.combinators, [Combinator::Descendant, Combinator::Descendant]);
    }

    #[test]
    fn rejects_unsupported_selectors_without_dropping_the_rule() {
        let sheet = parse_stylesheet("a:hover, p { color: red }");
        assert_eq!(sheet.rules.len(), 1);
        assert_eq!(sheet.rules[0].selectors.len(), 1);
        assert_eq!(sheet.rules[0].selectors[0].compounds[0].tag.as_deref(), Some("p"));
    }

    #[test]
    fn parses_colors() {
        assert_eq!(parse_value("#f80")[0], Value::Color(Color::rgb(0xff, 0x88, 0x00)));
        assert_eq!(parse_value("rgb(10, 20, 30)")[0], Value::Color(Color::rgb(10, 20, 30)));
        assert_eq!(
            parse_value("hsl(120, 100%, 50%)")[0],
            Value::Color(Color::rgb(0, 255, 0))
        );
        assert_eq!(parse_value("navy")[0], Value::Color(Color::rgb(0, 0, 128)));
    }

    #[test]
    fn parses_important_and_lengths() {
        let decls = parse_declarations("margin: 1.5em 10px !important; width: 50%");
        assert!(decls[0].important);
        assert_eq!(
            decls[0].value,
            [Value::Length(1.5, Unit::Em), Value::Length(10.0, Unit::Px)]
        );
        assert_eq!(decls[1].value, [Value::Length(50.0, Unit::Percent)]);
    }

    #[test]
    fn evaluates_media_queries() {
        assert!(media_matches("screen and (min-width: 600px)", 1024.0));
        assert!(!media_matches("(max-width: 600px)", 1024.0));
        assert!(!media_matches("print", 1024.0));
        assert!(media_matches("print, (width >= 800px)", 1024.0));
    }

    #[test]
    fn keeps_rules_inside_media_blocks() {
        let sheet =
            parse_stylesheet("@media (max-width: 500px) { p { color: red } } @import 'x.css'; div { color: blue }");
        assert_eq!(sheet.rules.len(), 2);
        assert_eq!(sheet.rules[0].media.as_deref(), Some("(max-width: 500px)"));
        assert_eq!(sheet.rules[1].media, None);
    }
}
