//! Selector matching, the cascade and computed styles.
//!
//! [`style_tree`] walks the DOM, collects matching declarations from the user-agent sheet,
//! author sheets, presentational attributes and `style` attributes, sorts them by importance,
//! origin, specificity and source order, and resolves them into a [`Style`] per element.

use crate::css::{
    AttrOp, Color, Combinator, Compound, Declaration, Selector, Stylesheet, Unit, Value, media_matches,
    parse_declarations,
};
use crate::dom::{ElementData, Node, NodeType};
use std::collections::HashMap;
use std::rc::Rc;

/// The subset of `display` values that layout distinguishes.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Display {
    Block,
    Inline,
    ListItem,
    Table,
    RowGroup,
    TableRow,
    TableCell,
    None,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum VAlign {
    Top,
    Middle,
    Bottom,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Align {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Family {
    Serif,
    Sans,
    Mono,
}

/// A length that may depend on the containing block.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Len {
    Px(f32),
    Percent(f32),
    Auto,
}

impl Len {
    pub fn resolve(self, base: f32) -> Option<f32> {
        match self {
            Len::Px(v) => Some(v),
            Len::Percent(p) => Some(base * p / 100.0),
            Len::Auto => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LineHeight {
    Normal,
    Px(f32),
    Factor(f32),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ListStyle {
    Disc,
    Circle,
    Square,
    Decimal,
    LowerAlpha,
    UpperAlpha,
    LowerRoman,
    UpperRoman,
    None,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Transform {
    None,
    Upper,
    Lower,
    Capitalize,
}

/// Computed style of an element. Lengths are in pixels unless they depend on layout.
#[derive(Clone, Debug)]
pub struct Style {
    pub display: Display,
    pub color: Color,
    pub background: Option<Color>,
    pub font_size: f32,
    pub bold: bool,
    pub italic: bool,
    pub family: Family,
    pub underline: bool,
    pub strike: bool,
    pub text_align: Align,
    pub line_height: LineHeight,
    pub preserve_ws: bool,
    pub no_wrap: bool,
    pub margin: [Len; 4],
    pub padding: [Len; 4],
    pub border_width: [f32; 4],
    pub border_visible: [bool; 4],
    pub border_color: [Option<Color>; 4],
    pub width: Len,
    pub max_width: Len,
    pub height: Len,
    pub min_height: Len,
    pub border_box: bool,
    pub list_style: ListStyle,
    pub transform: Transform,
    pub hidden: bool,
    pub valign: VAlign,
    pub border_spacing: f32,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            display: Display::Inline,
            color: Color::BLACK,
            background: None,
            font_size: 16.0,
            bold: false,
            italic: false,
            family: Family::Serif,
            underline: false,
            strike: false,
            text_align: Align::Left,
            line_height: LineHeight::Normal,
            preserve_ws: false,
            no_wrap: false,
            margin: [Len::Px(0.0); 4],
            padding: [Len::Px(0.0); 4],
            border_width: [3.0; 4],
            border_visible: [false; 4],
            border_color: [None; 4],
            width: Len::Auto,
            max_width: Len::Auto,
            height: Len::Auto,
            min_height: Len::Auto,
            border_box: false,
            list_style: ListStyle::Disc,
            transform: Transform::None,
            hidden: false,
            valign: VAlign::Top,
            border_spacing: 2.0,
        }
    }
}

impl Style {
    fn inherit(parent: &Style) -> Style {
        Style {
            color: parent.color,
            font_size: parent.font_size,
            bold: parent.bold,
            italic: parent.italic,
            family: parent.family,
            underline: parent.underline,
            strike: parent.strike,
            text_align: parent.text_align,
            line_height: parent.line_height,
            preserve_ws: parent.preserve_ws,
            no_wrap: parent.no_wrap,
            list_style: parent.list_style,
            transform: parent.transform,
            hidden: parent.hidden,
            border_spacing: parent.border_spacing,
            ..Style::default()
        }
    }

    pub fn line_height_px(&self) -> f32 {
        match self.line_height {
            LineHeight::Normal => self.font_size * 1.2,
            LineHeight::Px(p) => p,
            LineHeight::Factor(f) => self.font_size * f,
        }
    }

    pub fn border(&self, side: usize) -> f32 {
        if self.border_visible[side] {
            self.border_width[side]
        } else {
            0.0
        }
    }
}

/// A DOM node paired with its computed style. `block_level` is true for blocks and for inline elements that contain blocks.
pub struct StyledNode<'a> {
    pub node: &'a Node,
    pub style: Rc<Style>,
    pub children: Vec<StyledNode<'a>>,
    pub block_level: bool,
}

/// Where a stylesheet came from, in increasing cascade priority.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Origin {
    UserAgent,
    Author,
}

struct IndexedRule<'a> {
    selector: &'a Selector,
    decls: &'a [Declaration],
    origin: Origin,
    order: usize,
}

struct Ctx<'a> {
    rules: Vec<IndexedRule<'a>>,
    by_id: HashMap<&'a str, Vec<usize>>,
    by_class: HashMap<&'a str, Vec<usize>>,
    by_tag: HashMap<&'a str, Vec<usize>>,
    universal: Vec<usize>,
    root_font: f32,
    vw: f32,
    vh: f32,
}

