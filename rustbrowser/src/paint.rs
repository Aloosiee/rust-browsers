//! Display list generation and software rasterization.
//!
//! [`display_list`] flattens the laid-out box tree into drawing commands plus hit-test regions;
//! [`Canvas`] rasterizes those commands into a 0RGB framebuffer with alpha blending.

use crate::css::Color;
use crate::font::{FontId, FontStore};
use crate::layout::{BoxKind, LayoutBox, Marker, Rect};
use crate::style::ListStyle;
use std::collections::HashMap;
use std::rc::Rc;

/// A drawing command in page coordinates.
pub enum Cmd {
    Rect(Rect, Color),
    Text {
        x: f32,
        baseline: f32,
        text: String,
        font: FontId,
        size: f32,
        color: Color,
    },
    Circle {
        cx: f32,
        cy: f32,
        r: f32,
        color: Color,
        filled: bool,
    },
}

pub struct LinkArea {
    pub rect: Rect,
    pub href: Rc<str>,
}

/// Output of painting: commands, link regions, hit regions in paint order, and the bounding rectangle of each element.
#[derive(Default)]
pub struct Painted {
    pub cmds: Vec<Cmd>,
    pub links: Vec<LinkArea>,
    pub hits: Vec<(Rect, usize)>,
    pub rects: HashMap<usize, Rect>,
}

fn union(a: Rect, b: Rect) -> Rect {
    let x0 = a.x.min(b.x);
    let y0 = a.y.min(b.y);
    let x1 = (a.x + a.w).max(b.x + b.w);
    let y1 = (a.y + a.h).max(b.y + b.h);
    Rect {
        x: x0,
        y: y0,
        w: x1 - x0,
        h: y1 - y0,
    }
}

fn record(out: &mut Painted, id: usize, r: Rect) {
    if id == 0 {
        return;
    }
    out.hits.push((r, id));
    out.rects.entry(id).and_modify(|e| *e = union(*e, r)).or_insert(r);
}

/// Walks the layout tree in paint order and records commands and hit regions.
pub fn display_list(root: &LayoutBox, fonts: &FontStore) -> Painted {
    let mut out = Painted::default();
    paint_box(root, fonts, &mut out);
    out
}

fn paint_box(b: &LayoutBox, fonts: &FontStore, out: &mut Painted) {
    let s = &b.style;
    match &b.kind {
        BoxKind::Row => {
            if let (Some(bg), false) = (s.background, s.hidden) {
                out.cmds.push(Cmd::Rect(b.dims.border_box(), bg));
            }
            let r = b.dims.border_box();
            record(out, b.node, r);
            for child in &b.children {
                paint_box(child, fonts, out);
            }
        }
        BoxKind::Block { .. } | BoxKind::Table => {
            let marker = if let BoxKind::Block { marker, .. } = &b.kind {
                marker
            } else {
                &None
            };
            if !s.hidden {
                let bb = b.dims.border_box();
                if let Some(bg) = s.background {
                    out.cmds.push(Cmd::Rect(bb, bg));
                }
                let d = b.dims.border;
                let col = |i: usize| s.border_color[i].unwrap_or(s.color);
                if d[0] > 0.0 {
                    out.cmds.push(Cmd::Rect(
                        Rect {
                            x: bb.x,
                            y: bb.y,
                            w: bb.w,
                            h: d[0],
                        },
                        col(0),
                    ));
                }
                if d[2] > 0.0 {
                    out.cmds.push(Cmd::Rect(
                        Rect {
                            x: bb.x,
                            y: bb.y + bb.h - d[2],
                            w: bb.w,
                            h: d[2],
                        },
                        col(2),
                    ));
                }
                if d[3] > 0.0 {
                    out.cmds.push(Cmd::Rect(
                        Rect {
                            x: bb.x,
                            y: bb.y,
                            w: d[3],
                            h: bb.h,
                        },
                        col(3),
                    ));
                }
                if d[1] > 0.0 {
                    out.cmds.push(Cmd::Rect(
                        Rect {
                            x: bb.x + bb.w - d[1],
                            y: bb.y,
                            w: d[1],
                            h: bb.h,
                        },
                        col(1),
                    ));
                }
                if let Some(m) = marker {
                    let size = s.font_size;
                    let font = fonts.font_for(s.family, s.bold, s.italic);
                    let baseline = b
                        .first_baseline
                        .unwrap_or_else(|| b.dims.content.y + fonts.ascent_descent(font, size).0);
                    let c = b.dims.content;
                    match m {
                        Marker::Bullet(kind) => {
                            let r = (size * 0.17).max(2.0);
                            let cx = c.x - size * 0.75;
                            let cy = baseline - size * 0.32;
                            match kind {
                                ListStyle::Square => out.cmds.push(Cmd::Rect(
                                    Rect {
                                        x: cx - r,
                                        y: cy - r,
                                        w: r * 2.0,
                                        h: r * 2.0,
                                    },
                                    s.color,
                                )),
                                ListStyle::Circle => out.cmds.push(Cmd::Circle {
                                    cx,
                                    cy,
                                    r,
                                    color: s.color,
                                    filled: false,
                                }),
                                _ => out.cmds.push(Cmd::Circle {
                                    cx,
                                    cy,
                                    r,
                                    color: s.color,
                                    filled: true,
                                }),
                            }
                        }
                        Marker::Text(t) => {
                            let w = fonts.measure(t, font, size);
                            out.cmds.push(Cmd::Text {
                                x: c.x - w - size * 0.4,
                                baseline,
                                text: t.clone(),
                                font,
                                size,
                                color: s.color,
                            });
                        }
                    }
                }
            }
            record(out, b.node, b.dims.border_box());
            for child in &b.children {
                paint_box(child, fonts, out);
            }
        }
        BoxKind::Inline { frags, .. } => {
            for f in frags {
                let fs = &f.style;
                if fs.hidden {
                    continue;
                }
                if let Some(bg) = fs.background {
                    out.cmds.push(Cmd::Rect(f.rect, bg));
                }
                let size = fs.font_size;
                out.cmds.push(Cmd::Text {
                    x: f.rect.x,
                    baseline: f.baseline,
                    text: f.text.clone(),
                    font: f.font,
                    size,
                    color: fs.color,
                });
                let thick = (size / 15.0).max(1.0);
                if fs.underline {
                    out.cmds.push(Cmd::Rect(
                        Rect {
                            x: f.rect.x,
                            y: f.baseline + (size * 0.1).max(1.0),
                            w: f.rect.w,
                            h: thick,
                        },
                        fs.color,
                    ));
                }
                if fs.strike {
                    out.cmds.push(Cmd::Rect(
                        Rect {
                            x: f.rect.x,
                            y: f.baseline - size * 0.3,
                            w: f.rect.w,
                            h: thick,
                        },
                        fs.color,
                    ));
                }
                if let Some(href) = &f.link {
                    out.links.push(LinkArea {
                        rect: f.rect,
                        href: href.clone(),
                    });
                }
                record(out, f.elem, f.rect);
            }
        }
    }
}

