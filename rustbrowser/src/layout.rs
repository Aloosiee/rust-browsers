//! Box tree construction and layout.
//!
//! Styled nodes become block, table, row and anonymous inline boxes. Block layout follows CSS 2
//! normal flow (auto margins, `max-width`, margin collapsing), inline layout breaks text into
//! lines, and tables size their columns from the minimum and maximum content widths of cells.

use crate::css::Color;
use crate::dom::NodeType;
use crate::font::{FontId, FontStore};
use crate::style::{Align, Display, Len, ListStyle, Style, StyledNode, Transform, VAlign};
use std::rc::Rc;

/// An axis-aligned rectangle in CSS pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }

    fn expand(&self, e: [f32; 4]) -> Rect {
        Rect {
            x: self.x - e[3],
            y: self.y - e[0],
            w: self.w + e[1] + e[3],
            h: self.h + e[0] + e[2],
        }
    }
}

/// CSS box model edges around a content rectangle. Edge arrays are ordered top, right, bottom, left.
#[derive(Clone, Copy, Debug, Default)]
pub struct Dims {
    pub content: Rect,
    pub padding: [f32; 4],
    pub border: [f32; 4],
    pub margin: [f32; 4],
}

impl Dims {
    pub fn padding_box(&self) -> Rect {
        self.content.expand(self.padding)
    }
    pub fn border_box(&self) -> Rect {
        self.padding_box().expand(self.border)
    }
    pub fn margin_box(&self) -> Rect {
        self.border_box().expand(self.margin)
    }
}

/// Flattened inline content waiting to be broken into lines.
pub enum InlineItem {
    Text {
        text: String,
        style: Rc<Style>,
        link: Option<Rc<str>>,
        elem: usize,
    },
    Break(Rc<Style>),
    Anchor(String),
}

pub enum Marker {
    Bullet(ListStyle),
    Text(String),
}

/// A run of text with uniform style placed on a line.
pub struct Frag {
    pub rect: Rect,
    pub baseline: f32,
    pub text: String,
    pub font: FontId,
    pub style: Rc<Style>,
    pub link: Option<Rc<str>>,
    pub elem: usize,
}

/// What a layout box contains.
pub enum BoxKind {
    Block {
        marker: Option<Marker>,
        anchor: Option<String>,
    },
    Inline {
        items: Vec<InlineItem>,
        frags: Vec<Frag>,
    },
    Table,
    Row,
}

/// A node of the layout tree. `node` is the DOM id of the element that generated it.
pub struct LayoutBox {
    pub dims: Dims,
    pub style: Rc<Style>,
    pub kind: BoxKind,
    pub children: Vec<LayoutBox>,
    pub first_baseline: Option<f32>,
    pub span: usize,
    pub pref_width: Option<f32>,
    pub node: usize,
}