/// Computes styles for the whole document at the given viewport size. Returns `None` if there is no `html` element.
pub fn style_tree<'a>(doc: &'a Node, sheets: &'a [(Stylesheet, Origin)], vw: f32, vh: f32) -> Option<StyledNode<'a>> {
    let mut ctx = Ctx {
        rules: Vec::new(),
        by_id: HashMap::new(),
        by_class: HashMap::new(),
        by_tag: HashMap::new(),
        universal: Vec::new(),
        root_font: 16.0,
        vw,
        vh,
    };
    for (sheet, origin) in sheets {
        for rule in &sheet.rules {
            if let Some(m) = &rule.media {
                if !media_matches(m, vw) {
                    continue;
                }
            }
            for selector in &rule.selectors {
                let idx = ctx.rules.len();
                ctx.rules.push(IndexedRule {
                    selector,
                    decls: &rule.declarations,
                    origin: *origin,
                    order: idx + 1,
                });
                let last = selector.compounds.last().unwrap();
                if let Some(id) = &last.id {
                    ctx.by_id.entry(id.as_str()).or_default().push(idx);
                } else if let Some(class) = last.classes.first() {
                    ctx.by_class.entry(class.as_str()).or_default().push(idx);
                } else if let Some(tag) = &last.tag {
                    ctx.by_tag.entry(tag.as_str()).or_default().push(idx);
                } else {
                    ctx.universal.push(idx);
                }
            }
        }
    }
    let html = doc.children.iter().find(|c| c.tag() == Some("html"))?;
    let root_parent = Style {
        display: Display::Block,
        ..Style::default()
    };
    let mut ancestors = Vec::new();
    Some(build(html, &root_parent, &mut ancestors, &mut ctx))
}

fn build<'a>(node: &'a Node, parent: &Style, ancestors: &mut Vec<&'a ElementData>, ctx: &mut Ctx) -> StyledNode<'a> {
    let el = match &node.node_type {
        NodeType::Element(e) => e,
        _ => {
            let style = Rc::new(Style {
                display: Display::Inline,
                ..parent.clone()
            });
            return StyledNode {
                node,
                style,
                children: Vec::new(),
                block_level: false,
            };
        }
    };
    let style = compute_style(el, parent, ancestors, ctx);
    if el.tag == "html" {
        ctx.root_font = style.font_size;
    }
    let style = Rc::new(style);
    let mut children = Vec::new();
    if style.display != Display::None {
        ancestors.push(el);
        for child in &node.children {
            let sn = build(child, &style, ancestors, ctx);
            if sn.style.display != Display::None {
                children.push(sn);
            }
        }
        ancestors.pop();
    }
    let block_level = match style.display {
        Display::Inline => children.iter().any(|c| c.block_level),
        Display::None => false,
        _ => true,
    };
    StyledNode {
        node,
        style,
        children,
        block_level,
    }
}