/// A 0RGB framebuffer with a clip rectangle.
pub struct Canvas<'a> {
    pub buf: &'a mut [u32],
    pub width: usize,
    pub height: usize,
    clip: (i32, i32, i32, i32),
}

impl<'a> Canvas<'a> {
    pub fn new(buf: &'a mut [u32], width: usize, height: usize) -> Canvas<'a> {
        Canvas {
            buf,
            width,
            height,
            clip: (0, 0, width as i32, height as i32),
        }
    }

    pub fn set_clip(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let x0 = (x.floor() as i32).max(0);
        let y0 = (y.floor() as i32).max(0);
        let x1 = ((x + w).ceil() as i32).min(self.width as i32);
        let y1 = ((y + h).ceil() as i32).min(self.height as i32);
        self.clip = (x0, y0, x1, y1);
    }

    pub fn reset_clip(&mut self) {
        self.clip = (0, 0, self.width as i32, self.height as i32);
    }

    #[inline]
    fn blend(&mut self, x: i32, y: i32, c: Color, coverage: u32) {
        let (x0, y0, x1, y1) = self.clip;
        if x < x0 || y < y0 || x >= x1 || y >= y1 {
            return;
        }
        let a = coverage * c.a as u32 / 255;
        if a == 0 {
            return;
        }
        let idx = y as usize * self.width + x as usize;
        if a >= 255 {
            self.buf[idx] = (c.r as u32) << 16 | (c.g as u32) << 8 | c.b as u32;
            return;
        }
        let d = self.buf[idx];
        let mix = |s: u8, d: u32| (s as u32 * a + d * (255 - a)) / 255;
        let r = mix(c.r, (d >> 16) & 0xff);
        let g = mix(c.g, (d >> 8) & 0xff);
        let b = mix(c.b, d & 0xff);
        self.buf[idx] = r << 16 | g << 8 | b;
    }

    pub fn fill(&mut self, c: Color) {
        let v = (c.r as u32) << 16 | (c.g as u32) << 8 | c.b as u32;
        let (x0, y0, x1, y1) = self.clip;
        for y in y0..y1 {
            let row = y as usize * self.width;
            self.buf[row + x0 as usize..row + x1 as usize].fill(v);
        }
    }

    pub fn fill_rect(&mut self, r: Rect, c: Color) {
        let x0 = (r.x.round() as i32).max(self.clip.0);
        let y0 = (r.y.round() as i32).max(self.clip.1);
        let x1 = ((r.x + r.w).round() as i32).min(self.clip.2);
        let y1 = ((r.y + r.h).round() as i32).min(self.clip.3);
        if c.a == 255 {
            let v = (c.r as u32) << 16 | (c.g as u32) << 8 | c.b as u32;
            for y in y0..y1 {
                let row = y as usize * self.width;
                if x1 > x0 {
                    self.buf[row + x0 as usize..row + x1 as usize].fill(v);
                }
            }
        } else {
            for y in y0..y1 {
                for x in x0..x1 {
                    self.blend(x, y, c, 255);
                }
            }
        }
    }

    pub fn fill_circle(&mut self, cx: f32, cy: f32, r: f32, c: Color, filled: bool) {
        let x0 = (cx - r - 1.0).floor() as i32;
        let x1 = (cx + r + 1.0).ceil() as i32;
        let y0 = (cy - r - 1.0).floor() as i32;
        let y1 = (cy + r + 1.0).ceil() as i32;
        let thickness = (r * 0.3).max(1.0);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let dx = x as f32 + 0.5 - cx;
                let dy = y as f32 + 0.5 - cy;
                let d = (dx * dx + dy * dy).sqrt();
                let outer = (r + 0.5 - d).clamp(0.0, 1.0);
                let cov = if filled {
                    outer
                } else {
                    outer * (d - (r - thickness) + 0.5).clamp(0.0, 1.0)
                };
                if cov > 0.0 {
                    self.blend(x, y, c, (cov * 255.0) as u32);
                }
            }
        }
    }

    pub fn fill_triangle(&mut self, p: [(f32, f32); 3], c: Color) {
        let minx = p.iter().map(|q| q.0).fold(f32::MAX, f32::min).floor() as i32;
        let maxx = p.iter().map(|q| q.0).fold(f32::MIN, f32::max).ceil() as i32;
        let miny = p.iter().map(|q| q.1).fold(f32::MAX, f32::min).floor() as i32;
        let maxy = p.iter().map(|q| q.1).fold(f32::MIN, f32::max).ceil() as i32;
        let edge = |a: (f32, f32), b: (f32, f32), x: f32, y: f32| (b.0 - a.0) * (y - a.1) - (b.1 - a.1) * (x - a.0);
        let area = edge(p[0], p[1], p[2].0, p[2].1);
        if area.abs() < 0.01 {
            return;
        }
        for y in miny..=maxy {
            for x in minx..=maxx {
                let mut inside = 0;
                for (sx, sy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                    let px = x as f32 + sx;
                    let py = y as f32 + sy;
                    let w0 = edge(p[1], p[2], px, py) / area;
                    let w1 = edge(p[2], p[0], px, py) / area;
                    let w2 = edge(p[0], p[1], px, py) / area;
                    if w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0 {
                        inside += 1;
                    }
                }
                if inside > 0 {
                    self.blend(x, y, c, inside * 255 / 4);
                }
            }
        }
    }

    /// Draws a single line of text with its baseline at `baseline`.
    pub fn draw_text(
        &mut self,
        x: f32,
        baseline: f32,
        text: &str,
        font: FontId,
        size: f32,
        c: Color,
        fonts: &FontStore,
    ) {
        let mut pen = x;
        let by = baseline.round();
        for ch in text.chars() {
            let adv = fonts.advance(font, ch, size);
            if ch != ' ' && ch != '\u{a0}' {
                let g = fonts.glyph(font, ch, size);
                let (m, bitmap) = (&g.0, &g.1);
                let gx = (pen + m.xmin as f32).round() as i32;
                let gy = by as i32 - m.height as i32 - m.ymin;
                if gx + m.width as i32 >= self.clip.0
                    && gx < self.clip.2
                    && gy + m.height as i32 >= self.clip.1
                    && gy < self.clip.3
                {
                    for row in 0..m.height {
                        for col in 0..m.width {
                            let cov = bitmap[row * m.width + col];
                            if cov > 0 {
                                self.blend(gx + col as i32, gy + row as i32, c, cov as u32);
                            }
                        }
                    }
                }
            }
            pen += adv;
            if pen > self.clip.2 as f32 + 4.0 {
                break;
            }
        }
    }
}