thread_local! {
    static FOCUSED: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

fn new_box(style: Rc<Style>, kind: BoxKind, children: Vec<LayoutBox>) -> LayoutBox {
    LayoutBox {
        dims: Dims::default(),
        style,
        kind,
        children,
        first_baseline: None,
        span: 1,
        pref_width: None,
        node: 0,
    }
}

/// Builds the box tree. `focused` marks the text field that should draw a caret.
pub fn build_layout_tree(root: &StyledNode, focused: Option<usize>) -> LayoutBox {
    FOCUSED.with(|f| f.set(focused));
    build_block(root, None)
}

/// Builds a block box, wrapping runs of inline children in anonymous inline boxes.
fn build_block(sn: &StyledNode, marker: Option<Marker>) -> LayoutBox {
    let el = sn.node.as_element();
    let anchor = el.and_then(|e| e.id()).map(str::to_string);
    let mut counter: i64 = el
        .and_then(|e| e.attr("start"))
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(1);
    let mut children = Vec::new();
    let mut items = Vec::new();
    for child in &sn.children {
        if child.block_level {
            flush(&mut items, &mut children, &sn.style, sn.node.id);
            let mut m = None;
            if child.style.display == Display::ListItem {
                if let Some(v) = child
                    .node
                    .as_element()
                    .and_then(|e| e.attr("value"))
                    .and_then(|v| v.trim().parse().ok())
                {
                    counter = v;
                }
                m = make_marker(child.style.list_style, counter);
                counter += 1;
            }
            if child.style.display == Display::Table {
                children.push(build_table(child));
            } else {
                children.push(build_block(child, m));
            }
        } else {
            collect_inline(child, &mut items, None, sn.node.id);
        }
    }
    flush(&mut items, &mut children, &sn.style, sn.node.id);
    let mut b = new_box(sn.style.clone(), BoxKind::Block { marker, anchor }, children);
    b.node = sn.node.id;
    b
}

fn build_table(sn: &StyledNode) -> LayoutBox {
    let mut rows = Vec::new();
    collect_rows(sn, &mut rows);
    let mut b = new_box(sn.style.clone(), BoxKind::Table, rows);
    b.node = sn.node.id;
    b
}

fn collect_rows(sn: &StyledNode, out: &mut Vec<LayoutBox>) {
    let mut loose = Vec::new();
    let flush_loose = |loose: &mut Vec<LayoutBox>, out: &mut Vec<LayoutBox>| {
        if !loose.is_empty() {
            out.push(new_box(sn.style.clone(), BoxKind::Row, std::mem::take(loose)));
        }
    };
    for child in &sn.children {
        match child.style.display {
            Display::RowGroup => {
                flush_loose(&mut loose, out);
                collect_rows(child, out);
            }
            Display::TableRow => {
                flush_loose(&mut loose, out);
                let cells = child
                    .children
                    .iter()
                    .filter(|c| c.block_level)
                    .map(build_cell)
                    .collect();
                let mut row = new_box(child.style.clone(), BoxKind::Row, cells);
                row.node = child.node.id;
                out.push(row);
            }
            Display::TableCell => loose.push(build_cell(child)),
            _ if child.block_level => {
                flush_loose(&mut loose, out);
                out.push(build_block(child, None));
            }
            _ => {}
        }
    }
    flush_loose(&mut loose, out);
}

/// Builds a table cell. The cell's own width becomes a column hint and is cleared from its style.
fn build_cell(sn: &StyledNode) -> LayoutBox {
    let mut b = if sn.style.display == Display::Table {
        build_table(sn)
    } else {
        build_block(sn, None)
    };
    b.span = sn
        .node
        .as_element()
        .and_then(|e| e.attr("colspan"))
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(1)
        .clamp(1, 64);
    if let Len::Px(w) = sn.style.width {
        b.pref_width = Some(w);
    }
    if matches!(b.kind, BoxKind::Block { .. }) {
        b.style = derived(&sn.style, |s| {
            s.width = Len::Auto;
            s.max_width = Len::Auto;
            s.margin = [Len::Px(0.0); 4];
        });
    }
    b
}

fn is_ws(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c')
}

fn flush(items: &mut Vec<InlineItem>, children: &mut Vec<LayoutBox>, style: &Rc<Style>, node: usize) {
    let meaningful = items.iter().any(|i| match i {
        InlineItem::Text { text, style, .. } => style.preserve_ws || !text.chars().all(is_ws),
        _ => true,
    });
    if meaningful {
        let mut b = new_box(
            style.clone(),
            BoxKind::Inline {
                items: std::mem::take(items),
                frags: Vec::new(),
            },
            Vec::new(),
        );
        b.node = node;
        children.push(b);
    } else {
        items.clear();
    }
}

fn make_marker(ls: ListStyle, n: i64) -> Option<Marker> {
    Some(match ls {
        ListStyle::None => return None,
        ListStyle::Disc | ListStyle::Circle | ListStyle::Square => Marker::Bullet(ls),
        ListStyle::Decimal => Marker::Text(format!("{}.", n)),
        ListStyle::LowerAlpha | ListStyle::UpperAlpha => {
            let mut s = String::new();
            let mut k = n.max(1);
            while k > 0 {
                k -= 1;
                s.insert(0, (b'a' + (k % 26) as u8) as char);
                k /= 26;
            }
            if ls == ListStyle::UpperAlpha {
                s = s.to_uppercase();
            }
            Marker::Text(format!("{}.", s))
        }
        ListStyle::LowerRoman | ListStyle::UpperRoman => {
            let r = roman(n);
            Marker::Text(format!(
                "{}.",
                if ls == ListStyle::LowerRoman {
                    r.to_lowercase()
                } else {
                    r
                }
            ))
        }
    })
}

fn roman(mut n: i64) -> String {
    if n <= 0 || n >= 4000 {
        return n.to_string();
    }
    let table = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut s = String::new();
    for (v, r) in table {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

fn derived(style: &Rc<Style>, f: impl FnOnce(&mut Style)) -> Rc<Style> {
    let mut s = (**style).clone();
    f(&mut s);
    Rc::new(s)
}

/// Flattens an inline subtree into items, rendering form controls as styled text.
fn collect_inline(sn: &StyledNode, items: &mut Vec<InlineItem>, link: Option<Rc<str>>, parent_elem: usize) {
    let style = &sn.style;
    let el = match &sn.node.node_type {
        NodeType::Text(t) => {
            items.push(InlineItem::Text {
                text: t.clone(),
                style: style.clone(),
                link,
                elem: parent_elem,
            });
            return;
        }
        NodeType::Element(e) => e,
        NodeType::Document => return,
    };
    if let Some(id) = el.id() {
        items.push(InlineItem::Anchor(id.to_string()));
    }
    let mut link = link;
    if el.tag == "a" {
        if let Some(name) = el.attr("name") {
            items.push(InlineItem::Anchor(name.to_string()));
        }
        if let Some(href) = el.attr("href") {
            link = Some(Rc::from(href.trim()));
        }
    }
    let elem = sn.node.id;
    let focused = FOCUSED.with(|f| f.get()) == Some(elem);
    let boxed = |text: String, bg: Color, fg: Color| InlineItem::Text {
        elem,
        text,
        style: derived(style, |s| {
            s.background = Some(bg);
            s.color = fg;
            s.preserve_ws = true;
            s.no_wrap = true;
            s.underline = false;
        }),
        link: None,
    };
    let field_bg = Color::rgb(0xf1, 0xf1, 0xf1);
    let focus_bg = Color::rgb(0xdc, 0xe9, 0xfd);
    let button_bg = Color::rgb(0xe2, 0xe2, 0xe6);
    match el.tag.as_str() {
        "br" => items.push(InlineItem::Break(style.clone())),
        "img" => {
            let alt = el.attr("alt").unwrap_or("").trim();
            if !alt.is_empty() {
                let s = derived(style, |s| {
                    s.color = Color::rgb(0x70, 0x70, 0x70);
                    s.italic = true;
                });
                items.push(InlineItem::Text {
                    text: format!("[{}]", alt),
                    style: s,
                    link,
                    elem,
                });
            }
        }
        "input" => {
            let kind = el.attr("type").unwrap_or("text").to_ascii_lowercase();
            let value = el.attr("value").unwrap_or("");
            let item = match kind.as_str() {
                "hidden" => return,
                "checkbox" => boxed(
                    if el.attrs.contains_key("checked") { "[x]" } else { "[ ]" }.into(),
                    field_bg,
                    style.color,
                ),
                "radio" => boxed(
                    if el.attrs.contains_key("checked") { "(o)" } else { "( )" }.into(),
                    field_bg,
                    style.color,
                ),
                "submit" | "button" | "reset" | "image" => {
                    let label = if value.is_empty() {
                        if kind == "reset" { "Reset" } else { "Submit" }
                    } else {
                        value
                    };
                    boxed(format!(" {} ", label), button_bg, Color::BLACK)
                }
                _ => {
                    let (text, fg) = if !value.is_empty() {
                        let shown = if kind == "password" {
                            "•".repeat(value.chars().count())
                        } else {
                            value.to_string()
                        };
                        (shown, Color::BLACK)
                    } else {
                        (
                            el.attr("placeholder").unwrap_or("").to_string(),
                            Color::rgb(0x80, 0x80, 0x80),
                        )
                    };
                    let (text, fg) = if focused && value.is_empty() {
                        (String::new(), Color::BLACK)
                    } else {
                        (text, fg)
                    };
                    let count = text.chars().count();
                    let mut shown: String = text.chars().skip(count.saturating_sub(28)).collect();
                    if focused {
                        shown.push('|');
                    }
                    let pad = 22usize.saturating_sub(shown.chars().count());
                    boxed(
                        format!(" {}{} ", shown, " ".repeat(pad)),
                        if focused { focus_bg } else { field_bg },
                        fg,
                    )
                }
            };
            items.push(item);
        }
        "textarea" => {
            let text = el
                .attr("value")
                .map(str::to_string)
                .unwrap_or_else(|| sn.node.text_content());
            let last = text.lines().last().unwrap_or("");
            let count = last.chars().count();
            let mut shown: String = last.chars().skip(count.saturating_sub(40)).collect();
            if focused {
                shown.push('|');
            }
            let pad = 30usize.saturating_sub(shown.chars().count());
            items.push(boxed(
                format!(" {}{} ", shown, " ".repeat(pad)),
                if focused { focus_bg } else { field_bg },
                Color::BLACK,
            ));
        }
        "select" => {
            let mut options = Vec::new();
            sn.node.find_all("option", &mut options);
            let chosen = options
                .iter()
                .find(|o| o.as_element().map_or(false, |e| e.attrs.contains_key("selected")))
                .or(options.first())
                .map(|o| o.text_content().trim().to_string())
                .unwrap_or_default();
            items.push(boxed(format!(" {} v ", chosen), button_bg, Color::BLACK));
        }
        _ => {
            for child in &sn.children {
                collect_inline(child, items, link.clone(), elem);
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum PieceKind {
    Word,
    Space,
    HardSpace,
}

struct Piece {
    kind: PieceKind,
    text: String,
    width: f32,
    style: Option<Rc<Style>>,
    font: FontId,
    link: Option<Rc<str>>,
    elem: usize,
}

/// Accumulates pieces on the current line and turns finished lines into fragments.
struct LineBuilder<'a> {
    fonts: &'a FontStore,
    x0: f32,
    width: f32,
    align: Align,
    y: f32,
    pieces: Vec<(Piece, f32)>,
    cursor: f32,
    frags: Vec<Frag>,
    anchors_pending: Vec<String>,
    anchors: &'a mut Vec<(String, f32)>,
    first_baseline: Option<f32>,
}

impl<'a> LineBuilder<'a> {
    fn has_content(&self) -> bool {
        self.pieces
            .iter()
            .any(|(p, _)| p.kind == PieceKind::Word || p.kind == PieceKind::HardSpace)
    }

    fn place(&mut self, p: Piece) {
        let w = p.width;
        self.pieces.push((p, self.cursor));
        self.cursor += w;
    }

    /// Ends the current line: trims trailing spaces, aligns it, and merges adjacent pieces that share a style.
    fn finish(&mut self, forced_style: Option<&Rc<Style>>) {
        while matches!(self.pieces.last(), Some((p, _)) if p.kind == PieceKind::Space) {
            self.pieces.pop();
        }
        for name in self.anchors_pending.drain(..) {
            self.anchors.push((name, self.y));
        }
        if self.pieces.is_empty() {
            if let Some(s) = forced_style {
                self.y += s.line_height_px();
            }
            self.cursor = 0.0;
            return;
        }
        let mut above: f32 = 0.0;
        let mut below: f32 = 0.0;
        let mut metrics = Vec::with_capacity(self.pieces.len());
        for (p, _) in &self.pieces {
            let Some(style) = p.style.as_ref() else {
                metrics.push((0.0, 0.0));
                continue;
            };
            let (asc, desc) = self.fonts.ascent_descent(p.font, style.font_size);
            let lh = style.line_height_px();
            let half = (lh - (asc + desc)) / 2.0;
            above = above.max(half + asc);
            below = below.max(lh - half - asc);
            metrics.push((asc, desc));
        }
        let content_w = self.pieces.last().map(|(p, x)| x + p.width).unwrap_or(0.0);
        let offset = match self.align {
            Align::Left => 0.0,
            Align::Center => ((self.width - content_w) / 2.0).max(0.0),
            Align::Right => (self.width - content_w).max(0.0),
        };
        let baseline = self.y + above;
        if self.first_baseline.is_none() {
            self.first_baseline = Some(baseline);
        }
        let mut current: Option<Frag> = None;
        for ((p, x), (asc, desc)) in self.pieces.drain(..).zip(metrics) {
            let fx = self.x0 + offset + x;
            let Some(style) = p.style.clone() else {
                if let Some(f) = current.take() {
                    self.frags.push(f);
                }
                continue;
            };
            if let Some(f) = current.as_mut() {
                let same = f.font == p.font
                    && f.elem == p.elem
                    && Rc::ptr_eq(&f.style, &style)
                    && match (&f.link, &p.link) {
                        (Some(a), Some(b)) => Rc::ptr_eq(a, b),
                        (None, None) => true,
                        _ => false,
                    }
                    && (f.rect.x + f.rect.w - fx).abs() < 0.5;
                if same {
                    f.text.push_str(&p.text);
                    f.rect.w += p.width;
                    continue;
                }
                self.frags.push(current.take().unwrap());
            }
            current = Some(Frag {
                rect: Rect {
                    x: fx,
                    y: baseline - asc,
                    w: p.width,
                    h: asc + desc,
                },
                baseline,
                text: p.text,
                font: p.font,
                style,
                link: p.link,
                elem: p.elem,
            });
        }
        if let Some(f) = current {
            self.frags.push(f);
        }
        self.y = baseline + below;
        self.cursor = 0.0;
    }
}

fn transform(text: &str, t: Transform, at_word_start: &mut bool) -> String {
    match t {
        Transform::None => text.to_string(),
        Transform::Upper => text.to_uppercase(),
        Transform::Lower => text.to_lowercase(),
        Transform::Capitalize => text
            .chars()
            .map(|c| {
                let out = if *at_word_start {
                    c.to_uppercase().collect::<String>()
                } else {
                    c.to_string()
                };
                *at_word_start = is_ws(c);
                out
            })
            .collect(),
    }
}

impl LayoutBox {
    /// Lays out this box inside a containing block whose content starts at `cb_x` and is `cb_w` wide; `top` is the border-box top.
    pub fn layout(&mut self, cb_x: f32, cb_w: f32, top: f32, fonts: &FontStore, anchors: &mut Vec<(String, f32)>) {
        match self.kind {
            BoxKind::Inline { .. } => self.layout_inline(cb_x, cb_w, top, fonts, anchors),
            BoxKind::Table => self.layout_table(cb_x, cb_w, top, fonts, anchors),
            _ => self.layout_block(cb_x, cb_w, top, fonts, anchors),
        }
    }

    fn collapses_top(&self) -> bool {
        matches!(self.kind, BoxKind::Block { .. })
            && self.style.border(0) == 0.0
            && self.style.padding[0].resolve(0.0).unwrap_or(0.0) == 0.0
            && self
                .children
                .first()
                .map_or(false, |c| !matches!(c.kind, BoxKind::Inline { .. }))
    }

    fn collapses_bottom(&self) -> bool {
        matches!(self.kind, BoxKind::Block { .. })
            && self.style.border(2) == 0.0
            && self.style.padding[2].resolve(0.0).unwrap_or(0.0) == 0.0
            && self.style.height == Len::Auto
            && self.style.min_height == Len::Auto
            && self
                .children
                .last()
                .map_or(false, |c| !matches!(c.kind, BoxKind::Inline { .. }))
    }

    /// Top and bottom margins after collapsing with the first and last child where CSS allows it.
    pub fn vertical_margins(&self, cb_w: f32) -> (f32, f32) {
        match self.kind {
            BoxKind::Inline { .. } | BoxKind::Row => (0.0, 0.0),
            BoxKind::Block { .. } | BoxKind::Table => {
                let mut mt = self.style.margin[0].resolve(cb_w).unwrap_or(0.0);
                let mut mb = self.style.margin[2].resolve(cb_w).unwrap_or(0.0);
                if self.collapses_top() {
                    mt = collapse(mt, self.children[0].vertical_margins(cb_w).0);
                }
                if self.collapses_bottom() {
                    mb = collapse(mb, self.children.last().unwrap().vertical_margins(cb_w).1);
                }
                (mt, mb)
            }
        }
    }

    /// Normal-flow block layout: width and horizontal margins first, then children top to bottom.
    fn layout_block(&mut self, cb_x: f32, cb_w: f32, top: f32, fonts: &FontStore, anchors: &mut Vec<(String, f32)>) {
        let s = self.style.clone();
        let pad: [f32; 4] = [0, 1, 2, 3].map(|i| s.padding[i].resolve(cb_w).unwrap_or(0.0).max(0.0));
        let bor: [f32; 4] = [0, 1, 2, 3].map(|i| s.border(i));
        let extra = pad[1] + pad[3] + bor[1] + bor[3];
        let ml = s.margin[3].resolve(cb_w);
        let mr = s.margin[1].resolve(cb_w);
        let adjust = |w: f32| if s.border_box { (w - extra).max(0.0) } else { w };
        let mut width = s.width.resolve(cb_w).map(adjust);
        let max_w = s.max_width.resolve(cb_w).map(adjust);
        if let (Some(w), Some(m)) = (width, max_w) {
            width = Some(w.min(m));
        }
        if width.is_none() {
            let auto_w = cb_w - ml.unwrap_or(0.0) - mr.unwrap_or(0.0) - extra;
            if let Some(m) = max_w {
                if auto_w > m {
                    width = Some(m);
                }
            }
        }
        let (w, l, r) = match width {
            None => {
                let l = ml.unwrap_or(0.0);
                let r = mr.unwrap_or(0.0);
                ((cb_w - l - r - extra).max(0.0), l, r)
            }
            Some(w) => {
                let rem = cb_w - w - extra - ml.unwrap_or(0.0) - mr.unwrap_or(0.0);
                match (ml, mr) {
                    (None, None) => (w, rem.max(0.0) / 2.0, rem.max(0.0) / 2.0),
                    (None, Some(r)) => (w, rem.max(0.0), r),
                    (Some(l), None) => (w, l, rem),
                    (Some(l), Some(r)) => (w, l, r),
                }
            }
        };
        let (mt, mb) = self.vertical_margins(cb_w);
        self.dims.padding = pad;
        self.dims.border = bor;
        self.dims.margin = [mt, r, mb, l];
        self.dims.content = Rect {
            x: cb_x + l + bor[3] + pad[3],
            y: top + bor[0] + pad[0],
            w,
            h: 0.0,
        };

        if let BoxKind::Block { anchor: Some(a), .. } = &self.kind {
            anchors.push((a.clone(), top));
        }
        let content = self.dims.content;
        let mut cursor = content.y;
        let mut prev_mb: f32 = 0.0;
        let mut first = true;
        let mut first_baseline = None;
        let through_top = self.collapses_top();
        let through_bottom = self.collapses_bottom();
        for child in &mut self.children {
            let (cmt, cmb) = child.vertical_margins(content.w);
            let child_top = if first {
                cursor + if through_top { 0.0 } else { cmt }
            } else {
                cursor + collapse(prev_mb, cmt)
            };
            child.layout(content.x, content.w, child_top, fonts, anchors);
            let bb = child.dims.border_box();
            cursor = bb.y + bb.h;
            prev_mb = cmb;
            first = false;
            if first_baseline.is_none() {
                first_baseline = child.first_baseline;
            }
        }
        let trailing = if self.children.is_empty() || through_bottom {
            0.0
        } else {
            prev_mb
        };
        let mut h = (cursor + trailing - content.y).max(0.0);
        if let Len::Px(px) = s.height {
            h = if s.border_box {
                (px - pad[0] - pad[2] - bor[0] - bor[2]).max(0.0)
            } else {
                px
            };
        }
        if let Len::Px(px) = s.min_height {
            h = h.max(px);
        }
        self.dims.content.h = h;
        self.first_baseline = first_baseline;
    }

    /// Breaks inline items into lines and positions the resulting fragments.
    fn layout_inline(&mut self, cb_x: f32, cb_w: f32, top: f32, fonts: &FontStore, anchors: &mut Vec<(String, f32)>) {
        let align = self.style.text_align;
        let BoxKind::Inline { items, frags } = &mut self.kind else {
            return;
        };
        let mut lb = LineBuilder {
            fonts,
            x0: cb_x,
            width: cb_w,
            align,
            y: top,
            pieces: Vec::new(),
            cursor: 0.0,
            frags: Vec::new(),
            anchors_pending: Vec::new(),
            anchors,
            first_baseline: None,
        };
        let mut last_space = true;
        let mut word_start = true;
        for item in items.iter() {
            match item {
                InlineItem::Anchor(name) => lb.anchors_pending.push(name.clone()),
                InlineItem::Break(style) => {
                    lb.finish(Some(style));
                    last_space = true;
                }
                InlineItem::Text {
                    text,
                    style,
                    link,
                    elem,
                } => {
                    let font = fonts.font_for(style.family, style.bold, style.italic);
                    let size = style.font_size;
                    let text = transform(text, style.transform, &mut word_start);
                    let mk = |kind: PieceKind, t: String| Piece {
                        width: fonts.measure(&t, font, size),
                        kind,
                        text: t,
                        style: Some(style.clone()),
                        font,
                        link: link.clone(),
                        elem: *elem,
                    };
                    if style.preserve_ws {
                        let text = text.replace('\t', "    ").replace("\r\n", "\n").replace('\r', "\n");
                        for (i, line) in text.split('\n').enumerate() {
                            if i > 0 {
                                lb.finish(Some(style));
                            }
                            for run in split_runs(line) {
                                let kind = if run.starts_with(' ') {
                                    PieceKind::HardSpace
                                } else {
                                    PieceKind::Word
                                };
                                let p = mk(kind, run.to_string());
                                if !style.no_wrap
                                    && kind == PieceKind::Word
                                    && lb.cursor + p.width > cb_w
                                    && lb.has_content()
                                {
                                    lb.finish(None);
                                }
                                lb.place(p);
                            }
                        }
                        last_space = false;
                        continue;
                    }
                    for run in split_runs_ws(&text) {
                        if run.chars().next().map_or(false, is_ws) {
                            if !last_space {
                                let p = mk(PieceKind::Space, " ".to_string());
                                if lb.has_content() {
                                    lb.place(p);
                                }
                                last_space = true;
                            }
                            continue;
                        }
                        last_space = false;
                        let p = mk(PieceKind::Word, run.to_string());
                        if !style.no_wrap && lb.cursor + p.width > cb_w && lb.has_content() {
                            lb.finish(None);
                        }
                        if p.width > cb_w && !style.no_wrap {
                            let mut chunk = String::new();
                            let mut cw = 0.0;
                            for c in run.chars() {
                                let a = fonts.advance(font, c, size);
                                if cw + a > cb_w && !chunk.is_empty() {
                                    lb.place(mk(PieceKind::Word, std::mem::take(&mut chunk)));
                                    lb.finish(None);
                                    cw = 0.0;
                                }
                                chunk.push(c);
                                cw += a;
                            }
                            if !chunk.is_empty() {
                                lb.place(mk(PieceKind::Word, chunk));
                            }
                        } else {
                            lb.place(p);
                        }
                    }
                }
            }
        }
        lb.finish(None);
        let height = lb.y - top;
        *frags = std::mem::take(&mut lb.frags);
        self.first_baseline = lb.first_baseline;
        self.dims = Dims {
            content: Rect {
                x: cb_x,
                y: top,
                w: cb_w,
                h: height,
            },
            ..Dims::default()
        };
    }
}

fn split_runs(s: &str) -> Vec<&str> {
    split_by(s, |c| c == ' ')
}

fn split_runs_ws(s: &str) -> Vec<&str> {
    split_by(s, is_ws)
}

fn split_by(s: &str, pred: impl Fn(char) -> bool) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut prev: Option<bool> = None;
    for (i, c) in s.char_indices() {
        let k = pred(c);
        if prev.map_or(false, |p| p != k) {
            out.push(&s[start..i]);
            start = i;
        }
        prev = Some(k);
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

fn inline_intrinsic(items: &[InlineItem], fonts: &FontStore) -> (f32, f32) {
    let (mut min, mut max, mut line, mut word) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    let mut last_space = true;
    for item in items {
        match item {
            InlineItem::Text { text, style, .. } => {
                let font = fonts.font_for(style.family, style.bold, style.italic);
                let size = style.font_size;
                if style.preserve_ws {
                    for (i, seg) in text.split('\n').enumerate() {
                        if i > 0 {
                            max = max.max(line);
                            line = 0.0;
                            word = 0.0;
                        }
                        let w = fonts.measure(&seg.replace('\t', "    "), font, size);
                        line += w;
                        word += w;
                        min = min.max(word);
                    }
                    last_space = false;
                    continue;
                }
                for run in split_runs_ws(text) {
                    if run.chars().next().map_or(false, is_ws) {
                        if !last_space {
                            let sw = fonts.advance(font, ' ', size);
                            line += sw;
                            if style.no_wrap {
                                word += sw;
                            } else {
                                word = 0.0;
                            }
                        }
                        last_space = true;
                    } else {
                        let w = fonts.measure(run, font, size);
                        line += w;
                        word += w;
                        min = min.max(word);
                        last_space = false;
                    }
                }
            }
            InlineItem::Break(_) => {
                max = max.max(line);
                line = 0.0;
                word = 0.0;
                last_space = true;
            }
            InlineItem::Anchor(_) => {}
        }
    }
    (min, max.max(line))
}

impl LayoutBox {
    fn horizontal_extras(&self) -> f32 {
        let s = &self.style;
        let px = |l: Len| if let Len::Px(v) = l { v.max(0.0) } else { 0.0 };
        px(s.padding[1]) + px(s.padding[3]) + s.border(1) + s.border(3) + px(s.margin[1]) + px(s.margin[3])
    }

    /// Minimum and maximum content widths, used to size table columns.
    pub fn intrinsic(&self, fonts: &FontStore) -> (f32, f32) {
        let (min, max) = match &self.kind {
            BoxKind::Inline { items, .. } => return inline_intrinsic(items, fonts),
            BoxKind::Table => {
                let (mins, maxs) = self.table_columns(fonts);
                let sp = self.style.border_spacing * (mins.len() as f32 + 1.0);
                (mins.iter().sum::<f32>() + sp, maxs.iter().sum::<f32>() + sp)
            }
            BoxKind::Row => (0.0, 0.0),
            BoxKind::Block { .. } => self.children.iter().fold((0.0f32, 0.0f32), |(a, b), c| {
                let (cmin, cmax) = c.intrinsic(fonts);
                (a.max(cmin), b.max(cmax))
            }),
        };
        let extra = self.horizontal_extras();
        let specified = self.pref_width.or(if let Len::Px(w) = self.style.width {
            Some(w)
        } else {
            None
        });
        match specified {
            Some(w) => {
                let w = if self.style.border_box { w } else { w + extra };
                let lo = (min + extra).max(w);
                (lo, lo)
            }
            None => (min + extra, (max + extra).max(min + extra)),
        }
    }

    /// Per-column minimum and maximum widths. Spanning cells distribute any shortfall evenly.
    fn table_columns(&self, fonts: &FontStore) -> (Vec<f32>, Vec<f32>) {
        let rows: Vec<&LayoutBox> = self
            .children
            .iter()
            .filter(|c| matches!(c.kind, BoxKind::Row))
            .collect();
        let n = rows
            .iter()
            .map(|r| r.children.iter().map(|c| c.span).sum::<usize>())
            .max()
            .unwrap_or(0);
        let mut mins = vec![0.0f32; n];
        let mut maxs = vec![0.0f32; n];
        let sp = self.style.border_spacing;
        let mut spanning = Vec::new();
        for row in &rows {
            let mut col = 0;
            for cell in &row.children {
                let (cmin, cmax) = cell.intrinsic(fonts);
                if cell.span == 1 {
                    mins[col] = mins[col].max(cmin);
                    maxs[col] = maxs[col].max(cmax);
                } else {
                    spanning.push((col, cell.span.min(n - col), cmin, cmax));
                }
                col += cell.span;
            }
        }
        for (col, span, cmin, cmax) in spanning {
            let gaps = sp * (span as f32 - 1.0);
            let cur_min: f32 = mins[col..col + span].iter().sum::<f32>() + gaps;
            if cmin > cur_min {
                let add = (cmin - cur_min) / span as f32;
                mins[col..col + span].iter_mut().for_each(|m| *m += add);
            }
            let cur_max: f32 = maxs[col..col + span].iter().sum::<f32>() + gaps;
            if cmax > cur_max {
                let add = (cmax - cur_max) / span as f32;
                maxs[col..col + span].iter_mut().for_each(|m| *m += add);
            }
        }
        for i in 0..n {
            maxs[i] = maxs[i].max(mins[i]);
        }
        (mins, maxs)
    }

    fn translate(&mut self, dy: f32) {
        self.dims.content.y += dy;
        if let Some(b) = self.first_baseline.as_mut() {
            *b += dy;
        }
        if let BoxKind::Inline { frags, .. } = &mut self.kind {
            for f in frags {
                f.rect.y += dy;
                f.baseline += dy;
            }
        }
        for c in &mut self.children {
            c.translate(dy);
        }
    }

    /// Automatic table layout: shrink-to-fit width, columns scaled between their min and max widths, rows as tall as their tallest cell.
    fn layout_table(&mut self, cb_x: f32, cb_w: f32, top: f32, fonts: &FontStore, anchors: &mut Vec<(String, f32)>) {
        let s = self.style.clone();
        let pad: [f32; 4] = [0, 1, 2, 3].map(|i| s.padding[i].resolve(cb_w).unwrap_or(0.0).max(0.0));
        let bor: [f32; 4] = [0, 1, 2, 3].map(|i| s.border(i));
        let extra = pad[1] + pad[3] + bor[1] + bor[3];
        let ml = s.margin[3].resolve(cb_w);
        let mr = s.margin[1].resolve(cb_w);
        let sp = s.border_spacing;
        let (mins, maxs) = self.table_columns(fonts);
        let n = mins.len();
        let gaps = if n > 0 { sp * (n as f32 + 1.0) } else { 0.0 };
        let (smin, smax) = (mins.iter().sum::<f32>(), maxs.iter().sum::<f32>());
        let avail = cb_w - ml.unwrap_or(0.0) - mr.unwrap_or(0.0) - extra;
        let spec = s.width.resolve(cb_w).map(|w| if s.border_box { w - extra } else { w });
        let w = match spec {
            Some(w) => w.max(smin + gaps),
            None => (smax + gaps).min(avail).max(smin + gaps),
        };
        let rem = cb_w - w - extra - ml.unwrap_or(0.0) - mr.unwrap_or(0.0);
        let (l, r) = match (ml, mr) {
            (None, None) => (rem.max(0.0) / 2.0, rem.max(0.0) / 2.0),
            (None, Some(r)) => (rem.max(0.0), r),
            (Some(l), None) => (l, rem),
            (Some(l), Some(r)) => (l, r),
        };
        let (mt, mb) = self.vertical_margins(cb_w);
        self.dims.padding = pad;
        self.dims.border = bor;
        self.dims.margin = [mt, r, mb, l];
        self.dims.content = Rect {
            x: cb_x + l + bor[3] + pad[3],
            y: top + bor[0] + pad[0],
            w,
            h: 0.0,
        };

        let target = w - gaps;
        let widths: Vec<f32> = if target >= smax {
            if smax > 0.0 {
                maxs.iter().map(|m| m + (target - smax) * m / smax).collect()
            } else {
                vec![target / n.max(1) as f32; n]
            }
        } else if target > smin && smax > smin {
            (0..n)
                .map(|i| mins[i] + (maxs[i] - mins[i]) * (target - smin) / (smax - smin))
                .collect()
        } else {
            mins.clone()
        };

        let content = self.dims.content;
        let mut y = content.y;
        let mut first_row = true;
        let mut first_baseline = None;
        for child in &mut self.children {
            if !matches!(child.kind, BoxKind::Row) {
                let (cmt, cmb) = child.vertical_margins(content.w);
                child.layout(content.x, content.w, y + cmt, fonts, anchors);
                let bb = child.dims.border_box();
                y = bb.y + bb.h + cmb;
                continue;
            }
            if first_row {
                y += sp;
                first_row = false;
            }
            let mut x = content.x + sp;
            let mut col = 0;
            for cell in &mut child.children {
                if col >= n {
                    break;
                }
                let span = cell.span.min(n - col).max(1);
                let cw = widths[col..col + span].iter().sum::<f32>() + sp * (span as f32 - 1.0);
                cell.layout(x, cw, y, fonts, anchors);
                x += cw + sp;
                col += span;
            }
            let row_h = child
                .children
                .iter()
                .map(|c| c.dims.border_box().h)
                .fold(0.0f32, f32::max);
            for cell in &mut child.children {
                let h = cell.dims.border_box().h;
                let slack = row_h - h;
                if slack > 0.0 {
                    let offset = match cell.style.valign {
                        VAlign::Top => 0.0,
                        VAlign::Middle => slack / 2.0,
                        VAlign::Bottom => slack,
                    };
                    if offset > 0.0 {
                        for c in &mut cell.children {
                            c.translate(offset);
                        }
                        if let Some(b) = cell.first_baseline.as_mut() {
                            *b += offset;
                        }
                    }
                    cell.dims.content.h += slack;
                }
            }
            child.first_baseline = child.children.iter().find_map(|c| c.first_baseline);
            if first_baseline.is_none() {
                first_baseline = child.first_baseline;
            }
            child.dims = Dims {
                content: Rect {
                    x: content.x,
                    y,
                    w: content.w,
                    h: row_h,
                },
                ..Dims::default()
            };
            y += row_h + sp;
        }
        let mut h = (y - content.y).max(0.0);
        if let Len::Px(px) = s.height {
            h = h.max(px);
        }
        self.dims.content.h = h;
        self.first_baseline = first_baseline;
    }
}

/// Collapses two adjoining vertical margins per CSS 2.1 section 8.3.1.
fn collapse(a: f32, b: f32) -> f32 {
    if a >= 0.0 && b >= 0.0 {
        a.max(b)
    } else if a < 0.0 && b < 0.0 {
        a.min(b)
    } else {
        a + b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::parse_stylesheet;
    use crate::style::{Origin, style_tree};

    fn layout(html: &str, width: f32) -> (LayoutBox, Vec<(String, f32)>) {
        let doc = crate::html::parse(html);
        let sheets = vec![(parse_stylesheet(include_str!("ua.css")), Origin::UserAgent)];
        let styled = style_tree(&doc, &sheets, width, 600.0).unwrap();
        let fonts = FontStore::load().expect("tests need a TrueType font installed");
        let mut root = build_layout_tree(&styled, None);
        let mut anchors = Vec::new();
        let (mt, _) = root.vertical_margins(width);
        root.layout(0.0, width, mt, &fonts, &mut anchors);
        (root, anchors)
    }

    fn anchor(anchors: &[(String, f32)], name: &str) -> f32 {
        anchors.iter().find(|(n, _)| n == name).map(|(_, y)| *y).unwrap()
    }

    #[test]
    fn sibling_margins_collapse_to_the_larger_one() {
        let (_, a) = layout(
            r#"<body style="margin:0"><div id=a style="height:10px;margin-bottom:30px"></div><div id=b style="margin-top:20px"></div></body>"#,
            400.0,
        );
        assert_eq!(anchor(&a, "b") - anchor(&a, "a"), 40.0);
    }

    #[test]
    fn auto_margins_center_a_fixed_width_block() {
        let (root, _) = layout(
            r#"<body style="margin:0"><div style="width:100px;margin:0 auto;height:5px"></div></body>"#,
            500.0,
        );
        let body = &root.children[0];
        let div = &body.children[0];
        assert_eq!(div.dims.content.x, 200.0);
        assert_eq!(div.dims.content.w, 100.0);
    }

    #[test]
    fn long_text_wraps_inside_the_container() {
        let words = "word ".repeat(80);
        let (root, _) = layout(
            &format!(r#"<body style="margin:0"><p style="margin:0">{}</p></body>"#, words),
            200.0,
        );
        let p = &root.children[0].children[0];
        let BoxKind::Inline { frags, .. } = &p.children[0].kind else {
            panic!("expected inline content")
        };
        assert!(frags.iter().all(|f| f.rect.x + f.rect.w <= 200.5));
        assert!(p.dims.content.h > 100.0);
    }
}