/// Runs the cascade for one element: candidate rules come from an index keyed on each selector's rightmost ID, class or tag.
fn compute_style(el: &ElementData, parent: &Style, ancestors: &[&ElementData], ctx: &Ctx) -> Style {
    let mut candidates: Vec<usize> = ctx.universal.clone();
    if let Some(v) = ctx.by_tag.get(el.tag.as_str()) {
        candidates.extend(v);
    }
    if let Some(id) = el.id() {
        if let Some(v) = ctx.by_id.get(id) {
            candidates.extend(v);
        }
    }
    for class in el.classes() {
        if let Some(v) = ctx.by_class.get(class) {
            candidates.extend(v);
        }
    }
    candidates.sort_unstable();
    candidates.dedup();

    let hints = presentational_hints(el, ancestors);
    let inline = el.attr("style").map(parse_declarations).unwrap_or_default();

    let mut matched: Vec<((bool, Origin, u32, usize), &Declaration)> = Vec::new();
    for d in &hints {
        matched.push(((d.important, Origin::Author, 0, 0), d));
    }
    for idx in candidates {
        let r = &ctx.rules[idx];
        if matches(r.selector, el, ancestors) {
            for d in r.decls {
                matched.push(((d.important, r.origin, r.selector.specificity, r.order), d));
            }
        }
    }
    for d in &inline {
        matched.push(((d.important, Origin::Author, u32::MAX, usize::MAX), d));
    }
    matched.sort_by(|a, b| a.0.cmp(&b.0));

    let mut style = Style::inherit(parent);
    for (_, d) in &matched {
        if d.name == "font-size" || d.name == "font" {
            apply(&mut style, parent, d, ctx);
        }
    }
    for (_, d) in &matched {
        if d.name != "font-size" && d.name != "font" {
            apply(&mut style, parent, d, ctx);
        }
    }
    if el.attrs.contains_key("hidden") {
        style.display = Display::None;
    }
    style
}

/// Translates legacy attributes such as `bgcolor`, `align` and `cellpadding` into declarations with zero specificity.
fn presentational_hints(el: &ElementData, ancestors: &[&ElementData]) -> Vec<Declaration> {
    let mut css = String::new();
    let table = ancestors.iter().rev().find(|a| a.tag == "table");
    let px_or_pct = |w: &str| {
        let w = w.trim();
        if w.ends_with('%') {
            w.to_string()
        } else {
            format!("{}px", w.trim_end_matches("px"))
        }
    };
    if el.tag == "table" {
        if let Some(b) = el
            .attr("border")
            .and_then(|b| b.trim().parse::<f32>().ok().or(Some(1.0)))
        {
            if b > 0.0 {
                css += &format!("border: {}px outset #808080;", b);
            }
        }
        if let Some(sp) = el.attr("cellspacing").and_then(|v| v.trim().parse::<f32>().ok()) {
            css += &format!("border-spacing: {}px;", sp);
        }
    }
    if matches!(el.tag.as_str(), "td" | "th") {
        if let Some(t) = table {
            if t.attr("border")
                .map_or(false, |b| b.trim().parse::<f32>().map_or(true, |v| v > 0.0))
            {
                css += "border: 1px inset #808080;";
            }
            if let Some(cp) = t.attr("cellpadding").and_then(|v| v.trim().parse::<f32>().ok()) {
                css += &format!("padding: {}px;", cp);
            }
        }
        if let Some(w) = el.attr("width") {
            css += &format!("width: {};", px_or_pct(w));
        }
    }
    if let Some(v) = el.attr("valign") {
        css += &format!("vertical-align: {};", v.to_ascii_lowercase());
    }
    let color_attr = |v: &str| {
        let v = v.trim();
        if v.len() == 6 && v.chars().all(|c| c.is_ascii_hexdigit()) {
            format!("#{}", v)
        } else {
            v.to_string()
        }
    };
    if let Some(bg) = el.attr("bgcolor") {
        css += &format!("background-color: {};", color_attr(bg));
    }
    if el.tag == "body" {
        if let Some(t) = el.attr("text") {
            css += &format!("color: {};", color_attr(t));
        }
    }
    if el.tag == "font" {
        if let Some(c) = el.attr("color") {
            css += &format!("color: {};", color_attr(c));
        }
        if let Some(face) = el.attr("face") {
            css += &format!("font-family: {};", face);
        }
        if let Some(size) = el.attr("size").and_then(|s| s.trim().parse::<i32>().ok()) {
            let px = [10, 13, 16, 18, 24, 32, 48][(size.clamp(1, 7) - 1) as usize];
            css += &format!("font-size: {}px;", px);
        }
    }
    if let Some(align) = el.attr("align") {
        let a = align.to_ascii_lowercase();
        if el.tag == "table" && a == "center" {
            css += "margin-left: auto; margin-right: auto;";
        } else if matches!(a.as_str(), "left" | "right" | "center" | "justify") {
            css += &format!("text-align: {};", a);
        }
    }
    if el.tag == "table" {
        if let Some(w) = el.attr("width") {
            css += &format!("width: {};", px_or_pct(w));
        }
    }
    if el.attrs.contains_key("nowrap") {
        css += "white-space: nowrap;";
    }
    if css.is_empty() {
        Vec::new()
    } else {
        parse_declarations(&css)
    }
}