/// Rasterizes commands, mapping page coordinates to the screen as `page * scale + offset`. Commands outside the clip are skipped.
pub fn render(cmds: &[Cmd], canvas: &mut Canvas, dx: f32, dy: f32, scale: f32, fonts: &FontStore) {
    let top = canvas.clip.1 as f32;
    let bottom = canvas.clip.3 as f32;
    for cmd in cmds {
        match cmd {
            Cmd::Rect(r, c) => {
                let r = Rect {
                    x: r.x * scale + dx,
                    y: r.y * scale + dy,
                    w: r.w * scale,
                    h: r.h * scale,
                };
                if r.y + r.h >= top && r.y <= bottom {
                    canvas.fill_rect(r, *c);
                }
            }
            Cmd::Text {
                x,
                baseline,
                text,
                font,
                size,
                color,
            } => {
                let b = baseline * scale + dy;
                let sz = size * scale;
                if b + sz >= top && b - sz * 1.3 <= bottom {
                    canvas.draw_text(x * scale + dx, b, text, *font, sz, *color, fonts);
                }
            }
            Cmd::Circle {
                cx,
                cy,
                r,
                color,
                filled,
            } => {
                let y = cy * scale + dy;
                let r = r * scale;
                if y + r >= top && y - r <= bottom {
                    canvas.fill_circle(cx * scale + dx, y, r, *color, *filled);
                }
            }
        }
    }
}