/// Matches a selector against an element and its ancestors, right to left.
pub fn matches(sel: &Selector, el: &ElementData, ancestors: &[&ElementData]) -> bool {
    let n = sel.compounds.len();
    matches_compound(&sel.compounds[n - 1], el) && match_rest(sel, n - 1, ancestors)
}

fn match_rest(sel: &Selector, idx: usize, ancestors: &[&ElementData]) -> bool {
    if idx == 0 {
        return true;
    }
    let target = &sel.compounds[idx - 1];
    match sel.combinators[idx - 1] {
        Combinator::Child => match ancestors.split_last() {
            Some((p, rest)) => matches_compound(target, p) && match_rest(sel, idx - 1, rest),
            None => false,
        },
        Combinator::Descendant => (0..ancestors.len())
            .rev()
            .any(|k| matches_compound(target, ancestors[k]) && match_rest(sel, idx - 1, &ancestors[..k])),
    }
}

fn matches_compound(c: &Compound, el: &ElementData) -> bool {
    if let Some(t) = &c.tag {
        if *t != el.tag {
            return false;
        }
    }
    if c.root && el.tag != "html" {
        return false;
    }
    if let Some(id) = &c.id {
        if el.id() != Some(id.as_str()) {
            return false;
        }
    }
    if !c.classes.iter().all(|cl| el.has_class(cl)) {
        return false;
    }
    c.attrs.iter().all(|(name, op)| {
        let Some(v) = el.attr(name) else { return false };
        match op {
            AttrOp::Exists => true,
            AttrOp::Eq(x) => v == x,
            AttrOp::Word(x) => v.split_ascii_whitespace().any(|w| w == x),
            AttrOp::Prefix(x) => !x.is_empty() && v.starts_with(x.as_str()),
            AttrOp::Suffix(x) => !x.is_empty() && v.ends_with(x.as_str()),
            AttrOp::Contains(x) => !x.is_empty() && v.contains(x.as_str()),
            AttrOp::Dash(x) => v == x || v.starts_with(&format!("{}-", x)),
        }
    })
}

fn to_px(v: &Value, font_size: f32, ctx: &Ctx) -> Option<f32> {
    Some(match v {
        Value::Length(n, u) => match u {
            Unit::Px => *n,
            Unit::Em => n * font_size,
            Unit::Rem => n * ctx.root_font,
            Unit::Pt => n * 4.0 / 3.0,
            Unit::Vw => n * ctx.vw / 100.0,
            Unit::Vh => n * ctx.vh / 100.0,
            Unit::Ex | Unit::Ch => n * font_size * 0.5,
            Unit::Cm => n * 96.0 / 2.54,
            Unit::Mm => n * 96.0 / 25.4,
            Unit::In => n * 96.0,
            Unit::Percent => return None,
        },
        Value::Number(n) if *n == 0.0 => 0.0,
        _ => return None,
    })
}

fn to_len(v: &Value, font_size: f32, ctx: &Ctx) -> Option<Len> {
    match v {
        Value::Length(p, Unit::Percent) => Some(Len::Percent(*p)),
        Value::Keyword(k) if k == "auto" || k == "none" => Some(Len::Auto),
        _ => to_px(v, font_size, ctx).map(Len::Px),
    }
}

fn four<T: Copy>(vals: &[T]) -> Option<[T; 4]> {
    match vals.len() {
        1 => Some([vals[0]; 4]),
        2 => Some([vals[0], vals[1], vals[0], vals[1]]),
        3 => Some([vals[0], vals[1], vals[2], vals[1]]),
        4 => Some([vals[0], vals[1], vals[2], vals[3]]),
        _ => None,
    }
}

fn side_index(name: &str) -> Option<usize> {
    if name.contains("-top") {
        Some(0)
    } else if name.contains("-right") {
        Some(1)
    } else if name.contains("-bottom") {
        Some(2)
    } else if name.contains("-left") {
        Some(3)
    } else {
        None
    }
}

fn border_style_visible(k: &str) -> Option<bool> {
    match k {
        "none" | "hidden" => Some(false),
        "solid" | "dashed" | "dotted" | "double" | "groove" | "ridge" | "inset" | "outset" => Some(true),
        _ => None,
    }
}

fn border_width_kw(v: &Value, fs: f32, ctx: &Ctx) -> Option<f32> {
    match v {
        Value::Keyword(k) => match k.as_str() {
            "thin" => Some(1.0),
            "medium" => Some(3.0),
            "thick" => Some(5.0),
            _ => None,
        },
        _ => to_px(v, fs, ctx),
    }
}

fn font_size_value(v: &Value, parent: &Style, ctx: &Ctx) -> Option<f32> {
    match v {
        Value::Length(p, Unit::Percent) => Some(parent.font_size * p / 100.0),
        Value::Keyword(k) => Some(match k.as_str() {
            "xx-small" => 9.0,
            "x-small" => 10.0,
            "small" => 13.0,
            "medium" => 16.0,
            "large" => 18.0,
            "x-large" => 24.0,
            "xx-large" => 32.0,
            "xxx-large" => 48.0,
            "smaller" => parent.font_size / 1.2,
            "larger" => parent.font_size * 1.2,
            _ => return None,
        }),
        _ => to_px(v, parent.font_size, ctx),
    }
}

fn family_of(values: &[Value]) -> Option<Family> {
    for v in values {
        let name = match v {
            Value::Str(s) => s.to_ascii_lowercase(),
            Value::Keyword(k) => k.clone(),
            _ => continue,
        };
        let n = name.as_str();
        if n.contains("mono")
            || n.contains("courier")
            || n.contains("consol")
            || n.contains("menlo")
            || n == "monaco"
            || n.contains("code")
        {
            return Some(Family::Mono);
        }
        if n == "serif"
            || n.contains("times")
            || n.contains("georgia")
            || n.contains("garamond")
            || n.contains("palatino")
            || n.contains("book")
            || n == "cambria"
        {
            return Some(Family::Serif);
        }
        if n.contains("sans")
            || n.contains("arial")
            || n.contains("helvetica")
            || n.contains("verdana")
            || n.contains("system")
            || n.contains("segoe")
            || n.contains("roboto")
            || n.contains("tahoma")
            || n.contains("apple")
            || n.contains("inter")
            || n.contains("ubuntu")
            || n.contains("lato")
        {
            return Some(Family::Sans);
        }
    }
    values
        .iter()
        .any(|v| matches!(v, Value::Str(_) | Value::Keyword(_)))
        .then_some(Family::Sans)
}

fn line_height_value(v: &Value, fs: f32, ctx: &Ctx) -> Option<LineHeight> {
    match v {
        Value::Keyword(k) if k == "normal" => Some(LineHeight::Normal),
        Value::Number(n) => Some(LineHeight::Factor(*n)),
        Value::Length(p, Unit::Percent) => Some(LineHeight::Px(fs * p / 100.0)),
        _ => to_px(v, fs, ctx).map(LineHeight::Px),
    }
}

/// Applies one declaration to a style. Unknown properties and invalid values are ignored.
fn apply(s: &mut Style, parent: &Style, d: &Declaration, ctx: &Ctx) {
    let vals = &d.value;
    let first = &vals[0];
    let kw = match first {
        Value::Keyword(k) => k.as_str(),
        _ => "",
    };
    let fs = s.font_size;
    let name = d.name.as_str();
    match name {
        "display" => {
            s.display = match kw {
                "none" => Display::None,
                "inline" | "inline-block" | "inline-flex" | "inline-grid" | "contents" | "ruby" => Display::Inline,
                "list-item" => Display::ListItem,
                "table" | "inline-table" => Display::Table,
                "table-row" => Display::TableRow,
                "table-row-group" | "table-header-group" | "table-footer-group" => Display::RowGroup,
                "table-cell" => Display::TableCell,
                "" => return,
                _ => Display::Block,
            }
        }
        "color" => {
            if let Value::Color(c) = first {
                s.color = *c;
            }
        }
        "background-color" | "background" => {
            if kw == "none" || kw == "transparent" {
                s.background = None;
            }
            if let Some(c) = vals
                .iter()
                .find_map(|v| if let Value::Color(c) = v { Some(*c) } else { None })
            {
                s.background = if c.a == 0 { None } else { Some(c) };
            }
        }
        "font-size" => {
            if let Some(px) = font_size_value(first, parent, ctx) {
                s.font_size = px;
            }
        }
        "font-weight" => match first {
            Value::Number(n) => s.bold = *n >= 600.0,
            Value::Keyword(k) => match k.as_str() {
                "bold" | "bolder" => s.bold = true,
                "normal" | "lighter" => s.bold = false,
                _ => {}
            },
            _ => {}
        },
        "font-style" => match kw {
            "italic" | "oblique" => s.italic = true,
            "normal" => s.italic = false,
            _ => {}
        },
        "font-family" => {
            if let Some(f) = family_of(vals) {
                s.family = f;
            }
        }
        "font" => {
            let mut i = 0;
            let mut bold = false;
            let mut italic = false;
            while i < vals.len() {
                match &vals[i] {
                    Value::Keyword(k) if k == "bold" || k == "bolder" => bold = true,
                    Value::Keyword(k) if k == "italic" || k == "oblique" => italic = true,
                    Value::Keyword(k) if matches!(k.as_str(), "normal" | "small-caps" | "lighter") => {}
                    Value::Number(n) => bold = *n >= 600.0,
                    _ => break,
                }
                i += 1;
            }
            let Some(size) = vals.get(i).and_then(|v| font_size_value(v, parent, ctx)) else {
                return;
            };
            s.font_size = size;
            s.bold = bold;
            s.italic = italic;
            s.line_height = LineHeight::Normal;
            i += 1;
            if vals.get(i) == Some(&Value::Slash) {
                if let Some(lh) = vals.get(i + 1).and_then(|v| line_height_value(v, size, ctx)) {
                    s.line_height = lh;
                }
                i += 2;
            }
            if let Some(f) = family_of(&vals[i.min(vals.len())..]) {
                s.family = f;
            }
        }
        "text-decoration" | "text-decoration-line" => {
            for v in vals {
                if let Value::Keyword(k) = v {
                    match k.as_str() {
                        "underline" => s.underline = true,
                        "line-through" => s.strike = true,
                        "none" => {
                            s.underline = false;
                            s.strike = false;
                        }
                        _ => {}
                    }
                }
            }
        }
        "text-align" => {
            s.text_align = match kw {
                "center" | "-webkit-center" | "-moz-center" => Align::Center,
                "right" | "end" | "-webkit-right" => Align::Right,
                "left" | "start" | "justify" | "-webkit-left" => Align::Left,
                _ => return,
            }
        }
        "line-height" => {
            if let Some(lh) = line_height_value(first, fs, ctx) {
                s.line_height = lh;
            }
        }
        "white-space" => {
            let (p, n) = match kw {
                "pre" => (true, true),
                "pre-wrap" | "break-spaces" => (true, false),
                "nowrap" => (false, true),
                "normal" | "pre-line" => (false, false),
                _ => return,
            };
            s.preserve_ws = p;
            s.no_wrap = n;
        }
        "margin" | "padding" => {
            let lens: Vec<Len> = vals.iter().filter_map(|v| to_len(v, fs, ctx)).collect();
            if lens.len() != vals.len() {
                return;
            }
            if let Some(f) = four(&lens) {
                if name == "margin" {
                    s.margin = f;
                } else {
                    s.padding = f.map(|l| if l == Len::Auto { Len::Px(0.0) } else { l });
                }
            }
        }
        "margin-top" | "margin-right" | "margin-bottom" | "margin-left" | "padding-top" | "padding-right"
        | "padding-bottom" | "padding-left" => {
            if let (Some(i), Some(l)) = (side_index(name), to_len(first, fs, ctx)) {
                if name.starts_with("margin") {
                    s.margin[i] = l;
                } else if l != Len::Auto {
                    s.padding[i] = l;
                }
            }
        }
        "margin-inline" | "padding-inline" | "margin-block" | "padding-block" => {
            let lens: Vec<Len> = vals.iter().filter_map(|v| to_len(v, fs, ctx)).collect();
            if lens.is_empty() {
                return;
            }
            let (a, b) = (lens[0], *lens.get(1).unwrap_or(&lens[0]));
            let target = if name.starts_with("margin") {
                &mut s.margin
            } else {
                &mut s.padding
            };
            if name.ends_with("inline") {
                target[3] = a;
                target[1] = b;
            } else {
                target[0] = a;
                target[2] = b;
            }
        }
        "border" | "border-top" | "border-right" | "border-bottom" | "border-left" => {
            let mut width = 3.0;
            let mut visible = false;
            let mut color = None;
            for v in vals {
                if let Value::Color(c) = v {
                    color = Some(*c);
                } else if let Some(w) = border_width_kw(v, fs, ctx) {
                    width = w;
                } else if let Value::Keyword(k) = v {
                    if let Some(vis) = border_style_visible(k) {
                        visible = vis;
                    }
                }
            }
            let sides: Vec<usize> = match side_index(name) {
                Some(i) => vec![i],
                None => vec![0, 1, 2, 3],
            };
            for i in sides {
                s.border_width[i] = width;
                s.border_visible[i] = visible;
                s.border_color[i] = color;
            }
        }
        "border-width" => {
            let ws: Vec<f32> = vals.iter().filter_map(|v| border_width_kw(v, fs, ctx)).collect();
            if let Some(f) = four(&ws) {
                s.border_width = f;
            }
        }
        "border-style" => {
            let st: Vec<bool> = vals
                .iter()
                .filter_map(|v| {
                    if let Value::Keyword(k) = v {
                        border_style_visible(k)
                    } else {
                        None
                    }
                })
                .collect();
            if let Some(f) = four(&st) {
                s.border_visible = f;
            }
        }
        "border-color" => {
            let cs: Vec<Option<Color>> = vals
                .iter()
                .filter_map(|v| if let Value::Color(c) = v { Some(Some(*c)) } else { None })
                .collect();
            if let Some(f) = four(&cs) {
                s.border_color = f;
            }
        }
        "border-top-width" | "border-right-width" | "border-bottom-width" | "border-left-width" => {
            if let (Some(i), Some(w)) = (side_index(name), border_width_kw(first, fs, ctx)) {
                s.border_width[i] = w;
            }
        }
        "border-top-style" | "border-right-style" | "border-bottom-style" | "border-left-style" => {
            if let (Some(i), Some(v)) = (side_index(name), border_style_visible(kw)) {
                s.border_visible[i] = v;
            }
        }
        "border-top-color" | "border-right-color" | "border-bottom-color" | "border-left-color" => {
            if let (Some(i), Value::Color(c)) = (side_index(name), first) {
                s.border_color[i] = Some(*c);
            }
        }
        "width" => {
            if let Some(l) = to_len(first, fs, ctx) {
                s.width = l;
            }
        }
        "max-width" => {
            if let Some(l) = to_len(first, fs, ctx) {
                s.max_width = l;
            }
        }
        "height" => {
            if let Some(l) = to_len(first, fs, ctx) {
                s.height = l;
            }
        }
        "min-height" => {
            if let Some(l) = to_len(first, fs, ctx) {
                s.min_height = l;
            }
        }
        "box-sizing" => s.border_box = kw == "border-box",
        "list-style-type" | "list-style" => {
            for v in vals {
                if let Value::Keyword(k) = v {
                    s.list_style = match k.as_str() {
                        "disc" => ListStyle::Disc,
                        "circle" => ListStyle::Circle,
                        "square" => ListStyle::Square,
                        "decimal" | "decimal-leading-zero" => ListStyle::Decimal,
                        "lower-alpha" | "lower-latin" => ListStyle::LowerAlpha,
                        "upper-alpha" | "upper-latin" => ListStyle::UpperAlpha,
                        "lower-roman" => ListStyle::LowerRoman,
                        "upper-roman" => ListStyle::UpperRoman,
                        "none" => ListStyle::None,
                        _ => continue,
                    };
                }
            }
        }
        "text-transform" => {
            s.transform = match kw {
                "uppercase" => Transform::Upper,
                "lowercase" => Transform::Lower,
                "capitalize" => Transform::Capitalize,
                "none" => Transform::None,
                _ => return,
            }
        }
        "visibility" => s.hidden = kw == "hidden" || kw == "collapse",
        "vertical-align" => {
            s.valign = match kw {
                "middle" => VAlign::Middle,
                "bottom" | "text-bottom" => VAlign::Bottom,
                "top" | "text-top" | "baseline" => VAlign::Top,
                _ => return,
            }
        }
        "border-spacing" => {
            if let Some(px) = to_px(first, fs, ctx) {
                s.border_spacing = px;
            }
        }
        "border-collapse" => {
            if kw == "collapse" {
                s.border_spacing = 0.0;
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::parse_stylesheet;

    fn find<'a>(node: &'a StyledNode<'a>, tag: &str) -> Option<&'a StyledNode<'a>> {
        if node.node.tag() == Some(tag) {
            return Some(node);
        }
        node.children.iter().find_map(|c| find(c, tag))
    }

    fn styled(html: &str, css: &str, check: impl FnOnce(&StyledNode)) {
        let doc = crate::html::parse(html);
        let sheets = vec![
            (parse_stylesheet(include_str!("ua.css")), Origin::UserAgent),
            (parse_stylesheet(css), Origin::Author),
        ];
        let tree = style_tree(&doc, &sheets, 1024.0, 768.0).unwrap();
        check(&tree);
    }

    #[test]
    fn id_beats_class_and_important_beats_id() {
        styled(
            r#"<p id="x" class="c">a</p><span id="y" class="d">b</span>"#,
            "#x { color: red } .c { color: blue } .d { color: green !important } #y { color: red }",
            |t| {
                assert_eq!(find(t, "p").unwrap().style.color, Color::rgb(255, 0, 0));
                assert_eq!(find(t, "span").unwrap().style.color, Color::rgb(0, 128, 0));
            },
        );
    }

    #[test]
    fn inline_style_wins_and_color_inherits() {
        styled(
            r#"<div style="color: #00f"><em>x</em></div>"#,
            "div { color: red }",
            |t| {
                assert_eq!(find(t, "em").unwrap().style.color, Color::rgb(0, 0, 255));
            },
        );
    }

    #[test]
    fn resolves_em_against_own_font_size() {
        styled("<h1>t</h1>", "h1 { font-size: 20px; margin-top: 2em }", |t| {
            let h1 = find(t, "h1").unwrap();
            assert_eq!(h1.style.font_size, 20.0);
            assert_eq!(h1.style.margin[0], Len::Px(40.0));
        });
    }

    #[test]
    fn child_combinator_only_matches_direct_children() {
        styled(
            "<div><p>a</p><section><span>b</span></section></div>",
            "div > span { color: red } div > p { color: red }",
            |t| {
                assert_eq!(find(t, "p").unwrap().style.color, Color::rgb(255, 0, 0));
                assert_eq!(find(t, "span").unwrap().style.color, Color::BLACK);
            },
        );
    }

    #[test]
    fn hidden_attribute_and_display_none_remove_elements() {
        styled("<p hidden>a</p><div>b</div>", "div { display: none }", |t| {
            assert!(find(t, "p").is_none());
            assert!(find(t, "div").is_none());
        });
    }
}
